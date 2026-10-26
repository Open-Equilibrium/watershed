//! The chat of a session's current branch, rebuilt from its stored entries.

use std::collections::HashMap;

use super::entry::{Body, Entry};
use super::trajectory::path;
use super::{Transcript, TranscriptItem};
use crate::pi::call_target;
use crate::{ChangedLines, ItemKind};

/// The items of up to `limit` entries on the branch ending at `leaf`, before `before`.
pub(super) fn transcript(
    entries: &[Entry],
    leaf: Option<&str>,
    before: Option<&str>,
    limit: usize,
) -> Transcript {
    let path = path(entries, leaf);
    // How each tool call ended, from anywhere on the branch, so every page shows it.
    let mut ended: HashMap<&str, (bool, Option<u64>, Option<ChangedLines>)> = HashMap::new();
    let mut timing: HashMap<&str, Option<u64>> = HashMap::new();
    for entry in &path {
        match &entry.body {
            Body::ToolTiming { call, duration_ms } => {
                timing.insert(call, *duration_ms);
            }
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
                ended.insert(
                    call,
                    (*failed, timing.get(call.as_str()).copied().flatten(), lines),
                );
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
                for call in calls {
                    let (failed, duration_ms, lines) =
                        // A call without a result never finished, such as after a crash.
                        ended.get(call.id.as_str()).copied().unwrap_or((true, None, None));
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
                        ..item(tool_kind(&call.name), "")
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

/// The item kind of one of pi's tools, as the live chat shows it.
fn tool_kind(tool: &str) -> ItemKind {
    match tool {
        "bash" => ItemKind::CommandExecution,
        "edit" | "write" => ItemKind::FileChange,
        _ => ItemKind::DynamicToolCall,
    }
}
