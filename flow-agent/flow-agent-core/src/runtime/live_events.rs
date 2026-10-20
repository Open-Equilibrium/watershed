use std::{fmt, time::Duration};

pub const LIVE_EVENT_NOTIFICATION_CAPACITY: usize = 1;

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
    /// Conversation owning a productive Run; absent for an explicit fixture session.
    pub conversation_id: Option<String>,
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

pub struct LiveEventState {
    pub(crate) highest_committed_sequence: std::sync::atomic::AtomicU64,
}

/// Producer side of one bounded, caller-owned live-event notification channel.
///
/// This handle owns no task or thread. Pass it to one run or resume operation. Each
/// successful append advances a shared high-watermark and attempts a capacity-one wake-up.
pub struct LiveEventNotifier {
    pub(crate) sender: std::sync::mpsc::SyncSender<(Option<String>, String, u64)>,
    pub(crate) state: std::sync::Arc<LiveEventState>,
}

impl LiveEventNotifier {
    /// Advances the committed high-watermark and attempts a wake-up without waiting.
    ///
    /// Call this only after `committed_sequence` is readable from the authoritative session
    /// log. A full or closed channel never blocks and never changes persistence semantics.
    pub fn try_notify(&self, session_id: &str, committed_sequence: u64) -> LiveEventNotifyStatus {
        self.try_notify_run(None, session_id, committed_sequence)
    }

    /// Advances the committed high-watermark for one conversation-owned run.
    pub fn try_notify_conversation_run(
        &self,
        conversation_id: &str,
        run_session_id: &str,
        committed_sequence: u64,
    ) -> LiveEventNotifyStatus {
        self.try_notify_run(Some(conversation_id), run_session_id, committed_sequence)
    }

    fn try_notify_run(
        &self,
        conversation_id: Option<&str>,
        session_id: &str,
        committed_sequence: u64,
    ) -> LiveEventNotifyStatus {
        self.state
            .highest_committed_sequence
            .fetch_max(committed_sequence, std::sync::atomic::Ordering::Release);
        match self.sender.try_send((
            conversation_id.map(str::to_owned),
            session_id.to_owned(),
            committed_sequence,
        )) {
            Ok(()) => LiveEventNotifyStatus::Queued,
            Err(std::sync::mpsc::TrySendError::Full(_)) => LiveEventNotifyStatus::Coalesced,
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => LiveEventNotifyStatus::Closed,
        }
    }
}

/// Receiver side of one bounded, caller-owned live-event notification channel.
///
/// On wake-up, visit committed events after the caller's last processed sequence through
/// [`Self::highest_committed_sequence`] with [`crate::SessionEventReader::visit_incremental_after`].
/// Advance that cursor only after processing each event, then drain another wake-up before waiting
/// again. After the producer closes, use [`crate::SessionEventReader::visit_verified_after`] once to
/// verify the complete authoritative log. This closes the replay/live race because a commit either
/// advances the observed high-watermark or leaves another wake-up queued.
pub struct LiveEventReceiver {
    pub(crate) receiver: std::sync::mpsc::Receiver<(Option<String>, String, u64)>,
    pub(crate) state: std::sync::Arc<LiveEventState>,
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
        let (conversation_id, session_id, first_committed_sequence) = self
            .receiver
            .recv_timeout(timeout)
            .map_err(|err| match err {
                std::sync::mpsc::RecvTimeoutError::Timeout => LiveEventReceiveError::Timeout,
                std::sync::mpsc::RecvTimeoutError::Disconnected => LiveEventReceiveError::Closed,
            })?;
        Ok(LiveEventNotification {
            conversation_id,
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
