//! The event forwarder: batching, the window Liquid confirms, and falling behind (ADR-0272,
//! ADR-0290).

use std::sync::Arc;
use std::time::Duration;

use meta_harness_core::{
    AgentEvent, ItemDetail, ItemId, ItemKind, ItemStatus, ThreadEvent, TurnId,
};

use crate::api::events::{BridgeEvent, BridgeEventKind};
use crate::stream::{Credit, Outbox, WINDOW, forward, gather, push};

fn ev(event: AgentEvent) -> ThreadEvent {
    ThreadEvent { sequence: 1, event }
}

/// A resubscription that never yields, for streams whose subscription lasts.
fn never() -> tokio::sync::mpsc::Receiver<ThreadEvent> {
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    std::mem::forget(sender);
    receiver
}

/// A session event that starts item `item`, which no later event merges with.
fn started(item: u64) -> ThreadEvent {
    ev(AgentEvent::ItemStarted {
        turn: TurnId(1),
        item: ItemId(item),
        kind: ItemKind::CommandExecution,
        detail: ItemDetail::None,
    })
}

/// A session event that finishes item `item` with `text`.
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

/// A session event that appends `delta` to item `item`.
fn delta(item: u64, delta: &str) -> ThreadEvent {
    ev(AgentEvent::ContentDelta {
        item: ItemId(item),
        delta: delta.to_owned(),
    })
}

/// Starts forwarding a session's events under `credit`; the events are sent in, and what the
/// sink receives comes out.
fn start_forwarding(
    credit: Arc<Credit>,
) -> (
    tokio::sync::mpsc::Sender<ThreadEvent>,
    tokio::sync::mpsc::UnboundedReceiver<BridgeEvent>,
) {
    let (events, receiver) = tokio::sync::mpsc::channel(4096);
    let (delivered, deliveries) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(forward(receiver, never, credit, move |event| {
        delivered.send(event).is_ok()
    }));
    (events, deliveries)
}

/// What the sink received since the last call, in order.
fn taken(deliveries: &mut tokio::sync::mpsc::UnboundedReceiver<BridgeEvent>) -> Vec<BridgeEvent> {
    std::iter::from_fn(|| deliveries.try_recv().ok()).collect()
}

