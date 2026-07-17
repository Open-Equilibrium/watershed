const LIVE_EVENT_NOTIFICATION_CAPACITY: usize = 1;

/// Result of a non-blocking committed-event notification attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveEventNotifyStatus {
    /// A wake-up was queued for the receiver.
    Queued,
    /// A wake-up was already pending; the shared high-watermark was still advanced.
    Coalesced,
    /// The receiver was dropped.
    Closed,
}

/// A best-effort wake-up for events already committed to one session log.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveEventNotification {
    /// Session whose committed log should be read.
    pub session_id: String,
    /// Highest committed sequence observed when this wake-up was received.
    pub highest_committed_sequence: u64,
}

/// Error returned while waiting for a live-event wake-up.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveEventReceiveError {
    /// No wake-up arrived before the caller's deadline.
    Timeout,
    /// Every notifier was dropped and no wake-up remains queued.
    Closed,
}

impl fmt::Display for LiveEventReceiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Timeout => "live-event notification timed out",
            Self::Closed => "live-event notification channel is closed",
        })
    }
}

impl std::error::Error for LiveEventReceiveError {}

struct LiveEventState {
    highest_committed_sequence: std::sync::atomic::AtomicU64,
}

/// Producer side of one bounded, caller-owned live-event notification channel.
///
/// This handle owns no task or thread. Pass it to one run or resume operation. Each
/// successful append advances a shared high-watermark and attempts a capacity-one wake-up.
pub struct LiveEventNotifier {
    sender: std::sync::mpsc::SyncSender<String>,
    state: std::sync::Arc<LiveEventState>,
}

impl LiveEventNotifier {
    /// Advances the committed high-watermark and attempts a wake-up without waiting.
    ///
    /// Call this only after `committed_sequence` is readable from the authoritative session
    /// log. A full or closed channel never blocks and never changes persistence semantics.
    pub fn try_notify(&self, session_id: &str, committed_sequence: u64) -> LiveEventNotifyStatus {
        self.state
            .highest_committed_sequence
            .fetch_max(committed_sequence, std::sync::atomic::Ordering::Release);
        match self.sender.try_send(session_id.to_owned()) {
            Ok(()) => LiveEventNotifyStatus::Queued,
            Err(std::sync::mpsc::TrySendError::Full(_)) => LiveEventNotifyStatus::Coalesced,
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => LiveEventNotifyStatus::Closed,
        }
    }
}

/// Receiver side of one bounded, caller-owned live-event notification channel.
///
/// On wake-up, read committed events after the caller's last processed sequence with
/// [`SessionEventReader::read_incremental_after`]. Advance that cursor only after processing
/// each event, then drain another wake-up before waiting again. After the producer closes, use
/// [`SessionEventReader::read_after`] once to verify the complete authoritative log. This closes
/// the replay/live race because a commit either advances the observed high-watermark or leaves
/// another wake-up queued.
pub struct LiveEventReceiver {
    receiver: std::sync::mpsc::Receiver<String>,
    state: std::sync::Arc<LiveEventState>,
}

impl LiveEventReceiver {
    /// Waits up to `timeout` for a coalesced committed-event wake-up.
    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<LiveEventNotification, LiveEventReceiveError> {
        let session_id = self
            .receiver
            .recv_timeout(timeout)
            .map_err(|err| match err {
                std::sync::mpsc::RecvTimeoutError::Timeout => LiveEventReceiveError::Timeout,
                std::sync::mpsc::RecvTimeoutError::Disconnected => LiveEventReceiveError::Closed,
            })?;
        Ok(LiveEventNotification {
            session_id,
            highest_committed_sequence: self
                .state
                .highest_committed_sequence
                .load(std::sync::atomic::Ordering::Acquire),
        })
    }
}

