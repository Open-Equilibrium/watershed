//! The branch tree of a session's stored entries.

use std::collections::HashMap;

use super::entry::{Body, Entry, first_line};
use crate::{EntryKind, SessionTree, TreeNode};

/// Longest preview kept for a tree entry, in characters.
const PREVIEW_CHARS: usize = 80;

/// Builds the tree; label entries become node labels and drop out of the parent chain.
pub(super) fn session_tree(entries: &[Entry], leaf: Option<&str>) -> SessionTree {
    let mut label_parents: HashMap<&str, Option<&str>> = HashMap::new();
    let mut labels: HashMap<&str, Option<&str>> = HashMap::new();
    for entry in entries {
        if let Body::Label { target, label } = &entry.body {
            label_parents.insert(&entry.id, entry.parent.as_deref());
            if let Some(target) = target {
                labels.insert(target, label.as_deref());
            }
        }
    }
    let nodes = entries
        .iter()
        .filter(|entry| !matches!(entry.body, Body::Label { .. }))
        .map(|entry| {
            let (kind, preview) = summary(&entry.body);
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
            }
        })
        .collect();
    SessionTree {
        nodes,
        leaf: resolve(&label_parents, leaf),
    }
}

/// Follows label entries up to the nearest entry that is not a label.
fn resolve<'a>(
    label_parents: &HashMap<&'a str, Option<&'a str>>,
    mut id: Option<&'a str>,
) -> Option<String> {
    while let Some(parent) = id.and_then(|id| label_parents.get(id)) {
        id = *parent;
    }
    id.map(str::to_owned)
}

fn summary(body: &Body) -> (EntryKind, String) {
    let line = |text: &str| first_line(text, PREVIEW_CHARS);
    match body {
        Body::User { text } => (EntryKind::User, line(text)),
        Body::Assistant { text, .. } => (EntryKind::Assistant, line(text)),
        Body::ToolResult { tool, .. } => (EntryKind::ToolResult, line(tool)),
        Body::System(_) => (EntryKind::System, String::new()),
        Body::Approval { tool, .. } => (EntryKind::Approval, line(tool)),
        Body::RequestTiming { .. } | Body::ToolTiming { .. } => (
            EntryKind::Custom("watershed.timing".to_owned()),
            String::new(),
        ),
        Body::Custom { kind } => (EntryKind::Custom(kind.clone()), String::new()),
        Body::Message { .. } => (EntryKind::Other("message".to_owned()), String::new()),
        Body::CustomMessage { .. } => {
            (EntryKind::Other("custom_message".to_owned()), String::new())
        }
        Body::BranchSummary => (EntryKind::Other("branch_summary".to_owned()), String::new()),
        Body::Compaction { .. } => (EntryKind::Other("compaction".to_owned()), String::new()),
        Body::ContextEdit { .. } => (EntryKind::Other("context_edit".to_owned()), String::new()),
        Body::Label { .. } => (EntryKind::Other("label".to_owned()), String::new()),
        Body::Other { kind } => (EntryKind::Other(kind.clone()), String::new()),
    }
}
