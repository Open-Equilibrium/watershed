use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use meta_harness_core::remote::connection_lost;
use meta_harness_core::{ThreadEvent, Watch};
use tokio::sync::{Notify, mpsc, watch};

use crate::api::events::{BridgeEvent, BridgeEventKind};
use crate::frb_generated::{SseEncode, StreamSink};

pub(crate) fn push(batch: &mut Vec<BridgeEvent>, event: BridgeEvent, merge_point: Option<i64>) {
    if let Some(last) = batch.last_mut()
        && merge_point.is_some_and(|point| last.sequence > point || event.sequence <= point)
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

const STREAM_WINDOW: std::time::Duration = std::time::Duration::from_millis(50);

pub(crate) const WINDOW: usize = 256;

pub(crate) const PENDING_LIMIT: usize = 256;

pub(crate) struct Credit {
    applied: AtomicU64,
    snapshots_taken: AtomicU64,
    snapshot_sequence: AtomicI64,
    closed: watch::Sender<bool>,
    notify: Notify,
}

impl Default for Credit {
    fn default() -> Self {
        Self {
            applied: AtomicU64::default(),
            snapshots_taken: AtomicU64::default(),
            snapshot_sequence: AtomicI64::default(),
            closed: watch::channel(false).0,
            notify: Notify::default(),
        }
    }
}

impl Credit {
    pub(crate) fn confirm(&self, applied: u64) {
        self.applied.fetch_max(applied, Ordering::AcqRel);
        self.notify.notify_one();
    }

    pub(crate) fn close(&self) {
        self.closed.send_replace(true);
    }

    fn applied(&self) -> u64 {
        self.applied.load(Ordering::Acquire)
    }

    pub(crate) fn begin_snapshot(&self) {
        self.snapshots_taken.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn end_snapshot(&self, sequence: Option<i64>) {
        if let Some(sequence) = sequence {
            self.snapshot_sequence.fetch_max(sequence, Ordering::AcqRel);
        }
        self.snapshots_taken.fetch_sub(1, Ordering::AcqRel);
    }

    fn merge_point(&self) -> Option<i64> {
        if self.snapshots_taken.load(Ordering::Acquire) > 0 {
            None
        } else {
            Some(self.snapshot_sequence.load(Ordering::Acquire))
        }
    }

    fn is_closed(&self) -> bool {
        *self.closed.borrow()
    }

    async fn wait_closed(&self) {
        let _ = self.closed.subscribe().wait_for(|closed| *closed).await;
    }

    async fn wait(&self) {
        tokio::select! {
            () = self.notify.notified() => {}
            () = self.wait_closed() => {}
        }
    }
}

#[derive(Default)]
pub(crate) struct Outbox {
    pub(crate) pending: Vec<BridgeEvent>,
    sent: u64,
    behind: bool,
}

impl Outbox {
    pub(crate) fn admit(&mut self, event: BridgeEvent, credit: &Credit) {
        push(&mut self.pending, event, credit.merge_point());
        if self.pending.len() > PENDING_LIMIT {
            let waiting = self.pending.len();
            self.pending.retain(|event| !is_partial(event));
            if self.pending.len() < waiting && !self.behind {
                self.fell_behind();
            }
        }
    }

    pub(crate) fn fell_behind(&mut self) {
        let sequence = self.pending.last().map_or(0, |event| event.sequence);
        self.pending
            .push(crate::convert::event(BridgeEventKind::FellBehind, sequence));
        self.behind = true;
    }

    pub(crate) fn open(&self) -> bool {
        self.pending.len() <= PENDING_LIMIT
    }

    fn send(&mut self, credit: &Credit, deliver: &mut impl FnMut(BridgeEvent) -> bool) -> bool {
        let unconfirmed = self.sent.saturating_sub(credit.applied());
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

fn is_partial(event: &BridgeEvent) -> bool {
    event.kind == BridgeEventKind::ContentDelta
}

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

pub(crate) struct Generation(Mutex<watch::Sender<u64>>);

impl Default for Generation {
    fn default() -> Self {
        Self(Mutex::new(watch::channel(0).0))
    }
}

impl Generation {
    pub(crate) fn replace(&self) -> watch::Receiver<u64> {
        let sender = self.lock();
        sender.send_modify(|generation| *generation += 1);
        sender.subscribe()
    }

    pub(crate) fn stop(&self) {
        self.lock().send_modify(|generation| *generation += 1);
    }

    fn lock(&self) -> MutexGuard<'_, watch::Sender<u64>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

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

const RETRY_FIRST: std::time::Duration = std::time::Duration::from_millis(500);
const RETRY_MAX: std::time::Duration = std::time::Duration::from_secs(5);

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

pub(crate) async fn forward<Next: Future<Output = Option<mpsc::Receiver<ThreadEvent>>>>(
    mut receiver: mpsc::Receiver<ThreadEvent>,
    mut resubscribe: impl FnMut() -> Next,
    credit: Arc<Credit>,
    mut deliver: impl FnMut(BridgeEvent) -> bool,
) {
    let mut outbox = Outbox::default();
    loop {
        if credit.is_closed() {
            return;
        }
        if !outbox.send(&credit, &mut deliver) {
            return;
        }
        if !outbox.open() {
            credit.wait().await;
            continue;
        }
        let idle = outbox.pending.is_empty();
        match next_or_wake(&credit, &mut receiver).await {
            Some(Some(event)) => {
                outbox.admit(BridgeEvent::from(event), &credit);
                if idle {
                    gather(
                        &mut outbox,
                        &mut receiver,
                        &credit,
                        tokio::time::sleep(STREAM_WINDOW),
                    )
                    .await;
                }
            }
            Some(None) => {
                let mut attempt = Box::pin(resubscribe());
                let next = loop {
                    tokio::select! {
                        next = &mut attempt => break next,
                        () = credit.wait() => {
                            if credit.is_closed() || !outbox.send(&credit, &mut deliver) {
                                return;
                            }
                        }
                    }
                };
                let Some(next) = next else {
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

async fn send_waiting(
    outbox: &mut Outbox,
    credit: &Credit,
    deliver: &mut impl FnMut(BridgeEvent) -> bool,
) {
    while !credit.is_closed() && outbox.send(credit, deliver) && !outbox.pending.is_empty() {
        credit.wait().await;
    }
}

pub(crate) async fn gather(
    outbox: &mut Outbox,
    receiver: &mut mpsc::Receiver<ThreadEvent>,
    credit: &Credit,
    window: impl Future<Output = ()>,
) {
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
            Some(Some(event)) => outbox.admit(BridgeEvent::from(event), credit),
            _ => break,
        }
    }
}

pub(crate) trait Sink<T>: Clone + Send + 'static {
    fn send(&self, item: T) -> bool;
    fn fail(&self, error: String) -> bool;
}

impl<T: SseEncode + Clone + Send + 'static> Sink<T> for StreamSink<T> {
    fn send(&self, item: T) -> bool {
        self.add(item).is_ok()
    }

    fn fail(&self, error: String) -> bool {
        self.add_error(error).is_ok()
    }
}

pub(crate) fn attach<T: Send + 'static, S: Sink<T>>(
    sink: S,
    start: impl FnOnce(Box<dyn FnMut(T) -> bool + Send>) -> Result<(), String>,
) -> Result<(), String> {
    attach_results(sink, |mut deliver| {
        start(Box::new(move |item| deliver(Ok(item))))
    })
}

pub(crate) fn attach_results<T: Send + 'static, S: Sink<T>>(
    sink: S,
    start: impl FnOnce(Box<dyn FnMut(Result<T, String>) -> bool + Send>) -> Result<(), String>,
) -> Result<(), String> {
    let feed = sink.clone();
    if let Err(error) = start(Box::new(move |item| match item {
        Ok(item) => feed.send(item),
        Err(error) => feed.fail(error),
    })) {
        let _ = sink.fail(error);
    }
    Ok(())
}