/// Lets the forwarder run until it waits for an event or for Liquid, past any gathering window.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(100)).await;
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
    let delta = |text: &str| {
        ev(AgentEvent::ContentDelta {
            item: ItemId(1),
            delta: text.to_owned(),
        })
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime starts");
    runtime.block_on(async {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(16);
        let (pass, window) = tokio::sync::oneshot::channel::<()>();
        // Deltas arrive one by one while the window is open; one more comes after it passed.
        tokio::spawn(async move {
            for index in 1..10 {
                sender.send(delta(&index.to_string())).await.expect("sent");
                tokio::task::yield_now().await;
            }
            pass.send(()).expect("window passes");
            tokio::task::yield_now().await;
            sender.send(delta("late")).await.expect("sent");
        });
        let mut outbox = Outbox::default();
        outbox.admit(BridgeEvent::from(delta("0")));
        gather(&mut outbox, &mut receiver, async {
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

    // Anything but streaming text ends the gathering at once, without waiting for the window.
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
        outbox.admit(BridgeEvent::from(delta("0")));
        gather(&mut outbox, &mut receiver, std::future::pending()).await;
        assert_eq!(outbox.pending.len(), 2);
    });
}

#[test]
fn forwards_streaming_text_once_its_window_passed() {
    let delta = |text: &str| {
        ev(AgentEvent::ContentDelta {
            item: ItemId(1),
            delta: text.to_owned(),
        })
    };
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
            tokio::spawn(forward(receiver, never, Arc::default(), move |event| {
                delivered
                    .send((start.elapsed(), event.kind, event.text))
                    .is_ok()
            }));
            let streamed =
                |at, text: &str| Some((at, BridgeEventKind::ContentDelta, Some(text.to_owned())));
            events.send(delta("Hel")).await.expect("sent");
            tokio::time::sleep(ms(30)).await;
            events.send(delta("lo")).await.expect("sent");
            assert_eq!(deliveries.recv().await, streamed(ms(50), "Hello"));
            // Anything else ends the window at once, with the text before it.
            tokio::time::sleep(ms(10)).await;
            events.send(delta("!")).await.expect("sent");
            let completed = AgentEvent::TurnCompleted {
                turn: TurnId(1),
                checkpoint: None,
            };
            events.send(ev(completed)).await.expect("sent");
            assert_eq!(deliveries.recv().await, streamed(ms(60), "!"));
            assert_eq!(
                deliveries.recv().await,
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
        push(&mut merged, event);
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
fn holds_events_beyond_the_window_until_liquid_confirms() {
    paused_runtime().block_on(async {
        let credit = Arc::new(Credit::default());
        let (events, mut deliveries) = start_forwarding(credit.clone());
        for item in 1..=WINDOW as u64 + 44 {
            events.send(started(item)).await.expect("sent");
        }
        settle().await;
        assert_eq!(
            taken(&mut deliveries).len(),
            WINDOW,
            "the sink never holds more than the window unconfirmed"
        );
        credit.confirm(WINDOW as u64);
        settle().await;
        assert_eq!(
            taken(&mut deliveries).len(),
            44,
            "a confirmation reopens the window for what waits"
        );
    });
}

#[test]
fn closing_the_credit_releases_a_forwarder_that_waits_on_a_dropped_session() {
    paused_runtime().block_on(async {
        let credit = Arc::new(Credit::default());
        let probe = Arc::new(());
        let held = probe.clone();
        let (events, receiver) = tokio::sync::mpsc::channel(4096);
        let forwarding = tokio::spawn(forward(receiver, never, credit.clone(), move |_event| {
            let _held = &held;
            true
        }));
        for item in 1..=WINDOW as u64 + 1 {
            events.send(started(item)).await.expect("sent");
        }
        settle().await;
        // Liquid confirms nothing, so the one event past the window waits; dropping the session
        // closes the credit.
        credit.close();
        drop(events);
        tokio::time::timeout(Duration::from_secs(1), forwarding)
            .await
            .expect("forward returns once the session ended and its credit closed")
            .expect("forward completes");
        assert_eq!(
            Arc::strong_count(&probe),
            1,
            "the forwarder released its sink and its pending events"
        );
    });
}

#[test]
fn closing_the_credit_releases_a_forwarder_that_waits_for_the_next_event() {
    paused_runtime().block_on(async {
        let credit = Arc::new(Credit::default());
        let probe = Arc::new(());
        let held = probe.clone();
        // The session's sender stays open, as the core keeps a dropped thread's subscriber until
        // the thread's next event.
        let (_events, receiver) = tokio::sync::mpsc::channel(16);
        let forwarding = tokio::spawn(forward(receiver, never, credit.clone(), move |_event| {
            let _held = &held;
            true
        }));
        settle().await;
        credit.close();
        tokio::time::timeout(Duration::from_secs(1), forwarding)
            .await
            .expect("forward returns once the session closed, without waiting for an event")
            .expect("forward completes");
        assert_eq!(
            Arc::strong_count(&probe),
            1,
            "the forwarder released its sink"
        );
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
        // Liquid applies nothing: finished items queue, then more partial output than the bound
        // allows, and events after that overflow come behind it.
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
        // The partial output queued before the overflow is gone; what came after the fell-behind
        // event is live and stays.
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
        // Two overflows before Liquid confirms anything.
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
            move || resubscribed.take().expect("one resubscription"),
            Arc::default(),
            move |event| delivered.send(event).is_ok(),
        ));
        events.send(started(1)).await.expect("sent");
        // The core drops a subscriber that fell too far behind.
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
