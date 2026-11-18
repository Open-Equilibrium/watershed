use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use meta_harness_core::{
    AgentEvent, ItemDetail, ItemId, ItemKind, ItemStatus, ThreadEvent, TurnId,
};

use crate::api::events::{BridgeEvent, BridgeEventKind};
use crate::stream::{Credit, Outbox, Sink, WINDOW, attach, attach_results, forward, gather, push};

fn ev(event: AgentEvent) -> ThreadEvent {
    ThreadEvent { sequence: 1, event }
}

fn never() -> tokio::sync::mpsc::Receiver<ThreadEvent> {
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    std::mem::forget(sender);
    receiver
}

fn closed() -> tokio::sync::mpsc::Receiver<ThreadEvent> {
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    drop(sender);
    receiver
}

fn started(item: u64) -> ThreadEvent {
    ev(AgentEvent::ItemStarted {
        turn: TurnId(1),
        item: ItemId(item),
        kind: ItemKind::CommandExecution,
        detail: ItemDetail::None,
    })
}

fn finished(item: u64, text: &str) -> ThreadEvent {
    ev(AgentEvent::ItemCompleted {
        turn: TurnId(1),
        item: ItemId(item),
        kind: ItemKind::CommandExecution,
        text: text.to_owned(),
        status: ItemStatus::Completed,
        duration_ms: None,
    })
}

fn delta(item: u64, delta: &str) -> ThreadEvent {
    ev(AgentEvent::ContentDelta {
        item: ItemId(item),
        delta: delta.to_owned(),
    })
}

fn delta_at(sequence: u64, text: &str) -> BridgeEvent {
    BridgeEvent::from(ThreadEvent {
        sequence,
        event: AgentEvent::ContentDelta {
            item: ItemId(1),
            delta: text.to_owned(),
        },
    })
}

fn pending_text(outbox: &Outbox) -> Vec<String> {
    outbox
        .pending
        .iter()
        .filter_map(|event| event.text.clone())
        .collect()
}

fn start_forwarding(
    credit: Arc<Credit>,
) -> (
    tokio::sync::mpsc::Sender<ThreadEvent>,
    tokio::sync::mpsc::UnboundedReceiver<BridgeEvent>,
) {
    let (events, receiver) = tokio::sync::mpsc::channel(4096);
    let (delivered, deliveries) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(forward(
        receiver,
        || std::future::ready(Some(never())),
        credit,
        move |event| delivered.send(event).is_ok(),
    ));
    (events, deliveries)
}

fn taken(deliveries: &mut tokio::sync::mpsc::UnboundedReceiver<BridgeEvent>) -> Vec<BridgeEvent> {
    std::iter::from_fn(|| deliveries.try_recv().ok()).collect()
}

async fn settle() {
    tokio::time::sleep(Duration::from_millis(100)).await;
}

async fn within<T>(next: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(1), next)
        .await
        .expect("delivered within a second")
}

fn paused_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime starts")
}

#[test]
fn gathers_streaming_text_until_its_window_passes() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime starts");
    runtime.block_on(async {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(16);
        let (pass, window) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            for index in 1..10 {
                sender
                    .send(delta(1, &index.to_string()))
                    .await
                    .expect("sent");
                tokio::task::yield_now().await;
            }
            pass.send(()).expect("window passes");
            tokio::task::yield_now().await;
            sender.send(delta(1, "late")).await.expect("sent");
        });
        let mut outbox = Outbox::default();
        let credit = Credit::default();
        outbox.admit(BridgeEvent::from(delta(1, "0")), &credit);
        gather(&mut outbox, &mut receiver, &credit, async {
            let _ = window.await;
        })
        .await;
        let text: Vec<_> = outbox
            .pending
            .iter()
            .filter_map(|event| event.text.as_deref())
            .collect();
        assert_eq!(
            text.concat(),
            "0123456789",
            "everything until the window passed"
        );
        assert_eq!(
            outbox.pending.len(),
            1,
            "deltas merge as they arrive, so the batch stays small"
        );
    });

    runtime.block_on(async {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(16);
        sender
            .send(ev(AgentEvent::TurnCompleted {
                turn: TurnId(1),
                checkpoint: None,
            }))
            .await
            .expect("sent");
        let mut outbox = Outbox::default();
        let credit = Credit::default();
        outbox.admit(BridgeEvent::from(delta(1, "0")), &credit);
        gather(&mut outbox, &mut receiver, &credit, std::future::pending()).await;
        assert_eq!(outbox.pending.len(), 2);
    });
}

