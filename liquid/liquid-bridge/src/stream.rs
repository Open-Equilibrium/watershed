//! How the event forwarder batches what crosses to Dart, and how far ahead of Liquid it sends
//! (ADR-0272).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use meta_harness_core::SessionEvent;
use tokio::sync::{Notify, mpsc};

use crate::api::events::{BridgeEvent, BridgeEventKind};

/// Adds `event` to `batch`. A content delta joins the one before it when both belong to the same
/// item, and a partial output replaces the one before it for the same item, since each carries
/// the item's whole output so far; a batch holds one event per run of text however fast it
/// streams.
pub(crate) fn push(batch: &mut Vec<BridgeEvent>, event: BridgeEvent) {
    if let Some(last) = batch.last_mut()
        && last.kind == event.kind
        && last.item.as_ref().map(|item| item.id) == event.item.as_ref().map(|item| item.id)
    {
        match event.kind {
            BridgeEventKind::ItemUpdated => {
                *last = event;
                return;
            }
            BridgeEventKind::ContentDelta => {
                if let (Some(text), Some(more)) = (last.text.as_mut(), event.text.as_deref()) {
                    text.push_str(more);
                    return;
                }
            }
            _ => {}
        }
    }
    batch.push(event);
}

/// How long streaming text and partial output gather before Dart gets them, as T3 Code batches
/// its stream.
const STREAM_WINDOW: std::time::Duration = std::time::Duration::from_millis(50);

/// Most events sent to Dart and not yet applied by Liquid. It equals the core's event buffer
/// (`EVENT_BUFFER`), the largest queue the core already keeps for agent events.
pub(crate) const WINDOW: usize = 256;

/// Most events held back for Liquid while the window is full, counted after merging. It equals
/// the window, so the bridge holds at most about twice the core's buffer. Past it, partial output
/// goes and Liquid falls behind; other events are never dropped (ADR-0272).
pub(crate) const PENDING_LIMIT: usize = 256;

/// How many of the stream's events Liquid applied, as `AgentSession::confirm_applied` reports.
#[derive(Default)]
pub(crate) struct Credit {
    applied: AtomicU64,
    notify: Notify,
}

impl Credit {
    /// Records that Liquid applied its first `applied` events; a smaller count changes nothing.
    pub(crate) fn confirm(&self, applied: u64) {
        self.applied.fetch_max(applied, Ordering::AcqRel);
        self.notify.notify_one();
    }

    /// How many events Liquid applied so far.
    fn applied(&self) -> u64 {
        self.applied.load(Ordering::Acquire)
    }

    /// Waits until a confirmation may have reopened the window.
    async fn wait(&self) {
        self.notify.notified().await;
    }
}

/// What the forwarder holds for Liquid: merged events waiting for room in the window, and how
/// many events it sent.
#[derive(Default)]
pub(crate) struct Outbox {
    /// Events waiting for room, merged where they can be.
    pub(crate) pending: Vec<BridgeEvent>,
    /// Events handed to Dart so far.
    sent: u64,
    /// Set from dropping partial output until its fell-behind event goes out; further drops
    /// then add no second event.
    behind: bool,
}

impl Outbox {
    /// Queues `event` for Dart. Past the bound, the partial output waiting goes, since the reload
    /// replaces it, and one fell-behind event follows it; no other event is ever dropped.
    pub(crate) fn admit(&mut self, event: BridgeEvent) {
        push(&mut self.pending, event);
        if self.pending.len() > PENDING_LIMIT {
            let waiting = self.pending.len();
            self.pending.retain(|event| !is_partial(event));
            if self.pending.len() < waiting && !self.behind {
                self.pending
                    .push(crate::convert::event(BridgeEventKind::FellBehind));
                self.behind = true;
            }
        }
    }

    /// Whether the stream takes more events from the session. It does not while what waits is
    /// past the bound, so the core's buffer fills and pauses the agent until Liquid confirms.
    pub(crate) fn open(&self) -> bool {
        self.pending.len() <= PENDING_LIMIT
    }

