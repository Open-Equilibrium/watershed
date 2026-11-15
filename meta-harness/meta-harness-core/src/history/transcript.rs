//! The chat of a session's current branch, rebuilt from its stored entries.

use std::collections::HashMap;

use super::entry::{Body, Entry, OpenCalls, path};
use super::{Transcript, TranscriptItem};
use crate::pi::item_kind;
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
    let end = before
        .and_then(|before| path.iter().position(|entry| entry.id == before))
        .unwrap_or(path.len());
    let start = end.saturating_sub(limit);
    // How each tool call on the page ended, by its response and place there. A result follows its
    // call, so the scan from the page's start stops once every call on the page has one.
    let mut waiting: usize = path[start..end]
        .iter()
        .map(|entry| match &entry.body {
            Body::Assistant { calls, .. } => calls.len(),
            _ => 0,
        })
        .sum();
    let mut ended: HashMap<(&str, usize), Ended> = HashMap::new();
    let mut open = OpenCalls::default();
    let mut response = "";
    for (index, entry) in path.iter().enumerate().skip(start) {
        if waiting == 0 {
            break;
        }
        match &entry.body {
            Body::Assistant { calls, .. } => {
                // A response starts over the open calls, so none of the page's calls can be
                // answered from its start on, even when some never got a result.
                if index >= end {
                    break;
                }
                open.respond(calls);
                response = &entry.id;
            }
            Body::ToolTiming { call, duration_ms } => open.ended(call, *duration_ms),
            Body::ToolResult {
                call,
                failed,
                lines,
                ..
            } => {
                let duration_ms = open.timing(call).flatten();
                // The open calls belong to the latest response, which is on the page.
                if let Some((place, _)) = open.answer(call) {
                    waiting = waiting.saturating_sub(1);
                    ended.insert((response, place), (*failed, duration_ms, *lines));
                }
            }
            _ => {}
        }
    }
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
                        target: Some(call.target.clone()),
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
