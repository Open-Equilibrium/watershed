//! Starting an agent: waiting until it confirms Watershed's extension, and why it ended.

use std::future::Future;
use std::task::Poll;
use std::time::Duration;

use meta_harness_core::{CommandError, Session, SessionEvent};
use tokio::sync::mpsc;

use crate::api::session::SHUTDOWN_GRACE;

/// Time an agent gets to confirm Watershed's extension after it starts.
#[cfg(not(test))]
pub(crate) const START_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(test)]
pub(crate) const START_TIMEOUT: Duration = Duration::from_secs(2);

/// Most events kept from an agent that is starting; one that says more is not ready.
const EARLY_EVENTS: usize = 4096;

/// What an agent that ended wrote to its error output, once it reports its exit.
pub(crate) async fn error_output(receiver: &mut mpsc::Receiver<SessionEvent>) -> Option<String> {
    let exit = async {
        while let Some(event) = receiver.recv().await {
            if let SessionEvent::Exited { error_output, .. } = event {
                return Some(error_output);
            }
        }
        None
    };
    tokio::time::timeout(SHUTDOWN_GRACE, exit)
        .await
        .ok()
        .flatten()
}

/// Waits until the agent confirms Watershed's extension, keeping the events it sends meanwhile
/// so they cannot fill the session's channel; `None` when it does not answer in time or says
/// too much first.
pub(crate) async fn ready(
    session: &mut Session,
    receiver: &mut mpsc::Receiver<SessionEvent>,
) -> (Option<Result<(), CommandError>>, Vec<SessionEvent>) {
    let mut early = Vec::new();
    // Boxed rather than `pin!`, which flutter_rust_bridge's code generator cannot parse.
    let mut check = Box::pin(session.require_gate());
    let mut limit = Box::pin(tokio::time::sleep(START_TIMEOUT));
    let gate = std::future::poll_fn(|context| {
        if let Poll::Ready(gate) = check.as_mut().poll(context) {
            return Poll::Ready(Some(gate));
        }
        while early.len() < EARLY_EVENTS {
            match receiver.poll_recv(context) {
                Poll::Ready(Some(event)) => early.push(event),
                _ => break,
            }
        }
        if limit.as_mut().poll(context).is_ready() {
            return Poll::Ready(None);
        }
        Poll::Pending
    })
    .await;
    (gate, early)
}