    /// Hands pending events to `deliver` in order, as far as the window has room. Returns
    /// `false` once `deliver` refused one.
    fn send(&mut self, applied: u64, deliver: &mut impl FnMut(BridgeEvent) -> bool) -> bool {
        let unconfirmed = self.sent.saturating_sub(applied);
        let room = (WINDOW as u64).saturating_sub(unconfirmed);
        let count = room.min(self.pending.len() as u64) as usize;
        for event in self.pending.drain(..count) {
            if event.kind == BridgeEventKind::FellBehind {
                self.behind = false;
            }
            self.sent += 1;
            if !deliver(event) {
                return false;
            }
        }
        true
    }
}

/// Whether `event` is partial output, which a later event of the same item replaces or extends.
fn is_partial(event: &BridgeEvent) -> bool {
    matches!(
        event.kind,
        BridgeEventKind::ContentDelta | BridgeEventKind::ItemUpdated
    )
}

/// Waits until a confirmation reopens the window, which gives `None`, or the next event arrives,
/// which gives `Some`: `Some(None)` once the session's events ended.
async fn next_while_full(
    credit: &Credit,
    receiver: &mut mpsc::Receiver<SessionEvent>,
) -> Option<Option<SessionEvent>> {
    let mut reopened = Box::pin(credit.wait());
    std::future::poll_fn(|context| {
        if reopened.as_mut().poll(context).is_ready() {
            return std::task::Poll::Ready(None);
        }
        receiver.poll_recv(context).map(Some)
    })
    .await
}

/// Hands the session's events to `deliver` until it returns `false`, those from before the start
/// first. It sends at most `WINDOW` events that `credit` has not confirmed; what waits meanwhile
/// merges, and an overflow ends in one fell-behind event (ADR-0272).
pub(crate) async fn forward(
    early: Vec<SessionEvent>,
    mut receiver: mpsc::Receiver<SessionEvent>,
    credit: Arc<Credit>,
    mut deliver: impl FnMut(BridgeEvent) -> bool,
) {
    let mut outbox = Outbox::default();
    for event in early {
        outbox.admit(BridgeEvent::from(event));
    }
    let mut closed = false;
    loop {
        if !outbox.send(credit.applied(), &mut deliver) {
            // Dart closed the stream: stop; dropping the session ends the agent.
            return;
        }
        if outbox.pending.is_empty() {
            // Nothing waits: take the next event with the text that streams after it.
            let Some(event) = receiver.recv().await else {
                return;
            };
            outbox.admit(BridgeEvent::from(event));
            // Bounded by the window, so a busy agent never holds delivery back.
            gather(
                &mut outbox,
                &mut receiver,
                tokio::time::sleep(STREAM_WINDOW),
            )
            .await;
        } else if closed || !outbox.open() {
            // Nothing more is taken until Liquid confirms what waits; the agent ends with its
            // events, so its exit goes out after them.
            credit.wait().await;
        } else {
            // The window is full: events keep merging until a confirmation sends what waits.
            match next_while_full(&credit, &mut receiver).await {
                Some(Some(event)) => outbox.admit(BridgeEvent::from(event)),
                Some(None) => closed = true,
                None => {}
            }
        }
    }
}

/// Admits what arrives while `outbox` ends in streaming text or partial output and `window` has
/// not passed, so Dart gets at most one update per item per window (client architecture rule 4);
/// anything else ends the gathering at once.
pub(crate) async fn gather(
    outbox: &mut Outbox,
    receiver: &mut mpsc::Receiver<SessionEvent>,
    window: impl Future<Output = ()>,
) {
    // Boxed, as `pin!` expands to `super let`, which flutter_rust_bridge's parser rejects.
    let mut window = Box::pin(window);
    while outbox.open() && outbox.pending.last().is_some_and(is_partial) {
        let next = std::future::poll_fn(|context| {
            if window.as_mut().poll(context).is_ready() {
                return std::task::Poll::Ready(None);
            }
            receiver.poll_recv(context).map(Some)
        })
        .await;
        match next {
            Some(Some(event)) => outbox.admit(BridgeEvent::from(event)),
            _ => break,
        }
    }
}
