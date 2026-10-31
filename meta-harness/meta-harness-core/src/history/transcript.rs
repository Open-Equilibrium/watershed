//! The chat of a session's current branch, rebuilt from its stored entries.

use std::collections::HashMap;

use super::entry::{Body, Entry, OpenCalls};
use super::trajectory::path;
use super::{Transcript, TranscriptItem};
use crate::pi::{call_target, item_kind};
use crate::{ChangedLines, ItemKind};

/// Whether a tool call failed, how long it ran and the lines it changed.
type Ended = (bool, Option<u64>, Option<ChangedLines>);

/// The items of up to `limit` entries on the branch ending at `leaf`, before `before`.
pub(super) fn transcript(
    entries: &[Entry],
    leaf: Option<&str>,
    before: Option<&str>,
    limit: usize,
) -> Transcript {
    let path = path(entries, leaf);
    // How each tool call ended, by its response and place there, from anywhere on the branch,
    // so every page shows it.
    let mut ended: HashMap<(&str, usize), Ended> = HashMap::new();
    let mut open = OpenCalls::default();
    let mut response = "";
    for entry in &path {
        match &entry.body {
            Body::Assistant { calls, .. } => {
                open.respond(calls);
                response = &entry.id;
            }
            Body::ToolTiming { call, duration_ms } => open.ended(call, *duration_ms),
            Body::ToolResult {
                call,
                failed,
                outcome,
                ..
            } => {
                let lines = outcome
                    .added
                    .zip(outcome.removed)
                    .map(|(added, removed)| ChangedLines { added, removed });
                let duration_ms = open.timing(call).flatten();
                if let Some((place, _)) = open.answer(call) {
                    ended.insert((response, place), (*failed, duration_ms, lines));
                }
            }
            _ => {}
        }
    }
    let end = before
        .and_then(|before| path.iter().position(|entry| entry.id == before))
        .unwrap_or(path.len());
    let start = end.saturating_sub(limit);
    let mut items = Vec::new();
    for entry in &path[start..end] {
        let item = |kind, text: &str| TranscriptItem {
            entry: entry.id.clone(),
            kind,
            text: text.to_owned(),
            tool: None,
            target: None,
            failed: false,
            duration_ms: None,
            lines: None,
        };
        match &entry.body {
            Body::User { text, .. } => items.push(item(ItemKind::UserMessage, text)),
            Body::Assistant {
                text,
                thinking,
                calls,
                ..
            } => {
                if *thinking {
                    items.push(item(ItemKind::Reasoning, ""));
                }
                if !text.trim().is_empty() {
                    items.push(item(ItemKind::AssistantMessage, text));
                }
                for (place, call) in calls.iter().enumerate() {
                    let (failed, duration_ms, lines) = ended
                        .get(&(entry.id.as_str(), place))
                        .copied()
                        // A call without a result never finished, such as after a crash.
                        .unwrap_or((true, None, None));
                    // A written file counts the lines it wrote.
                    let written = call
                        .written_lines
                        .filter(|_| !failed)
                        .map(|added| ChangedLines { added, removed: 0 });
                    items.push(TranscriptItem {
                        tool: Some(call.name.clone()),
                        target: Some(call_target(&call.arguments)),
                        failed,
                        duration_ms,
                        lines: lines.or(written),
                        ..item(item_kind(&call.name), "")
                    });
                }
            }
            _ => {}
        }
    }
    Transcript {
        items,
        more: start > 0,
        start: path[start..end].first().map(|entry| entry.id.clone()),
    }
}
