//! How streams cross to Dart: how the event forwarder batches them and how far ahead of Liquid it
//! sends (ADR-0272), and when the follows of a session or checkout end (`until_ended`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use meta_harness_core::ThreadEvent;
use tokio::sync::{Notify, mpsc, watch};

use crate::api::events::{BridgeEvent, BridgeEventKind};

/// Adds `event` to `batch`. A content delta joins the one before it when both belong to the same
/// item, taking its sequence, so a batch holds one event per run of text however fast it
/// streams.
pub(crate) fn push(batch: &mut Vec<BridgeEvent>, event: BridgeEvent) {
    if let Some(last) = batch.last_mut()
        && last.kind == event.kind
        && event.kind == BridgeEventKind::ContentDelta
        && last.item.as_ref().map(|item| item.id) == event.item.as_ref().map(|item| item.id)
        && let (Some(text), Some(more)) = (last.text.as_mut(), event.text.as_deref())
    {
        text.push_str(more);
        last.sequence = event.sequence;
        return;
    }
    batch.push(event);
}

/// How long streaming text and partial output gather before Dart gets them, as T3 Code batches
/// its stream.
const STREAM_WINDOW: std::time::Duration = std::time::Duration::from_millis(50);

/// How long a stream waits before it resubscribes after a subscription that closed before any
/// event; a thread that ended may restart, so the stream keeps trying until its session drops.
const RESUBSCRIBE_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

/// Most events sent to Dart and not yet applied by Liquid.
pub(crate) const WINDOW: usize = 256;

/// Most events held back for Liquid while the window is full, counted after merging. It equals
/// the window, so the bridge holds at most twice the window. Past it, partial output
/// goes and Liquid falls behind; other events are never dropped (ADR-0272).
pub(crate) const PENDING_LIMIT: usize = 256;

/// How many of the stream's events Liquid applied, as `AgentSession::confirm_applied` reports,
/// and whether the session that owns the stream was dropped and no confirmation will come.
#[derive(Default)]
pub(crate) struct Credit {
    applied: AtomicU64,
    closed: AtomicBool,
    notify: Notify,
}

impl Credit {
    /// Records that Liquid applied its first `applied` events; a smaller count changes nothing.
    pub(crate) fn confirm(&self, applied: u64) {
        self.applied.fetch_max(applied, Ordering::AcqRel);
        self.notify.notify_one();
    }

    /// Records that the session was dropped, which ends the stream's wait for Liquid.
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.notify.notify_one();
    }

    /// How many events Liquid applied so far.
    fn applied(&self) -> u64 {
        self.applied.load(Ordering::Acquire)
    }

    /// Whether the session was dropped.
    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// Waits until a confirmation may have reopened the window, or the session was dropped.
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
                self.fell_behind();
            }
        }
    }

    /// Queues one fell-behind event: Liquid reloads from a snapshot (ADR-0290).
    pub(crate) fn fell_behind(&mut self) {
        let sequence = self.pending.last().map_or(0, |event| event.sequence);
        self.pending
            .push(crate::convert::event(BridgeEventKind::FellBehind, sequence));
        self.behind = true;
    }

    /// Whether the stream takes more events from the session. It does not while what waits is
    /// past the bound: the events then queue in this subscriber's channel, and a full channel
    /// makes the core drop the subscriber rather than pause the agent, so the stream resubscribes
    /// behind a fell-behind event.
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

/// Whether `event` is streaming text, which a later event of the same item extends.
fn is_partial(event: &BridgeEvent) -> bool {
    event.kind == BridgeEventKind::ContentDelta
}

/// Waits until a confirmation or the session's drop wakes the forwarder, which gives `None`, or
/// the next event arrives, which gives `Some`: `Some(None)` once the session's events ended.
async fn next_or_wake(
    credit: &Credit,
    receiver: &mut mpsc::Receiver<ThreadEvent>,
) -> Option<Option<ThreadEvent>> {
    let mut woken = Box::pin(credit.wait());
    std::future::poll_fn(|context| {
        if woken.as_mut().poll(context).is_ready() {
            return std::task::Poll::Ready(None);
        }
        receiver.poll_recv(context).map(Some)
    })
    .await
}

/// Waits for `next` unless the owner of the stream drops first, which gives `None`. The owner
/// holds the sender of `ended` and never sends on it, so its drop ends the wait at once; the
/// stream then ends with its owner instead of at its next change.
pub(crate) async fn until_ended<T>(
    ended: &mut watch::Receiver<()>,
    next: impl Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        biased;
        _ = ended.changed() => None,
        value = next => Some(value),
    }
}

/// Hands the thread's events to `deliver` until it returns `false` or the session is dropped.
/// It sends at most `WINDOW` events that `credit` has not confirmed; what waits meanwhile
/// merges, and an overflow ends in one fell-behind event (ADR-0272). When the core drops this
/// subscriber, which fell too far behind, the stream takes `resubscribe`'s events after a
/// fell-behind event.
pub(crate) async fn forward(
    mut receiver: mpsc::Receiver<ThreadEvent>,
    mut resubscribe: impl FnMut() -> mpsc::Receiver<ThreadEvent>,
    credit: Arc<Credit>,
    mut deliver: impl FnMut(BridgeEvent) -> bool,
) {
    let mut outbox = Outbox::default();
    // Whether `receiver` came from `resubscribe` and has yielded no event yet.
    let mut resubscribed = false;
    loop {
        if credit.is_closed() {
            // The session was dropped, so no confirmation comes; what waits is released with it.
            return;
        }
        if !outbox.send(credit.applied(), &mut deliver) {
            // Dart closed the stream.
            return;
        }
        if !outbox.open() {
            // Nothing more is taken until Liquid confirms what waits.
            credit.wait().await;
            continue;
        }
        // Events keep merging until a confirmation sends what waits; with nothing waiting, the
        // next event starts a batch.
        let idle = outbox.pending.is_empty();
        match next_or_wake(&credit, &mut receiver).await {
            Some(Some(event)) => {
                resubscribed = false;
                outbox.admit(BridgeEvent::from(event));
                if idle {
                    // Bounded by the window, so a busy agent never holds delivery back.
                    gather(
                        &mut outbox,
                        &mut receiver,
                        tokio::time::sleep(STREAM_WINDOW),
                    )
                    .await;
                }
            }
            Some(None) => {
                if resubscribed {
                    // The new subscription closed before any event: the thread may have ended.
                    // Retry after a pause, and not at all once the session dropped.
                    tokio::select! {
                        _ = credit.wait() => {}
                        _ = tokio::time::sleep(RESUBSCRIBE_DELAY) => {}
                    }
                    if credit.is_closed() {
                        return;
                    }
                }
                receiver = resubscribe();
                resubscribed = true;
                if !outbox.behind {
                    outbox.fell_behind();
                }
            }
            None => {}
        }
    }
}

/// Admits what arrives while `outbox` ends in streaming text or partial output and `window` has
/// not passed, so Dart gets at most one update per item per window (client architecture rule 4);
/// anything else ends the gathering at once.
pub(crate) async fn gather(
    outbox: &mut Outbox,
    receiver: &mut mpsc::Receiver<ThreadEvent>,
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
