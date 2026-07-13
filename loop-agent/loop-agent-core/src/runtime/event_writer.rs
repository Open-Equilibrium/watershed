const EVENT_WRITER_QUEUE_CAPACITY: usize = 64;
const EVENT_WRITER_DIRTY_SYNC_INTERVAL: Duration = Duration::from_secs(1);

trait RuntimeEventSink {
    fn commit(
        &mut self,
        event: &EventEnvelope,
        canonical_jsonl: &str,
    ) -> Result<(), RuntimeError>;
}

struct WriterOutcome {
    appended: bool,
    error: Option<RuntimeError>,
}

enum SessionWriterCommand {
    Commit {
        acknowledgement: std::sync::mpsc::SyncSender<WriterOutcome>,
        canonical_jsonl: String,
        event: EventEnvelope,
    },
    Shutdown {
        acknowledgement: std::sync::mpsc::SyncSender<WriterOutcome>,
    },
}

struct SerialSessionWriter<'a> {
    commit_reservation: Option<&'a SessionReservation>,
    emit: EmitMode,
    failed: bool,
    observer: Option<&'a mut dyn Write>,
    sender: Option<std::sync::mpsc::SyncSender<SessionWriterCommand>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl<'a> SerialSessionWriter<'a> {
    fn start(
        reservation: &'a SessionReservation,
        emit: EmitMode,
        observer: &'a mut dyn Write,
    ) -> Result<Self, RuntimeError> {
        Self::start_with_validation(
            &reservation.session_path,
            &reservation.session_id,
            SessionAppendValidationState::empty(&reservation.session_id),
            Some(reservation),
            emit,
            observer,
        )
    }

    fn start_existing(
        path: &Path,
        session_id: &str,
        prior_events: &[EventEnvelope],
        prior_stream_bytes: usize,
        emit: EmitMode,
        observer: &'a mut dyn Write,
    ) -> Result<Self, RuntimeError> {
        let validation = SessionAppendValidationState::from_prior_events(
            path,
            session_id,
            prior_events,
            prior_stream_bytes,
        )?;
        Self::start_with_validation(
            path,
            session_id,
            validation,
            None,
            emit,
            observer,
        )
    }

    fn start_with_validation(
        path: &Path,
        session_id: &str,
        validation: SessionAppendValidationState,
        commit_reservation: Option<&'a SessionReservation>,
        emit: EmitMode,
        observer: &'a mut dyn Write,
    ) -> Result<Self, RuntimeError> {
        let (sender, receiver) =
            std::sync::mpsc::sync_channel(EVENT_WRITER_QUEUE_CAPACITY);
        let path = path.to_owned();
        let session_id = session_id.to_owned();
        let worker = thread::Builder::new()
            .name(format!("loop-event-writer-{session_id}"))
            .spawn(move || session_writer_worker(&path, validation, &receiver))
            .map_err(|source| RuntimeError::Io {
                path: PathBuf::from("<event-writer-thread>"),
                source,
            })?;
        Ok(Self {
            commit_reservation,
            emit,
            failed: false,
            observer: Some(observer),
            sender: Some(sender),
            worker: Some(worker),
        })
    }

    fn finish(&mut self) -> Result<(), RuntimeError> {
        let Some(sender) = self.sender.take() else {
            return Ok(());
        };
        let (acknowledgement, response) = std::sync::mpsc::sync_channel(1);
        let send_result = sender.send(SessionWriterCommand::Shutdown { acknowledgement });
        drop(sender);
        let outcome = send_result
            .map_err(|_| writer_channel_closed_error())
            .and_then(|()| response.recv().map_err(|_| writer_channel_closed_error()));
        let join_result = self
            .worker
            .take()
            .expect("started event writer owns a worker")
            .join()
            .map_err(|_| RuntimeError::Protocol("session event writer panicked".to_owned()));
        let outcome = outcome?;
        join_result?;
        if let Some(err) = outcome.error {
            self.failed = true;
            return Err(err);
        }
        Ok(())
    }

    fn publish_human_status(&mut self, status: &str) {
        if self.emit == EmitMode::Human {
            self.publish(status.as_bytes());
        }
    }

    fn publish(&mut self, bytes: &[u8]) {
        let Some(observer) = self.observer.as_deref_mut() else {
            return;
        };
        if observer
            .write_all(bytes)
            .and_then(|()| observer.flush())
            .is_err()
        {
            // WHY: the append-only log is authoritative. A disconnected or failed observer
            // detaches and can catch up by sequence without rolling back committed events.
            self.observer = None;
        }
    }
}

impl RuntimeEventSink for SerialSessionWriter<'_> {
    fn commit(
        &mut self,
        event: &EventEnvelope,
        canonical_jsonl: &str,
    ) -> Result<(), RuntimeError> {
        if self.failed {
            return Err(event_writer_failure(RuntimeError::Protocol(
                "session event writer is closed after a prior failure".to_owned(),
            )));
        }
        let sender = self.sender.as_ref().ok_or_else(|| {
            RuntimeError::Protocol("session event writer is already closed".to_owned())
        })?;
        let (acknowledgement, response) = std::sync::mpsc::sync_channel(1);
        sender
            .send(SessionWriterCommand::Commit {
                acknowledgement,
                canonical_jsonl: canonical_jsonl.to_owned(),
                event: event.clone(),
            })
            .map_err(|_| event_writer_failure(writer_channel_closed_error()))?;
        let outcome = response
            .recv()
            .map_err(|_| event_writer_failure(writer_channel_closed_error()))?;
        if outcome.appended {
            if let Some(reservation) = self.commit_reservation {
                reservation.mark_committed();
            }
        }
        if let Some(err) = outcome.error {
            self.failed = true;
            return Err(event_writer_failure(err));
        }
        if self.emit == EmitMode::Jsonl {
            self.publish(canonical_jsonl.as_bytes());
        }
        Ok(())
    }
}

