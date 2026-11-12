//! How the event forwarder batches what crosses to Dart.

use meta_harness_core::SessionEvent;
use tokio::sync::mpsc;

use crate::api::events::{BridgeEvent, BridgeEventKind};

/// Adds `event` to `batch`, appending a content delta to the one before it when both belong to
/// the same item, so a batch holds one event per run of text however fast it streams.
pub(crate) fn push(batch: &mut Vec<BridgeEvent>, event: BridgeEvent) {
    if let Some(last) = batch.last_mut()
        && last.kind == BridgeEventKind::ContentDelta
        && event.kind == BridgeEventKind::ContentDelta
        && last.item.as_ref().map(|item| item.id) == event.item.as_ref().map(|item| item.id)
        && let (Some(text), Some(more)) = (last.text.as_mut(), event.text.as_deref())
    {
        text.push_str(more);
        return;
    }
    batch.push(event);
}

/// How long streaming text gathers before Dart gets it, as T3 Code batches its stream.
const STREAM_WINDOW: std::time::Duration = std::time::Duration::from_millis(50);

/// Hands the session's events to `deliver` in batches until it returns `false`, those from
/// before the start first.
pub(crate) async fn forward(
    early: Vec<SessionEvent>,
    mut receiver: mpsc::Receiver<SessionEvent>,
    mut deliver: impl FnMut(BridgeEvent) -> bool,
) {
    let mut batch = Vec::new();
    for event in early {
        push(&mut batch, BridgeEvent::from(event));
    }
    if !std::mem::take(&mut batch).into_iter().all(&mut deliver) {
        return;
    }
    while let Some(event) = receiver.recv().await {
        push(&mut batch, BridgeEvent::from(event));
        // Bounded by the window, so a busy agent never holds delivery back.
        gather(&mut batch, &mut receiver, tokio::time::sleep(STREAM_WINDOW)).await;
        // Dart closed the stream: stop; dropping the session ends the agent.
        if !std::mem::take(&mut batch).into_iter().all(&mut deliver) {
            break;
        }
    }
}

/// Adds what arrives to `batch` while it ends in streaming text and `window` has not passed, so
/// Dart gets at most one update per item per window (client architecture rule 4); anything
/// else ends the gathering at once.
pub(crate) async fn gather(
    batch: &mut Vec<BridgeEvent>,
    receiver: &mut mpsc::Receiver<SessionEvent>,
    window: impl Future<Output = ()>,
) {
    // Boxed, as `pin!` expands to `super let`, which flutter_rust_bridge's parser rejects.
    let mut window = Box::pin(window);
    while batch
        .last()
        .is_some_and(|event| event.kind == BridgeEventKind::ContentDelta)
    {
        let next = std::future::poll_fn(|context| {
            if window.as_mut().poll(context).is_ready() {
                return std::task::Poll::Ready(None);
            }
            receiver.poll_recv(context).map(Some)
        })
        .await;
        match next {
            Some(Some(event)) => push(batch, BridgeEvent::from(event)),
            _ => break,
        }
    }
}