/// Creates a capacity-one live-event notification channel with no runtime-owned worker.
pub fn live_event_channel() -> (LiveEventNotifier, LiveEventReceiver) {
    let (sender, receiver) = std::sync::mpsc::sync_channel(LIVE_EVENT_NOTIFICATION_CAPACITY);
    let state = std::sync::Arc::new(LiveEventState {
        highest_committed_sequence: std::sync::atomic::AtomicU64::new(0),
    });
    (
        LiveEventNotifier {
            sender,
            state: std::sync::Arc::clone(&state),
        },
        LiveEventReceiver { receiver, state },
    )
}

/// Validated, append-only reader for one authoritative session log.
///
/// Reads are capped by [`MAX_SESSION_LOG_BYTES`]. The reader tolerates an incomplete final
/// JSONL line while an append is in progress, rejects mutation of an already observed event,
/// and leaves cursor advancement to the caller.
pub struct SessionEventReader {
    observed: Vec<EventEnvelope>,
    observed_signature: RuntimeStreamSignatureBuilder,
    path: PathBuf,
    validation: SessionAppendValidationState,
}

impl SessionEventReader {
    /// Opens a session's validated log boundary without reading event payloads yet.
    pub fn open(workspace: impl AsRef<Path>, session_id: &str) -> Result<Self, RuntimeError> {
        let workspace = workspace.as_ref();
        let path = session_path(workspace, session_id)?;
        ensure_existing_session_log_path(workspace, &path)?;
        Ok(Self {
            observed: Vec::new(),
            observed_signature: RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN),
            path,
            validation: SessionAppendValidationState::empty(session_id),
        })
    }

    /// Reads every complete committed event whose sequence is greater than `cursor`.
    ///
    /// The caller must advance `cursor` only after successfully processing each returned
    /// event. Repeating this call is safe after a processing failure.
    pub fn read_after(&mut self, cursor: u64) -> Result<Vec<EventEnvelope>, RuntimeError> {
        let bytes = read_file_with_limit(&self.path, MAX_SESSION_LOG_BYTES)?;
        let complete_len = complete_jsonl_prefix_len(&bytes);
        let has_partial_line = complete_len != bytes.len();
        let complete = &bytes[..complete_len];
        let observed_records = self.validation.line_count;
        let prefix_len = jsonl_record_prefix_len(complete, observed_records);
        if prefix_len.is_none_or(|prefix_len| {
            stream_signature(&complete[..prefix_len]).signature()
                != self.observed_signature.signature()
        }) {
            return Err(RuntimeError::Protocol(format!(
                "{} changed outside append-only session semantics",
                self.path.display()
            )));
        }
        let complete_text = std::str::from_utf8(complete).map_err(|source| {
            RuntimeError::Protocol(format!(
                "{} is not valid UTF-8: {source}",
                self.path.display()
            ))
        })?;
        let session_id = self
            .validation
            .expected_session_id
            .as_deref()
            .expect("session readers always validate one session");
        let mut validation = SessionAppendValidationState::empty(session_id);
        let events = validation.validate_appended(&self.path, complete_text)?;
        if has_partial_line && validation.terminal_line.is_some() {
            return Err(RuntimeError::Protocol(format!(
                "{} contains a partial line after a terminal event",
                self.path.display()
            )));
        }
        self.ensure_cursor(cursor, validation.previous_sequence)?;
        self.observed = events;
        self.observed_signature = stream_signature(complete);
        self.validation = validation;
        Ok(events_after(&self.observed, cursor))
    }

    /// Reads only the newly appended complete suffix after an initial verified read.
    ///
    /// This is the efficient path for a receiver attached to the same live operation. Call
    /// [`Self::read_after`] once after the producer closes to verify the complete authoritative
    /// log before treating delivery as final.
    pub fn read_incremental_after(
        &mut self,
        cursor: u64,
    ) -> Result<Vec<EventEnvelope>, RuntimeError> {
        if self.validation.line_count == 0 {
            return self.read_after(cursor);
        }
        let observed_len = self.observed_signature.byte_count;
        let observed_len_u64 = u64::try_from(observed_len).unwrap_or(u64::MAX);
        let (mut file, metadata) = open_real_file_for_read(&self.path)?;
        if metadata.len() < observed_len_u64 {
            return Err(self.changed_outside_append_only());
        }
        file.seek(SeekFrom::Start(observed_len_u64))
            .map_err(|source| path_io_error(&self.path, source))?;
        let remaining_limit = MAX_SESSION_LOG_BYTES.saturating_sub(observed_len_u64);
        let mut suffix = Vec::new();
        file.take(remaining_limit.saturating_add(1))
            .read_to_end(&mut suffix)
            .map_err(|source| path_io_error(&self.path, source))?;
        if u64::try_from(suffix.len()).unwrap_or(u64::MAX) > remaining_limit {
            return Err(RuntimeError::Protocol(format!(
                "{} read size exceeds max {MAX_SESSION_LOG_BYTES}",
                self.path.display()
            )));
        }
        let complete_len = complete_jsonl_prefix_len(&suffix);
        let has_partial_line = complete_len != suffix.len();
        let appended_bytes = &suffix[..complete_len];
        let appended_text = std::str::from_utf8(appended_bytes).map_err(|source| {
            RuntimeError::Protocol(format!(
                "{} is not valid UTF-8: {source}",
                self.path.display()
            ))
        })?;
        let mut validation = std::mem::replace(
            &mut self.validation,
            SessionAppendValidationState::unscoped(),
        );
        let appended = match validation.validate_appended(&self.path, appended_text) {
            Ok(appended) => appended,
            Err(error) => {
                self.restore_validation(&validation);
                return Err(error);
            }
        };
        if has_partial_line && validation.terminal_line.is_some() {
            self.restore_validation(&validation);
            return Err(RuntimeError::Protocol(format!(
                "{} contains a partial line after a terminal event",
                self.path.display()
            )));
        }
        if let Err(error) = self.ensure_cursor(cursor, validation.previous_sequence) {
            self.restore_validation(&validation);
            return Err(error);
        }
        for record in appended_bytes.split_inclusive(|byte| *byte == b'\n') {
            self.observed_signature.push(record);
        }
        self.observed.extend(appended);
        self.validation = validation;
        Ok(events_after(&self.observed, cursor))
    }

    fn restore_validation(&mut self, candidate: &SessionAppendValidationState) {
        let session_id = candidate
            .expected_session_id
            .as_deref()
            .expect("session readers always validate one session");
        self.validation =
            SessionAppendValidationState::from_prior_events(&self.path, session_id, &self.observed)
                .expect("cached observed events remain valid");
    }

    fn ensure_cursor(&self, cursor: u64, latest_sequence: u64) -> Result<(), RuntimeError> {
        if cursor <= latest_sequence {
            return Ok(());
        }
        Err(RuntimeError::Protocol(format!(
            "{} no longer contains processed sequence {cursor}",
            self.path.display()
        )))
    }

    fn changed_outside_append_only(&self) -> RuntimeError {
        RuntimeError::Protocol(format!(
            "{} changed outside append-only session semantics",
            self.path.display()
        ))
    }
}

fn stream_signature(bytes: &[u8]) -> RuntimeStreamSignatureBuilder {
    let mut signature = RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN);
    for record in bytes.split_inclusive(|byte| *byte == b'\n') {
        signature.push(record);
    }
    signature
}

fn jsonl_record_prefix_len(bytes: &[u8], record_count: usize) -> Option<usize> {
    if record_count == 0 {
        return Some(0);
    }
    bytes
        .iter()
        .enumerate()
        .filter(|(_, byte)| **byte == b'\n')
        .nth(record_count - 1)
        .map(|(index, _)| index + 1)
}

fn events_after(events: &[EventEnvelope], cursor: u64) -> Vec<EventEnvelope> {
    events[events.partition_point(|event| event.sequence <= cursor)..].to_vec()
}

fn complete_jsonl_prefix_len(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |newline_index| newline_index + 1)
}
