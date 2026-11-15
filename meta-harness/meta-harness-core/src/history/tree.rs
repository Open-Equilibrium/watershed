//! The branch tree of a session's stored entries.

use std::collections::HashMap;

use super::entry::{Body, Entry};
use crate::{EntryKind, SessionTree, TreeNode};

/// Builds the tree; label entries drop out of the parent chain.
pub(super) fn session_tree(entries: &[Entry], leaf: Option<&str>) -> SessionTree {
    let mut label_parents: HashMap<&str, Option<&str>> = HashMap::new();
    for entry in entries {
        if matches!(entry.body, Body::Label) {
            label_parents.insert(&entry.id, entry.parent.as_deref());
        }
    }
    let ends = label_ends(&label_parents);
    let nodes = entries
        .iter()
        .filter(|entry| !matches!(entry.body, Body::Label))
        .map(|entry| TreeNode {
            id: entry.id.clone(),
            parent: resolve(&ends, entry.parent.as_deref()),
            kind: entry_kind(&entry.body),
        })
        .collect();
    SessionTree {
        nodes,
        leaf: resolve(&ends, leaf),
    }
}

/// Where each label entry leads: the nearest entry above it that is not a label; labels that
/// lead back to each other lead nowhere. Each label is followed once.
fn label_ends<'a>(
    label_parents: &HashMap<&'a str, Option<&'a str>>,
) -> HashMap<&'a str, Option<&'a str>> {
    let mut ends = HashMap::with_capacity(label_parents.len());
    for &label in label_parents.keys() {
        let mut walked = Vec::new();
        let mut id = Some(label);
        let end = loop {
            let Some(current) = id else { break None };
            // A label walked before leads where it led; one this walk met is still `None`, so a
            // cycle leads nowhere.
            if let Some(&end) = ends.get(current) {
                break end;
            }
            let Some(&parent) = label_parents.get(current) else {
                break Some(current);
            };
            ends.insert(current, None);
            walked.push(current);
            id = parent;
        };
        for label in walked {
            ends.insert(label, end);
        }
    }
    ends
}

/// The entry `id` names once labels drop out of the parent chain.
fn resolve(ends: &HashMap<&str, Option<&str>>, id: Option<&str>) -> Option<String> {
    let id = id?;
    ends.get(id).copied().unwrap_or(Some(id)).map(str::to_owned)
}

fn entry_kind(body: &Body) -> EntryKind {
    match body {
        Body::User { .. } => EntryKind::User,
        Body::Assistant { .. } => EntryKind::Assistant,
        Body::ToolResult { .. } => EntryKind::ToolResult,
        Body::System => EntryKind::System,
        Body::ToolTiming { .. }
        | Body::RequestTiming { .. }
        | Body::Checkpoint { .. }
        | Body::Custom => EntryKind::Custom,
        Body::Label | Body::Other => EntryKind::Other,
    }
}
