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
    pub fn try_notify(
        &self,
        session_id: &str,
        committed_sequence: u64,
    ) -> LiveEventNotifyStatus {
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
/// [`SessionEventReader::read_after`]. Advance that cursor only after processing each event,
/// then drain another wake-up before waiting again. This closes the replay/live race because
/// a commit either advances the observed high-watermark or leaves another wake-up queued.
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
        let session_id = self.receiver.recv_timeout(timeout).map_err(|err| match err {
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
    let (sender, receiver) =
        std::sync::mpsc::sync_channel(LIVE_EVENT_NOTIFICATION_CAPACITY);
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
    observed_bytes: Vec<u8>,
    path: PathBuf,
    validation: SessionAppendValidationState,
}

impl SessionEventReader {
    /// Opens a session's validated log boundary without reading event payloads yet.
    pub fn open(
        workspace: impl AsRef<Path>,
        session_id: &str,
    ) -> Result<Self, RuntimeError> {
        let workspace = workspace.as_ref();
        let path = session_path(workspace, session_id)?;
        ensure_existing_session_log_path(workspace, &path)?;
        Ok(Self {
            observed: Vec::new(),
            observed_bytes: Vec::new(),
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
        if complete_len < self.observed_bytes.len()
            || bytes[..self.observed_bytes.len()] != self.observed_bytes
        {
            return Err(RuntimeError::Protocol(format!(
                "{} changed outside append-only session semantics",
                self.path.display()
            )));
        }
        let appended_bytes = &bytes[self.observed_bytes.len()..complete_len];
        let appended_text = std::str::from_utf8(appended_bytes).map_err(|source| {
            RuntimeError::Protocol(format!("{} is not valid UTF-8: {source}", self.path.display()))
        })?;
        let (appended, next_validation) = if appended_text.is_empty() {
            (Vec::new(), None)
        } else {
            let mut validation = self.validation.clone();
            let appended = validation.validate_appended(&self.path, appended_text)?;
            (appended, Some(validation))
        };
        let validation = next_validation.as_ref().unwrap_or(&self.validation);
        if has_partial_line && validation.terminal_line.is_some() {
            return Err(RuntimeError::Protocol(format!(
                "{} contains a partial line after a terminal event",
                self.path.display()
            )));
        }
        let latest_sequence = validation.previous_sequence;
        if cursor > latest_sequence {
            return Err(RuntimeError::Protocol(format!(
                "{} no longer contains processed sequence {cursor}",
                self.path.display()
            )));
        }
        if let Some(validation) = next_validation {
            self.observed_bytes.extend_from_slice(appended_bytes);
            self.observed.extend(appended);
            self.validation = validation;
        }
        Ok(self
            .observed
            .iter()
            .filter(|event| event.sequence > cursor)
            .cloned()
            .collect())
    }
}

fn complete_jsonl_prefix_len(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |newline_index| newline_index + 1)
}