struct ResumeEventSink<'writer, 'observer> {
    clock: EventClock,
    marker_committed: bool,
    marker_event: EventEnvelope,
    marker_stream: String,
    planned_event_count: usize,
    resume_marker_count: usize,
    writer: &'writer mut SerialSessionWriter<'observer>,
}

impl RuntimeEventSink for ResumeEventSink<'_, '_> {
    fn commit(
        &mut self,
        event: &EventEnvelope,
        _canonical_jsonl: &str,
    ) -> Result<(), RuntimeError> {
        if event.sequence <= self.planned_event_count as u64 {
            return Ok(());
        }
        if !self.marker_committed {
            self.writer
                .commit(&self.marker_event, &self.marker_stream)?;
            self.marker_committed = true;
        }
        let shifted = shift_resumed_event(
            event.clone(),
            self.resume_marker_count as u64 + 1,
            self.clock,
        );
        let canonical = shifted.canonical_jsonl().map_err(|err| {
            RuntimeError::Protocol(format!(
                "failed to serialize resumed runtime event: {err}"
            ))
        })?;
        self.writer.commit(&shifted, &canonical)
    }
}

impl Drop for SerialSessionWriter<'_> {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

fn session_writer_worker(
    path: &Path,
    mut validation: SessionAppendValidationState,
    receiver: &std::sync::mpsc::Receiver<SessionWriterCommand>,
) {
    let mut dirty = false;
    let mut stopped = false;
    let mut pending_error = None;
    loop {
        match receiver.recv_timeout(EVENT_WRITER_DIRTY_SYNC_INTERVAL) {
            Ok(SessionWriterCommand::Commit {
                acknowledgement,
                canonical_jsonl,
                event,
            }) => {
                let outcome = if let Some(err) = pending_error.take() {
                    stopped = true;
                    WriterOutcome {
                        appended: false,
                        error: Some(err),
                    }
                } else if stopped {
                    WriterOutcome {
                        appended: false,
                        error: Some(RuntimeError::Protocol(
                            "session event writer is closed after a prior failure".to_owned(),
                        )),
                    }
                } else {
                    commit_session_event(
                        path,
                        &mut validation,
                        &event,
                        &canonical_jsonl,
                        &mut dirty,
                    )
                };
                if outcome.error.is_some() {
                    stopped = true;
                }
                let _ = acknowledgement.send(outcome);
            }
            Ok(SessionWriterCommand::Shutdown { acknowledgement }) => {
                let error = pending_error.take().or_else(|| {
                    if dirty && !stopped {
                        sync_session_log(path).err()
                    } else {
                        None
                    }
                });
                let _ = acknowledgement.send(WriterOutcome {
                    appended: false,
                    error,
                });
                break;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) if dirty && !stopped => {
                if let Err(err) = sync_session_log(path) {
                    pending_error = Some(err);
                } else {
                    dirty = false;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                if dirty && !stopped {
                    let _ = sync_session_log(path);
                }
                break;
            }
        }
    }
}

fn commit_session_event(
    path: &Path,
    validation: &mut SessionAppendValidationState,
    event: &EventEnvelope,
    canonical_jsonl: &str,
    dirty: &mut bool,
) -> WriterOutcome {
    if let Err(err) = validation.validate_constructed_event(
        path,
        event,
        canonical_jsonl.len(),
    ) {
        return WriterOutcome {
            appended: false,
            error: Some(err),
        };
    }
    if let Err(err) = append_session_log_text(path, canonical_jsonl) {
        return WriterOutcome {
            appended: false,
            error: Some(err),
        };
    }
    *dirty = true;
    if is_event_sync_checkpoint(&event.event_type) {
        if let Err(err) = sync_session_log(path) {
            return WriterOutcome {
                appended: true,
                error: Some(err),
            };
        }
        *dirty = false;
    }
    WriterOutcome {
        appended: true,
        error: None,
    }
}

fn is_event_sync_checkpoint(event_type: &EventType) -> bool {
    matches!(
        event_type,
        EventType::MessageCompleted
            | EventType::ToolCompleted
            | EventType::ToolFailed
            | EventType::ToolTimedOut
            | EventType::SessionPaused
            | EventType::SessionCompleted
            | EventType::SessionFailed
    )
}

fn sync_session_log(path: &Path) -> Result<(), RuntimeError> {
    ensure_non_hardlinked_real_file(path)?;
    let expected_metadata = fs::symlink_metadata(path).map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })?;
    validate_real_file(path, &expected_metadata)?;
    let file = open_file_for_sync_without_following_reparse(path).map_err(|source| {
        RuntimeError::Io {
            path: path.to_owned(),
            source,
        }
    })?;
    ensure_opened_real_file_for_read_matches_path(path, &expected_metadata, &file)?;
    file.sync_all().map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })
}

#[cfg(windows)]
fn open_file_for_sync_without_following_reparse(path: &Path) -> io::Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(windows))]
fn open_file_for_sync_without_following_reparse(path: &Path) -> io::Result<fs::File> {
    fs::OpenOptions::new().read(true).write(true).open(path)
}

fn writer_channel_closed_error() -> RuntimeError {
    RuntimeError::Protocol("session event writer channel closed unexpectedly".to_owned())
}

fn event_writer_failure(source: RuntimeError) -> RuntimeError {
    RuntimeError::EventWriter(Box::new(source))
}
