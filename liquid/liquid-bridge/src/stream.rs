//! How streams cross to Dart: how the event forwarder batches them and how far ahead of Liquid it
//! sends (ADR-0272), and when the follows of a session or checkout end (`until_ended`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use meta_harness_core::remote::connection_lost;
use meta_harness_core::{ThreadEvent, Watch};
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

/// Most events sent to Dart and not yet applied by Liquid.
pub(crate) const WINDOW: usize = 256;

/// Most events held back for Liquid while the window is full, counted after merging. It equals
/// the window, so the bridge holds at most twice the window. Past it, partial output
/// goes and Liquid falls behind; other events are never dropped (ADR-0272).
pub(crate) const PENDING_LIMIT: usize = 256;

/// The session's window (ADR-0272): how many events its streams sent to Dart, how many Liquid
/// applied (as `AgentSession::confirm_applied` reports), and whether the session was dropped and no
/// confirmation will come.
pub(crate) struct Credit {
    applied: AtomicU64,
    /// Events sent to Dart by the session's streams, which the window counts from.
    sent: Mutex<u64>,
    /// Set once the session was dropped, which every waiter sees, including a later one.
    closed: watch::Sender<bool>,
    notify: Notify,
}

impl Default for Credit {
    fn default() -> Self {
        Self {
            applied: AtomicU64::default(),
            sent: Mutex::default(),
            closed: watch::channel(false).0,
            notify: Notify::default(),
        }
    }
}

impl Credit {
    /// Records that Liquid applied its first `applied` events; a smaller count changes nothing.
    pub(crate) fn confirm(&self, applied: u64) {
        self.applied.fetch_max(applied, Ordering::AcqRel);
        self.notify.notify_one();
    }

    /// Records that the session was dropped, which ends the stream's wait for Liquid.
    pub(crate) fn close(&self) {
        self.closed.send_replace(true);
    }

    /// How many events Liquid applied so far.
    fn applied(&self) -> u64 {
        self.applied.load(Ordering::Acquire)
    }

    fn lock_sent(&self) -> MutexGuard<'_, u64> {
        self.sent.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes room for up to `wanted` events and gives how many it took: the session's events sent
    /// and not yet applied stay within `WINDOW`, whichever of its streams sent them.
    fn reserve(&self, wanted: usize) -> usize {
        let mut sent = self.lock_sent();
        let unconfirmed = sent.saturating_sub(self.applied());
        let room = (WINDOW as u64).saturating_sub(unconfirmed);
        let count = room.min(wanted as u64);
        *sent += count;
        count as usize
    }

    /// Gives back `count` events that `reserve` took but that were not sent.
    fn release(&self, count: usize) {
        *self.lock_sent() -= count as u64;
    }

    /// Whether the session was dropped.
    fn is_closed(&self) -> bool {
        *self.closed.borrow()
    }

    /// Waits until the session was dropped.
    async fn wait_closed(&self) {
        let _ = self.closed.subscribe().wait_for(|closed| *closed).await;
    }

    /// Waits until a confirmation may have reopened the window, or the session was dropped.
    async fn wait(&self) {
        tokio::select! {
            () = self.notify.notified() => {}
            () = self.wait_closed() => {}
        }
    }
}

/// What the forwarder holds for Liquid: merged events waiting for room in the window.
#[derive(Default)]
pub(crate) struct Outbox {
    /// Events waiting for room, merged where they can be.
    pub(crate) pending: Vec<BridgeEvent>,
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

