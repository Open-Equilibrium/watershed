use std::collections::HashMap;

use serde_json::Value;

use super::record::{describe, duration, pending, status_of, text_of, turn_of, turns, usage_of};
use super::{ChatItem, Snapshot, State, ThreadEvent};
use crate::event::{AgentEvent, ItemDetail, ItemId, RequestId, TurnId};
use crate::t3::{Change, Projection};

fn item_id(items: &mut HashMap<String, ItemId>, id: &Value) -> ItemId {
    let next = ItemId(items.len() as u64 + 1);
    *items
        .entry(id.as_str().unwrap_or_default().to_owned())
        .or_insert(next)
}

pub(super) fn restore(state: &mut State, snapshot: &Value) {
    state.projection = Projection::from_snapshot(&snapshot["projection"]);
    state.sequence = snapshot["snapshotSequence"].as_u64().unwrap_or_default();
    state.subscribers.clear();
    state.metadata += 1;
}

pub(super) fn apply(state: &mut State, message: &Value) {
    if message["kind"] != "event" {
        return;
    }
    let sequence = message["sequence"].as_u64().unwrap_or_default();
    if sequence <= state.sequence {
        return;
    }
    state.sequence = sequence;
    let event = &message["event"];
    let change = state.projection.apply(event);
    if matches!(change, Change::Thread) {
        state.metadata += 1;
    }
    if matches!(change, Change::Run(_) | Change::Thread | Change::Other) {
        state.wake_lineage();
    }
    let events = translate(state, change, &event["payload"]);
    state.subscribers.retain(|subscriber| {
        events.iter().all(|event| {
            subscriber
                .try_send(ThreadEvent {
                    sequence,
                    event: event.clone(),
                })
                .is_ok()
        })
    });
}

fn translate(state: &mut State, change: Change, payload: &Value) -> Vec<AgentEvent> {
    match change {
        Change::Run(before) => {
            let turn = TurnId(payload["ordinal"].as_u64().unwrap_or_default());
            let was = before.as_ref().and_then(|run| run["status"].as_str());
            let status = payload["status"].as_str();
            if was == status {
                return Vec::new();
            }
            match status {
                Some("starting" | "running")
                    if !matches!(was, Some("starting" | "running" | "waiting")) =>
                {
                    vec![AgentEvent::TurnStarted { turn }]
                }
                Some("completed") => vec![AgentEvent::TurnCompleted {
                    turn,
                    checkpoint: Some(turn.0.to_string()),
                }],
                Some("interrupted" | "failed" | "cancelled") => vec![AgentEvent::TurnAborted {
                    turn,
                    checkpoint: Some(turn.0.to_string()),
                }],
                Some("rolled_back") => vec![AgentEvent::RolledBack],
                _ => vec![AgentEvent::TurnsChanged],
            }
        }
        Change::TurnItem(before) => {
            if !state.projection.visible(payload) {
                return Vec::new();
            }
            if let Some(request) = pending(&state.projection, payload)
                && before.is_none()
            {
                return vec![AgentEvent::RequestOpened { request }];
            }
            let Some((kind, detail)) = describe(payload) else {
                return Vec::new();
            };
            let turn = turn_of(&state.projection, payload);
            let item = item_id(&mut state.items, &payload["id"]);
            let mut events = Vec::new();
            match before.as_ref().and_then(describe) {
                None => events.push(AgentEvent::ItemStarted {
                    turn,
                    item,
                    kind,
                    detail,
                }),
                Some((_, old)) if old != detail && matches!(detail, ItemDetail::Tasks { .. }) => {
                    events.push(AgentEvent::ItemUpdated { item, detail });
                }
                Some(_) => {}
            }
            let old = before.as_ref().map_or("", |b| text_of(b));
            let new = text_of(payload);
            if let Some(delta) = new.strip_prefix(old).filter(|d| !d.is_empty()) {
                events.push(AgentEvent::ContentDelta {
                    item,
                    delta: delta.to_owned(),
                });
            }
            let ended =
                |item: &Value| matches!(item["status"].as_str(), Some("completed" | "failed"));
            if ended(payload) && !before.as_ref().is_some_and(ended) {
                events.push(AgentEvent::ItemCompleted {
                    turn,
                    item,
                    kind,
                    text: new.to_owned(),
                    status: status_of(payload),
                    duration_ms: duration(payload),
                });
            }
            events
        }
        Change::ProviderTurn => match usage_of(payload) {
            Some(usage) if payload["status"] == "completed" => {
                vec![AgentEvent::ModelRequestCompleted {
                    turn: turn_of(&state.projection, payload),
                    usage,
                }]
            }
            _ => Vec::new(),
        },
        Change::Checkpoint => vec![AgentEvent::TurnsChanged],
        Change::RuntimeRequest(was_pending) => {
            if was_pending && payload["status"] != "pending" {
                vec![AgentEvent::RequestResolved {
                    request: RequestId(payload["id"].as_str().unwrap_or_default().to_owned()),
                }]
            } else {
                Vec::new()
            }
        }
        Change::Message(true) => vec![AgentEvent::TurnsChanged],
        Change::Thread | Change::Message(false) | Change::Other => Vec::new(),
    }
}

pub(super) fn snapshot(state: &mut State) -> Snapshot {
    let sequence = state.sequence;
    let State {
        projection, items, ..
    } = state;
    let transcript = projection
        .inherited
        .iter()
        .chain(
            projection
                .turn_items
                .iter()
                .filter(|item| projection.visible(item)),
        )
        .filter_map(|item| {
            let (kind, detail) = describe(item)?;
            Some(ChatItem {
                id: item_id(items, &item["id"]),
                turn: turn_of(projection, item),
                kind,
                detail,
                text: text_of(item).to_owned(),
                status: matches!(item["status"].as_str(), Some("completed" | "failed"))
                    .then(|| status_of(item)),
                duration_ms: duration(item),
            })
        })
        .collect();
    Snapshot {
        sequence,
        turns: turns(projection),
        transcript,
        requests: projection
            .turn_items
            .iter()
            .filter_map(|item| pending(projection, item))
            .collect(),
    }
}
