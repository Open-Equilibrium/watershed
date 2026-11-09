//! Starting an agent: waiting until it confirms Watershed's extension, and why it ended.

use std::future::Future;
use std::task::Poll;
use std::time::Duration;

use meta_harness_core::{CommandError, Session, SessionEvent};
use tokio::sync::mpsc;

/// Time an agent gets to exit after its input closes before it is killed.
pub(crate) const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// Time an agent gets to confirm Watershed's extension after it starts.
#[cfg(not(test))]
const START_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(test)]
const START_TIMEOUT: Duration = Duration::from_secs(2);

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
/// so they cannot fill the session's channel; why it is not ready when it does not answer in
/// time or says too much first.
pub(crate) async fn ready(
    session: &mut Session,
    receiver: &mut mpsc::Receiver<SessionEvent>,
) -> (Result<Result<(), CommandError>, String>, Vec<SessionEvent>) {
    let mut early = Vec::new();
    // Boxed rather than `pin!`, which flutter_rust_bridge's code generator cannot parse.
    let mut check = Box::pin(session.require_extension());
    let mut limit = Box::pin(tokio::time::sleep(START_TIMEOUT));
    let gate = std::future::poll_fn(|context| {
        let checked = check.as_mut().poll(context);
        // Events the agent sent before its answer are in the channel once the answer arrived.
        while let Poll::Ready(Some(event)) = receiver.poll_recv(context) {
            if early.len() == EARLY_EVENTS {
                return Poll::Ready(Err(format!(
                    "pi sent more than {EARLY_EVENTS} events before it was ready"
                )));
            }
            early.push(event);
        }
        if let Poll::Ready(gate) = checked {
            return Poll::Ready(Ok(gate));
        }
        if limit.as_mut().poll(context).is_ready() {
            return Poll::Ready(Err(format!(
                "pi did not answer within {} s of starting",
                START_TIMEOUT.as_secs()
            )));
        }
        Poll::Pending
    })
    .await;
    (gate, early)
}