#[test]
fn forwards_streaming_text_once_its_window_passed() {
    let ms = Duration::from_millis;
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime starts")
        .block_on(async {
            let (events, receiver) = tokio::sync::mpsc::channel(16);
            let (delivered, mut deliveries) = tokio::sync::mpsc::unbounded_channel();
            let start = tokio::time::Instant::now();
            tokio::spawn(forward(
                receiver,
                || std::future::ready(Some(never())),
                Arc::default(),
                move |event| {
                    delivered
                        .send((start.elapsed(), event.kind, event.text))
                        .is_ok()
                },
            ));
            let streamed =
                |at, text: &str| Some((at, BridgeEventKind::ContentDelta, Some(text.to_owned())));
            events.send(delta(1, "Hel")).await.expect("sent");
            tokio::time::sleep(ms(30)).await;
            events.send(delta(1, "lo")).await.expect("sent");
            assert_eq!(within(deliveries.recv()).await, streamed(ms(50), "Hello"));
            tokio::time::sleep(ms(10)).await;
            events.send(delta(1, "!")).await.expect("sent");
            let completed = AgentEvent::TurnCompleted {
                turn: TurnId(1),
                checkpoint: None,
            };
            events.send(ev(completed)).await.expect("sent");
            assert_eq!(within(deliveries.recv()).await, streamed(ms(60), "!"));
            assert_eq!(
                within(deliveries.recv()).await,
                Some((ms(60), BridgeEventKind::TurnCompleted, None))
            );
        });
}

