//! The branch tree of a session's stored entries.

use std::collections::HashMap;

use super::entry::{Body, Entry, first_line};
use crate::{EntryKind, SessionTree, TreeNode, pi};

/// Longest preview kept for a tree entry, in characters.
const PREVIEW_CHARS: usize = 80;

/// Builds the tree; label entries become node labels and drop out of the parent chain.
pub(super) fn session_tree(entries: &[Entry], leaf: Option<&str>) -> SessionTree {
    let mut label_parents: HashMap<&str, Option<&str>> = HashMap::new();
    let mut labels: HashMap<&str, Option<&str>> = HashMap::new();
    // What each tool call works on, by call id, for its result's preview.
    let mut targets: HashMap<&str, String> = HashMap::new();
    for entry in entries {
        match &entry.body {
            Body::Label { target, label } => {
                label_parents.insert(&entry.id, entry.parent.as_deref());
                if let Some(target) = target {
                    labels.insert(target, label.as_deref());
                }
            }
            Body::Assistant { calls, .. } => {
                for call in calls {
                    targets.insert(&call.id, pi::call_target(&call.arguments));
                }
            }
            _ => {}
        }
    }
    let nodes = entries
        .iter()
        .filter(|entry| !matches!(entry.body, Body::Label { .. }))
        .map(|entry| {
            let (kind, preview) = summary(&entry.body, &targets);
            TreeNode {
                id: entry.id.clone(),
                parent: resolve(&label_parents, entry.parent.as_deref()),
                kind,
                preview,
                label: labels
                    .get(entry.id.as_str())
                    .copied()
                    .flatten()
                    .map(str::to_owned),
                at_ms: entry.at_ms,
                failed: match &entry.body {
                    Body::ToolResult { failed, .. } => *failed,
                    Body::Assistant { stop_reason, .. } => stop_reason == "error",
                    _ => false,
                },
            }
        })
        .collect();
    SessionTree {
        nodes,
        leaf: resolve(&label_parents, leaf),
    }
}

/// Follows label entries up to the nearest entry that is not a label; labels that lead back to
/// each other lead nowhere.
fn resolve<'a>(
    label_parents: &HashMap<&'a str, Option<&'a str>>,
    mut id: Option<&'a str>,
) -> Option<String> {
    let mut steps = label_parents.len();
    while let Some(parent) = id.and_then(|id| label_parents.get(id)) {
        if steps == 0 {
            return None;
        }
        steps -= 1;
        id = *parent;
    }
    id.map(str::to_owned)
}

fn summary(body: &Body, targets: &HashMap<&str, String>) -> (EntryKind, String) {
    let line = |text: &str| first_line(text, PREVIEW_CHARS);
    match body {
        Body::User { text, .. } => (EntryKind::User, line(text)),
        Body::Assistant { text, .. } => (EntryKind::Assistant, line(text)),
        Body::ToolResult { tool, call, .. } => {
            let preview = match targets.get(call.as_str()) {
                Some(target) if !target.is_empty() => format!("{tool} {target}"),
                _ => tool.clone(),
            };
            (EntryKind::ToolResult, line(&preview))
        }
        Body::System(_) => (EntryKind::System, String::new()),
        Body::Approval { tool, .. } => (EntryKind::Approval, line(tool)),
        Body::RequestTiming { .. } | Body::ToolTiming { .. } | Body::Custom => {
            (EntryKind::Custom, String::new())
        }
        Body::Message { .. }
        | Body::CustomMessage { .. }
        | Body::BranchSummary { .. }
        | Body::Compaction { .. }
        | Body::ContextEdit { .. }
        | Body::Label { .. }
        | Body::Other => (EntryKind::Other, String::new()),
    }
}