    /// Hands pending events to `deliver` in order, as far as the session's window has room. Returns
    /// `false` once `deliver` refused one.
    fn send(&mut self, credit: &Credit, deliver: &mut impl FnMut(BridgeEvent) -> bool) -> bool {
        let count = credit.reserve(self.pending.len());
        let mut delivered = 0;
        for event in self.pending.drain(..count) {
            if event.kind == BridgeEventKind::FellBehind {
                self.behind = false;
            }
            if !deliver(event) {
                // What was not handed over leaves the window, so the session's other streams can
                // use it.
                credit.release(count - delivered);
                return false;
            }
            delivered += 1;
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

/// The counter of the current stream of one kind. A new stream or a stop bumps it, which ends the
/// stream before it (see [`until_ended`]).
pub(crate) struct Generation(Mutex<watch::Sender<u64>>);

impl Default for Generation {
    fn default() -> Self {
        Self(Mutex::new(watch::channel(0).0))
    }
}

impl Generation {
    /// Ends the current stream and gives the new one's end signal. The bump and the subscription
    /// share one lock, so a concurrent replacement cannot slip in between them.
    pub(crate) fn replace(&self) -> watch::Receiver<u64> {
        let sender = self.lock();
        sender.send_modify(|generation| *generation += 1);
        sender.subscribe()
    }

    /// Ends the current stream.
    pub(crate) fn stop(&self) {
        self.lock().send_modify(|generation| *generation += 1);
    }

    fn lock(&self) -> MutexGuard<'_, watch::Sender<u64>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Waits for `next` unless `ended` ends first, which gives `None`. The owner of the stream holds
/// the sender of `ended`: its drop ends the wait at once, and so does a change of a [`Generation`],
/// which a newer stream of its kind or a stop makes. The stream then ends at once, not at its
/// next change.
pub(crate) async fn until_ended<S, T>(
    ended: &mut watch::Receiver<S>,
    next: impl Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        biased;
        _ = ended.changed() => None,
        value = next => Some(value),
    }
}

/// The first pause before Meta-Harness is asked again, doubling after each failed attempt up to
/// `RETRY_MAX`.
const RETRY_FIRST: std::time::Duration = std::time::Duration::from_millis(500);
const RETRY_MAX: std::time::Duration = std::time::Duration::from_secs(5);

/// Asks Meta-Harness through `attempt` until it answers. A lost or missing connection means
/// Meta-Harness cannot be reached now, such as while it restarts, so the next attempt comes after a
/// pause. Any other error is the answer. Gives `None` once `ended` ends (see [`until_ended`]).
pub(crate) async fn reconnect<S, T, Fut: Future<Output = Result<T, String>>>(
    ended: &mut watch::Receiver<S>,
    mut attempt: impl FnMut() -> Fut,
) -> Option<Result<T, String>> {
    let mut pause = RETRY_FIRST;
    loop {
        match until_ended(ended, attempt()).await? {
            Err(error) if connection_lost(&error) => {}
            answer => return Some(answer),
        }
        until_ended(ended, tokio::time::sleep(pause)).await?;
        pause = (pause * 2).min(RETRY_MAX);
    }
}

/// Forwards each change of `watch` to `deliver` until `deliver` refuses one or `ended` ends (see
/// [`until_ended`]). Once the watch's stream ended, `open` reopens it (see [`reconnect`]), and its
/// current value goes out, since changes may have been missed meanwhile. Any other error ends the
/// follow, as a watch has no channel to report it.
pub(crate) async fn follow_watch<S, T: Clone, Fut: Future<Output = Result<Watch<T>, String>>>(
    mut watch: Watch<T>,
    ended: &mut watch::Receiver<S>,
    mut open: impl FnMut() -> Fut,
    mut deliver: impl FnMut(T) -> bool,
) {
    loop {
        match until_ended(ended, watch.changed()).await {
            None => return,
            Some(Some(value)) => {
                if !deliver(value) {
                    return;
                }
            }
            Some(None) => {
                let Some(Ok(reopened)) = reconnect(ended, &mut open).await else {
                    return;
                };
                watch = reopened;
                if !deliver(watch.current()) {
                    return;
                }
            }
        }
    }
}

/// Hands the thread's events to `deliver` until it returns `false` or the session is dropped.
/// It sends at most `WINDOW` events that `credit` has not confirmed; what waits meanwhile
/// merges, and an overflow ends in one fell-behind event (ADR-0272). When the core drops this
/// subscriber, which fell too far behind, or a reconnect restored the thread, the stream takes
/// `resubscribe`'s events after a fell-behind event; `resubscribe` gives none once the thread
/// ended for good, and the stream ends once what waits is sent.
pub(crate) async fn forward<Next: Future<Output = Option<mpsc::Receiver<ThreadEvent>>>>(
    mut receiver: mpsc::Receiver<ThreadEvent>,
    mut resubscribe: impl FnMut() -> Next,
    credit: Arc<Credit>,
    mut deliver: impl FnMut(BridgeEvent) -> bool,
) {
    let mut outbox = Outbox::default();
    loop {
        if credit.is_closed() {
            // The session was dropped, so no confirmation comes; what waits is released with it.
            return;
        }
        if !outbox.send(&credit, &mut deliver) {
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
                let next = tokio::select! {
                    next = resubscribe() => next,
                    // Only the session's drop ends the wait; an ordinary confirmation leaves the
                    // attempt pending.
                    () = credit.wait_closed() => return,
                };
                let Some(next) = next else {
                    // T3 ended the thread for good: no event comes, but what waits still goes.
                    return send_waiting(&mut outbox, &credit, &mut deliver).await;
                };
                receiver = next;
                if !outbox.behind {
                    outbox.fell_behind();
                }
            }
            None => {}
        }
    }
}

/// Sends what waits once no event comes, as each confirmation reopens room for it. Returns once
/// nothing waits, the session was dropped or `deliver` refused an event.
async fn send_waiting(
    outbox: &mut Outbox,
    credit: &Credit,
    deliver: &mut impl FnMut(BridgeEvent) -> bool,
) {
    while !credit.is_closed() && outbox.send(credit, deliver) && !outbox.pending.is_empty() {
        credit.wait().await;
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
