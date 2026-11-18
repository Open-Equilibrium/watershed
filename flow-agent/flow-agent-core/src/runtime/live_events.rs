use std::{fmt, time::Duration};

pub const LIVE_EVENT_NOTIFICATION_CAPACITY: usize = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveEventNotifyStatus {
    Queued,
    Coalesced,
    Closed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveEventNotification {
    pub conversation_id: Option<String>,
    pub session_id: String,
    pub first_committed_sequence: u64,
    pub highest_committed_sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveEventReceiveError {
    Timeout,
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

pub struct LiveEventNotifier {
    pub(crate) sender: std::sync::mpsc::SyncSender<(Option<String>, String, u64)>,
    pub(crate) state: std::sync::Arc<LiveEventState>,
}

impl LiveEventNotifier {
    pub fn try_notify(&self, session_id: &str, committed_sequence: u64) -> LiveEventNotifyStatus {
        self.try_notify_run(None, session_id, committed_sequence)
    }

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

pub struct LiveEventReceiver {
    pub(crate) receiver: std::sync::mpsc::Receiver<(Option<String>, String, u64)>,
    pub(crate) state: std::sync::Arc<LiveEventState>,
}

impl LiveEventReceiver {
    pub fn highest_committed_sequence(&self) -> u64 {
        self.state
            .highest_committed_sequence
            .load(std::sync::atomic::Ordering::Acquire)
    }

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
