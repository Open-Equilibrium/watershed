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
    /// Earliest committed sequence represented by this pending wake-up.
    pub first_committed_sequence: u64,
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
    sender: std::sync::mpsc::SyncSender<(String, u64)>,
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
        match self
            .sender
            .try_send((session_id.to_owned(), committed_sequence))
        {
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
    receiver: std::sync::mpsc::Receiver<(String, u64)>,
    state: std::sync::Arc<LiveEventState>,
}

impl LiveEventReceiver {
    /// Returns the highest committed sequence currently published by this operation.
    ///
    /// Join the producer before using this value as the final replay boundary.
    pub fn highest_committed_sequence(&self) -> u64 {
        self.state
            .highest_committed_sequence
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// Waits up to `timeout` for a coalesced committed-event wake-up.
    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<LiveEventNotification, LiveEventReceiveError> {
        let (session_id, first_committed_sequence) =
            self.receiver
                .recv_timeout(timeout)
                .map_err(|err| match err {
                    std::sync::mpsc::RecvTimeoutError::Timeout => LiveEventReceiveError::Timeout,
                    std::sync::mpsc::RecvTimeoutError::Disconnected => {
                        LiveEventReceiveError::Closed
                    }
                })?;
        Ok(LiveEventNotification {
            session_id,
            first_committed_sequence,
            highest_committed_sequence: self.highest_committed_sequence(),
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
/// Each read is bounded by the per-segment and per-session event-data limits. The reader
/// tolerates an incomplete final JSONL line while the session lock is present, rejects
/// mutation of an already observed event, and leaves cursor advancement to the caller.
pub struct SessionEventReader {
    observed_current_segment_bytes: u64,
    observed_segment_count: usize,
    observed_signature: RuntimeStreamSignatureBuilder,
    lock_path: AnchoredFile,
    path: AnchoredFile,
    validation: SessionAppendValidationState,
}

impl SessionEventReader {
    /// Opens a session's validated log boundary without reading event payloads yet.
    pub fn open(workspace: impl AsRef<Path>, session_id: &str) -> Result<Self, RuntimeError> {
        let workspace = workspace.as_ref();
        if !proto::is_valid_session_id(session_id) {
            return Err(RuntimeError::Usage(format!(
                "invalid session_id {session_id:?}"
            )));
        }
        let sessions =
            open_runtime_dir(workspace, "sessions")?.ok_or_else(|| RuntimeError::Io {
                path: workspace.join(LOCAL_SESSION_DIR),
                source: io::Error::from(io::ErrorKind::NotFound),
            })?;
        let path = sessions.file(format!("{session_id}.jsonl"));
        ensure_anchored_real_file(&path)?;
        Ok(Self {
            observed_current_segment_bytes: 0,
            observed_segment_count: 0,
            observed_signature: RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN),
            lock_path: sessions.file(format!("{session_id}.lock")),
            path,
            validation: SessionAppendValidationState::empty(session_id),
        })
    }

    /// Reads every complete committed event whose sequence is greater than `cursor`.
    ///
    /// The caller must advance `cursor` only after successfully processing each returned
    /// event. Repeating this call is safe after a processing failure.
    pub fn read_after(&mut self, cursor: u64) -> Result<Vec<EventEnvelope>, RuntimeError> {
        let mut retried_inactive_partial = false;
        loop {
            let segments = segmented_jsonl_files(&self.path, EVENT_STREAM_LIMITS)?;
            let mut bytes = Vec::new();
            let mut final_complete_bytes = 0u64;
            for (index, segment) in segments.iter().enumerate() {
                let segment_bytes =
                    read_anchored_file_with_limit(segment, MAX_SESSION_SEGMENT_BYTES)?;
                if index + 1 != segments.len()
                    && complete_jsonl_prefix_len(&segment_bytes) != segment_bytes.len()
                {
                    return Err(RuntimeError::Protocol(format!(
                        "{} non-final segment must end with LF",
                        segment.diagnostic_path().display()
                    )));
                }
                if index + 1 == segments.len() {
                    final_complete_bytes = u64::try_from(complete_jsonl_prefix_len(&segment_bytes))
                        .unwrap_or(u64::MAX);
                }
                if u64::try_from(bytes.len().saturating_add(segment_bytes.len()))
                    .unwrap_or(u64::MAX)
                    > MAX_SESSION_EVENT_BYTES
                {
                    return Err(RuntimeError::Protocol(format!(
                        "{} session event data exceeds max {MAX_SESSION_EVENT_BYTES}",
                        self.path.diagnostic_path().display()
                    )));
                }
                bytes.extend_from_slice(&segment_bytes);
            }
            let complete_len = complete_jsonl_prefix_len(&bytes);
            let has_partial_line = complete_len != bytes.len();
            let inactive_partial = has_partial_line && !self.session_lock_present()?;
            if inactive_partial && !retried_inactive_partial {
                retried_inactive_partial = true;
                continue;
            }
            let complete = &bytes[..complete_len];
            let prefix_len =
                jsonl_record_prefix_len(complete, self.observed_signature.record_count);
            if prefix_len.is_none_or(|prefix_len| {
                stream_signature(&complete[..prefix_len]).signature()
                    != self.observed_signature.signature()
            }) {
                return Err(self.changed_outside_append_only());
            }
            let complete_text = std::str::from_utf8(complete).map_err(|source| {
                RuntimeError::Protocol(format!(
                    "{} is not valid UTF-8: {source}",
                    self.path.diagnostic_path().display()
                ))
            })?;
            let session_id = self
                .validation
                .expected_session_id
                .as_deref()
                .expect("session readers always validate one session");
            let mut validation = SessionAppendValidationState::empty(session_id);
            let events =
                validation.validate_appended(self.path.diagnostic_path(), complete_text)?;
            if has_partial_line && validation.terminal_line.is_some() {
                return Err(RuntimeError::Protocol(format!(
                    "{} contains a partial line after a terminal event",
                    self.path.diagnostic_path().display()
                )));
            }
            if inactive_partial {
                return Err(self.inactive_partial());
            }
            self.ensure_cursor(cursor, validation.previous_sequence)?;
            self.observed_current_segment_bytes = final_complete_bytes;
            self.observed_segment_count = segments.len();
            self.observed_signature = stream_signature(complete);
            self.validation = validation;
            return Ok(events_after(events, cursor));
        }
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
        self.read_incremental_after_with(cursor, &mut || {})
    }

    fn read_incremental_after_with(
        &mut self,
        cursor: u64,
        after_read: &mut impl FnMut(),
    ) -> Result<Vec<EventEnvelope>, RuntimeError> {
        if self.validation.line_count == 0 || cursor < self.validation.previous_sequence {
            return self.read_after(cursor);
        }
        let observed_event_bytes =
            u64::try_from(self.observed_signature.byte_count).unwrap_or(u64::MAX);
        let mut retried_inactive_partial = false;
        loop {
            let segments = self.incremental_segments()?;
            if segments.len() < self.observed_segment_count || self.observed_segment_count == 0 {
                return Err(self.changed_outside_append_only());
            }
            let mut suffix = Vec::new();
            let mut final_complete_bytes = 0u64;
            let prior_final_index = self.observed_segment_count - 1;
            for (index, segment) in segments.iter().enumerate().skip(prior_final_index) {
                let (mut file, metadata) = open_anchored_file_for_read(segment)?;
                let offset = if index == prior_final_index {
                    self.observed_current_segment_bytes
                } else {
                    0
                };
                if metadata.len() < offset {
                    return Err(self.changed_outside_append_only());
                }
                file.seek(SeekFrom::Start(offset))
                    .map_err(|source| path_io_error(segment.diagnostic_path(), source))?;
                let remaining_limit = MAX_SESSION_SEGMENT_BYTES.saturating_sub(offset);
                let start = suffix.len();
                let remaining_event_bytes = MAX_SESSION_EVENT_BYTES
                    .saturating_sub(observed_event_bytes)
                    .saturating_sub(u64::try_from(start).unwrap_or(u64::MAX));
                file.take(remaining_limit.min(remaining_event_bytes).saturating_add(1))
                    .read_to_end(&mut suffix)
                    .map_err(|source| path_io_error(segment.diagnostic_path(), source))?;
                let segment_suffix = &suffix[start..];
                let segment_complete_len = complete_jsonl_prefix_len(segment_suffix);
                if u64::try_from(segment_suffix.len()).unwrap_or(u64::MAX) > remaining_limit {
                    return Err(RuntimeError::Protocol(format!(
                        "{} read size exceeds max {MAX_SESSION_SEGMENT_BYTES}",
                        segment.diagnostic_path().display()
                    )));
                }
                if observed_event_bytes
                    .saturating_add(u64::try_from(suffix.len()).unwrap_or(u64::MAX))
                    > MAX_SESSION_EVENT_BYTES
                {
                    return Err(RuntimeError::Protocol(format!(
                        "{} session event data exceeds max {MAX_SESSION_EVENT_BYTES}",
                        self.path.diagnostic_path().display()
                    )));
                }
                if index + 1 != segments.len() && segment_complete_len != segment_suffix.len() {
                    return Err(RuntimeError::Protocol(format!(
                        "{} non-final segment must end with LF",
                        segment.diagnostic_path().display()
                    )));
                }
                if index + 1 == segments.len() {
                    final_complete_bytes = offset
                        .saturating_add(u64::try_from(segment_complete_len).unwrap_or(u64::MAX));
                }
            }
            after_read();
            let complete_len = complete_jsonl_prefix_len(&suffix);
            let has_partial_line = complete_len != suffix.len();
            let inactive_partial = has_partial_line && !self.session_lock_present()?;
            if inactive_partial && !retried_inactive_partial {
                retried_inactive_partial = true;
                continue;
            }
            let appended_bytes = &suffix[..complete_len];
            let appended_text = std::str::from_utf8(appended_bytes).map_err(|source| {
                RuntimeError::Protocol(format!(
                    "{} is not valid UTF-8: {source}",
                    self.path.diagnostic_path().display()
                ))
            })?;
            let appended = match self
                .validation
                .validate_appended(self.path.diagnostic_path(), appended_text)
            {
                Ok(appended) => appended,
                Err(error) => {
                    self.reset_validation();
                    return Err(error);
                }
            };
            if has_partial_line && self.validation.terminal_line.is_some() {
                self.reset_validation();
                return Err(RuntimeError::Protocol(format!(
                    "{} contains a partial line after a terminal event",
                    self.path.diagnostic_path().display()
                )));
            }
            if inactive_partial {
                self.reset_validation();
                return Err(self.inactive_partial());
            }
            if let Err(error) = self.ensure_cursor(cursor, self.validation.previous_sequence) {
                self.reset_validation();
                return Err(error);
            }
            for record in appended_bytes.split_inclusive(|byte| *byte == b'\n') {
                self.observed_signature.push(record);
            }
            self.observed_current_segment_bytes = final_complete_bytes;
            self.observed_segment_count = segments.len();
            return Ok(events_after(appended, cursor));
        }
    }

    fn incremental_segments(&self) -> Result<Vec<AnchoredFile>, RuntimeError> {
        let observed = u64::try_from(self.observed_segment_count).unwrap_or(u64::MAX);
        let mut segments = Vec::new();
        for ordinal in 1..=observed {
            let segment = segmented_jsonl_path(&self.path, ordinal)?;
            ensure_anchored_real_file(&segment)?;
            segments.push(segment);
        }
        for ordinal in observed.saturating_add(1)..=EVENT_STREAM_LIMITS.max_segments {
            let segment = segmented_jsonl_path(&self.path, ordinal)?;
            match segment.metadata() {
                Ok(_) => ensure_anchored_real_file(&segment)?,
                Err(RuntimeError::Io { source, .. })
                    if source.kind() == io::ErrorKind::NotFound =>
                {
                    break;
                }
                Err(error) => return Err(error),
            }
            segments.push(segment);
        }
        Ok(segments)
    }

    fn session_lock_present(&self) -> Result<bool, RuntimeError> {
        match ensure_anchored_real_file(&self.lock_path) {
            Ok(()) => Ok(true),
            Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    fn inactive_partial(&self) -> RuntimeError {
        RuntimeError::Protocol(format!(
            "{} contains an incomplete final JSONL line without an active session lock",
            self.path.diagnostic_path().display()
        ))
    }

    fn reset_validation(&mut self) {
        let session_id = self
            .validation
            .expected_session_id
            .as_deref()
            .expect("session readers always validate one session")
            .to_owned();
        self.validation = SessionAppendValidationState::empty(&session_id);
    }

    fn ensure_cursor(&self, cursor: u64, latest_sequence: u64) -> Result<(), RuntimeError> {
        if cursor <= latest_sequence {
            return Ok(());
        }
        Err(RuntimeError::Protocol(format!(
            "{} no longer contains processed sequence {cursor}",
            self.path.diagnostic_path().display()
        )))
    }

    fn changed_outside_append_only(&self) -> RuntimeError {
        RuntimeError::Protocol(format!(
            "{} changed outside append-only session semantics",
            self.path.diagnostic_path().display()
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

fn events_after(mut events: Vec<EventEnvelope>, cursor: u64) -> Vec<EventEnvelope> {
    let first = events.partition_point(|event| event.sequence <= cursor);
    events.drain(..first);
    events
}

fn complete_jsonl_prefix_len(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |newline_index| newline_index + 1)
}