#[test]
fn merges_consecutive_deltas_of_one_item() {
    let delta = |item: u64, text: &str| {
        BridgeEvent::from(ev(AgentEvent::ContentDelta {
            item: ItemId(item),
            delta: text.to_owned(),
        }))
    };
    let mut merged = Vec::new();
    for event in [
        delta(1, "He"),
        delta(1, "llo"),
        delta(2, "Hi"),
        BridgeEvent::from(ev(AgentEvent::TurnCompleted {
            turn: TurnId(1),
            checkpoint: None,
        })),
        delta(2, "!"),
    ] {
        push(&mut merged, event, Some(0));
    }
    let summary: Vec<_> = merged
        .iter()
        .map(|event| {
            (
                event.kind,
                event.item.as_ref().map(|item| item.id),
                event.text.clone(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (
                BridgeEventKind::ContentDelta,
                Some(1),
                Some("Hello".to_owned())
            ),
            (
                BridgeEventKind::ContentDelta,
                Some(2),
                Some("Hi".to_owned())
            ),
            (BridgeEventKind::TurnCompleted, None, None),
            (BridgeEventKind::ContentDelta, Some(2), Some("!".to_owned())),
        ]
    );
}

#[test]
fn deltas_on_either_side_of_a_recovery_snapshot_stay_apart() {
    let credit = Credit::default();
    let mut outbox = Outbox::default();
    outbox.admit(delta_at(1, "A"), &credit);
    credit.begin_snapshot();
    credit.end_snapshot(Some(1));
    outbox.admit(delta_at(2, "B"), &credit);
    assert_eq!(pending_text(&outbox), ["A", "B"]);
}

#[test]
fn deltas_taken_while_a_recovery_snapshot_is_taken_stay_apart() {
    let credit = Credit::default();
    let mut outbox = Outbox::default();
    credit.begin_snapshot();
    outbox.admit(delta_at(1, "A"), &credit);
    outbox.admit(delta_at(2, "B"), &credit);
    credit.end_snapshot(Some(2));
    assert_eq!(pending_text(&outbox), ["A", "B"]);
}

#[test]
fn closing_the_credit_releases_every_forwarder_that_shares_it() {
    paused_runtime().block_on(async {
        let credit = Arc::new(Credit::default());
        let probe = Arc::new(());
        let (events, pending) = tokio::sync::mpsc::channel(4096);
        let (_idle, idle) = tokio::sync::mpsc::channel(16);
        let mut forwarding = Vec::new();
        for receiver in [pending, idle] {
            let held = probe.clone();
            forwarding.push(tokio::spawn(forward(
                receiver,
                || std::future::ready(Some(never())),
                credit.clone(),
                move |_event| {
                    let _held = &held;
                    true
                },
            )));
        }
        for item in 1..=WINDOW as u64 + 1 {
            events.send(started(item)).await.expect("sent");
        }
        settle().await;
        credit.close();
        drop(events);
        for forwarding in forwarding {
            tokio::time::timeout(Duration::from_secs(1), forwarding)
                .await
                .expect("each forwarder returns once the session closed")
                .expect("forward completes");
        }
        assert_eq!(
            Arc::strong_count(&probe),
            1,
            "every forwarder released its sink and its pending events"
        );
    });
}

#[test]
fn closing_the_credit_ends_a_resubscription_that_is_still_pending() {
    paused_runtime().block_on(async {
        let credit = Arc::new(Credit::default());
        let probe = Arc::new(());
        let held = probe.clone();
        let (entered, mut reached) = tokio::sync::mpsc::unbounded_channel();
        let (sender, receiver) = tokio::sync::mpsc::channel(16);
        let forwarding = tokio::spawn(forward(
            receiver,
            move || {
                let _ = entered.send(());
                let held = held.clone();
                async move {
                    let _held = held;
                    std::future::pending::<Option<tokio::sync::mpsc::Receiver<ThreadEvent>>>().await
                }
            },
            credit.clone(),
            |_event| true,
        ));
        drop(sender);
        within(reached.recv())
            .await
            .expect("the resubscription starts");
        credit.confirm(0);
        settle().await;
        assert!(
            !forwarding.is_finished(),
            "the resubscription is still pending"
        );
        credit.close();
        tokio::time::timeout(Duration::from_secs(1), forwarding)
            .await
            .expect("forward returns once the session closed, while the resubscription waits")
            .expect("forward completes");
        assert_eq!(
            Arc::strong_count(&probe),
            1,
            "the pending resubscription was released"
        );
    });
}

#[test]
fn what_waits_goes_out_while_a_resubscription_is_pending() {
    paused_runtime().block_on(async {
        let credit = Arc::new(Credit::default());
        let (events, receiver) = tokio::sync::mpsc::channel(4096);
        let (delivered, mut deliveries) = tokio::sync::mpsc::unbounded_channel();
        let forwarding = tokio::spawn(forward(
            receiver,
            std::future::pending::<Option<tokio::sync::mpsc::Receiver<ThreadEvent>>>,
            credit.clone(),
            move |event| delivered.send(event).is_ok(),
        ));
        for item in 1..=WINDOW as u64 + 1 {
            events.send(started(item)).await.expect("sent");
        }
        drop(events);
        settle().await;
        assert_eq!(taken(&mut deliveries).len(), WINDOW);
        credit.confirm(WINDOW as u64);
        settle().await;
        assert_eq!(
            taken(&mut deliveries).len(),
            1,
            "what waits is sent while the resubscription is pending"
        );
        credit.close();
        tokio::time::timeout(Duration::from_secs(1), forwarding)
            .await
            .expect("forward returns once the session closed")
            .expect("forward completes");
    });
}

#[test]
fn falling_behind_keeps_finished_items_and_sends_one_fell_behind_event() {
    paused_runtime().block_on(async {
        let credit = Arc::new(Credit::default());
        let (events, mut deliveries) = start_forwarding(credit.clone());
        for item in 1..=WINDOW as u64 {
            events.send(started(item)).await.expect("sent");
        }
        for item in 1000..1100 {
            events.send(finished(item, "done")).await.expect("sent");
        }
        for item in 3000..3200 {
            events.send(delta(item, "partial")).await.expect("sent");
        }
        for item in 5000..5010 {
            events.send(started(item)).await.expect("sent");
        }
        settle().await;
        assert_eq!(taken(&mut deliveries).len(), WINDOW);
        credit.confirm(WINDOW as u64);
        settle().await;
        let sent: Vec<_> = taken(&mut deliveries)
            .iter()
            .map(|event| (event.kind, event.item.as_ref().map(|item| item.id)))
            .collect();
        let finished_items =
            (1000i64..1100).map(|item| (BridgeEventKind::ItemCompleted, Some(item)));
        let live_after = (3157i64..3200).map(|item| (BridgeEventKind::ContentDelta, Some(item)));
        let started_after = (5000i64..5010).map(|item| (BridgeEventKind::ItemStarted, Some(item)));
        let expected: Vec<_> = finished_items
            .chain([(BridgeEventKind::FellBehind, None)])
            .chain(live_after)
            .chain(started_after)
            .collect();
        assert_eq!(
            sent, expected,
            "the finished items stay in order, one fell-behind event follows the partial output that went, and what comes after is live"
        );
    });
}

#[test]
fn events_other_than_partial_output_wait_for_liquid_and_are_never_dropped() {
    paused_runtime().block_on(async {
        let credit = Arc::new(Credit::default());
        let (events, mut deliveries) = start_forwarding(credit.clone());
        for item in 1..=1000 {
            events.send(started(item)).await.expect("sent");
        }
        settle().await;
        let mut delivered = taken(&mut deliveries);
        assert_eq!(
            delivered.len(),
            WINDOW,
            "the sink never holds more than the window unconfirmed"
        );
        for applied in [256u64, 512, 768] {
            credit.confirm(applied);
            settle().await;
            delivered.extend(taken(&mut deliveries));
        }
        let items: Vec<_> = delivered
            .iter()
            .map(|event| (event.kind, event.item.as_ref().map(|item| item.id)))
            .collect();
        let expected: Vec<_> = (1..=1000i64)
            .map(|item| (BridgeEventKind::ItemStarted, Some(item)))
            .collect();
        assert_eq!(
            items, expected,
            "a queue past its bound waits for confirmations, in order, and drops nothing"
        );
    });
}

#[test]
fn a_second_overflow_while_the_fell_behind_event_waits_adds_no_second_one() {
    paused_runtime().block_on(async {
        let credit = Arc::new(Credit::default());
        let (events, mut deliveries) = start_forwarding(credit.clone());
        for item in 1..=WINDOW as u64 {
            events.send(started(item)).await.expect("sent");
        }
        for item in 3000..3700 {
            events.send(delta(item, "partial")).await.expect("sent");
        }
        settle().await;
        let mut delivered = taken(&mut deliveries);
        for applied in [512u64, 768, 1024, 1280] {
            credit.confirm(applied);
            settle().await;
            delivered.extend(taken(&mut deliveries));
        }
        let fell_behind = delivered
            .iter()
            .filter(|event| event.kind == BridgeEventKind::FellBehind)
            .count();
        assert_eq!(fell_behind, 1, "one fell-behind event per stall");
    });
}

#[test]
fn a_dropped_subscription_falls_behind_once_and_continues_on_a_new_one() {
    paused_runtime().block_on(async {
        let (events, receiver) = tokio::sync::mpsc::channel(16);
        let (again, resubscribed) = tokio::sync::mpsc::channel(16);
        let mut resubscribed = Some(resubscribed);
        let (delivered, mut deliveries) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(forward(
            receiver,
            move || std::future::ready(Some(resubscribed.take().expect("one resubscription"))),
            Arc::default(),
            move |event| delivered.send(event).is_ok(),
        ));
        events.send(started(1)).await.expect("sent");
        drop(events);
        again.send(started(2)).await.expect("sent");
        settle().await;
        let kinds: Vec<_> = taken(&mut deliveries)
            .iter()
            .map(|event| (event.kind, event.item.as_ref().map(|item| item.id)))
            .collect();
        assert_eq!(
            kinds,
            [
                (BridgeEventKind::ItemStarted, Some(1)),
                (BridgeEventKind::FellBehind, None),
                (BridgeEventKind::ItemStarted, Some(2)),
            ]
        );
    });
}

#[test]
fn a_thread_that_ended_ends_its_stream_without_a_reload() {
    paused_runtime().block_on(async {
        let reloads = Arc::new(AtomicUsize::new(0));
        let delivered = reloads.clone();
        let forwarding = tokio::spawn(forward(
            closed(),
            || std::future::ready(None),
            Arc::<Credit>::default(),
            move |event| {
                if event.kind == BridgeEventKind::FellBehind {
                    delivered.fetch_add(1, Ordering::SeqCst);
                }
                true
            },
        ));
        tokio::time::timeout(Duration::from_secs(1), forwarding)
            .await
            .expect("forward returns once the thread ended")
            .expect("forward completes");
        assert_eq!(reloads.load(Ordering::SeqCst), 0);
    });
}

#[test]
fn what_waits_when_the_thread_ended_is_sent_once_liquid_confirms() {
    paused_runtime().block_on(async {
        let credit = Arc::new(Credit::default());
        let (events, receiver) = tokio::sync::mpsc::channel(4096);
        let (delivered, mut deliveries) = tokio::sync::mpsc::unbounded_channel();
        let forwarding = tokio::spawn(forward(
            receiver,
            || std::future::ready(None),
            credit.clone(),
            move |event| delivered.send(event).is_ok(),
        ));
        for item in 1..=WINDOW as u64 + 1 {
            events.send(started(item)).await.expect("sent");
        }
        drop(events);
        settle().await;
        assert!(
            !forwarding.is_finished(),
            "the forwarder waits for Liquid while an event waits"
        );
        assert_eq!(taken(&mut deliveries).len(), WINDOW);
        credit.confirm(WINDOW as u64);
        tokio::time::timeout(Duration::from_secs(1), forwarding)
            .await
            .expect("forward ends once what waits is sent")
            .expect("forward completes");
        assert_eq!(taken(&mut deliveries).len(), 1, "the waiting event is sent");
    });
}

#[test]
fn reconnect_retries_a_lost_connection_and_returns_any_other_error_at_once() {
    paused_runtime().block_on(async {
        let (_sender, mut ended) = tokio::sync::watch::channel(());
        let lost = "Meta-Harness is not running: nothing answers at ws://127.0.0.1:1.";
        let mut replies = vec![Ok(7), Err(lost.to_owned()), Err(lost.to_owned())];
        let answer = crate::stream::reconnect(&mut ended, || {
            std::future::ready(replies.pop().expect("no attempt after the answer"))
        })
        .await;
        assert_eq!(answer, Some(Ok(7)));
        assert!(replies.is_empty(), "every lost connection was retried");

        let mut replies = vec![Err::<u8, _>("This session no longer exists.".to_owned())];
        let answer = crate::stream::reconnect(&mut ended, || {
            std::future::ready(replies.pop().expect("no attempt after the error"))
        })
        .await;
        assert_eq!(
            answer,
            Some(Err("This session no longer exists.".to_owned()))
        );
    });
}

#[test]
fn an_idle_thread_stays_live_across_two_restores_without_an_event_between() {
    paused_runtime().block_on(async {
        let (live, receiver) = tokio::sync::mpsc::channel(16);
        let mut next = vec![receiver, closed()];
        let (delivered, mut deliveries) = tokio::sync::mpsc::unbounded_channel();
        let forwarding = tokio::spawn(forward(
            closed(),
            move || std::future::ready(next.pop()),
            Arc::default(),
            move |event| delivered.send(event).is_ok(),
        ));
        live.send(started(1)).await.expect("sent");
        settle().await;
        assert!(!forwarding.is_finished(), "the stream stays live");
        assert!(
            taken(&mut deliveries)
                .iter()
                .any(|event| event.kind == BridgeEventKind::ItemStarted),
            "the event after the restores arrives"
        );
    });
}

#[derive(Clone, Default)]
struct Recorder {
    items: Arc<std::sync::Mutex<Vec<i64>>>,
    errors: Arc<std::sync::Mutex<Vec<String>>>,
}

impl Sink<i64> for Recorder {
    fn send(&self, item: i64) -> bool {
        self.items.lock().unwrap().push(item);
        true
    }

    fn fail(&self, error: String) -> bool {
        self.errors.lock().unwrap().push(error);
        true
    }
}

#[test]
fn a_stream_that_cannot_start_ends_with_its_refusal_as_its_error() {
    let sink = Recorder::default();
    let started = attach(sink.clone(), |mut deliver| {
        deliver(1);
        Ok(())
    });
    assert_eq!(started, Ok(()));
    let refused = attach(sink.clone(), |_| {
        Err("This subscription is not open.".to_owned())
    });
    assert_eq!(refused, Ok(()));
    assert_eq!(*sink.items.lock().unwrap(), [1]);
    assert_eq!(
        *sink.errors.lock().unwrap(),
        ["This subscription is not open."]
    );
}

#[test]
fn a_stream_of_results_sends_its_failed_items_and_its_refusal_as_errors() {
    let sink = Recorder::default();
    let started = attach_results(sink.clone(), |mut deliver| {
        assert!(
            deliver(Err("a failed read".to_owned())),
            "Dart still listens"
        );
        assert!(deliver(Ok(2)));
        Ok(())
    });
    assert_eq!(started, Ok(()));
    let refused = attach_results(sink.clone(), |_| {
        Err("Subscribe to the lineage before streaming it.".to_owned())
    });
    assert_eq!(refused, Ok(()));
    assert_eq!(*sink.items.lock().unwrap(), [2]);
    assert_eq!(
        *sink.errors.lock().unwrap(),
        [
            "a failed read",
            "Subscribe to the lineage before streaming it."
        ]
    );
}
