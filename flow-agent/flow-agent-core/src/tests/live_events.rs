use crate::runtime::live_events::{
    LiveEventNotification, LiveEventNotifyStatus, LiveEventReceiveError, live_event_channel,
};
use std::time::Duration;

#[test]
fn live_notification_receive_errors_have_stable_operator_diagnostics() {
    for (error, expected) in [
        (
            LiveEventReceiveError::Timeout,
            "live-event notification timed out",
        ),
        (
            LiveEventReceiveError::Closed,
            "live-event notification channel is closed",
        ),
    ] {
        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn live_notification_is_bounded_coalesced_and_non_blocking() {
    let (notifier, receiver) = live_event_channel();

    assert_eq!(
        notifier.try_notify("bounded001", 1),
        LiveEventNotifyStatus::Queued
    );
    assert_eq!(
        notifier.try_notify("bounded001", 2),
        LiveEventNotifyStatus::Coalesced
    );
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_millis(50))
            .expect("pending notification is received"),
        LiveEventNotification {
            conversation_id: None,
            session_id: "bounded001".to_owned(),
            first_committed_sequence: 1,
            highest_committed_sequence: 2,
        }
    );
    assert_eq!(
        notifier.try_notify("bounded001", 3),
        LiveEventNotifyStatus::Queued
    );
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_millis(50))
            .expect("next notification is received"),
        LiveEventNotification {
            conversation_id: None,
            session_id: "bounded001".to_owned(),
            first_committed_sequence: 3,
            highest_committed_sequence: 3,
        }
    );
    drop(receiver);
    assert_eq!(
        notifier.try_notify("bounded001", 4),
        LiveEventNotifyStatus::Closed
    );
}
