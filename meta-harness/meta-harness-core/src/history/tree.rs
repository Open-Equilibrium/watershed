//! The branch tree of a session's stored entries.

use std::collections::HashMap;

use super::entry::{Body, Call, Entry, OpenCalls, first_line};
use crate::{EntryKind, SessionTree, TreeNode, pi};

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
        .zip(result_targets(entries))
        .filter(|(entry, _)| !matches!(entry.body, Body::Label { .. }))
        .map(|(entry, target)| {
            let (kind, preview) = summary(&entry.body, target);
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

/// What the call each tool result answers works on, by the result's place in `entries`. One walk
/// down every branch carries the calls its responses left open, so each result is matched once.
fn result_targets(entries: &[Entry]) -> Vec<Option<String>> {
    /// Entering an entry, or leaving it with the call it answered.
    enum Step<'a> {
        Enter(usize),
        Leave(usize, Option<(usize, &'a Call)>),
    }
    // A parent id names its last entry, as on a branch; an entry reached only through a stored
    // cycle is never entered.
    let by_id: HashMap<&str, usize> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.id.as_str(), index))
        .collect();
    let mut children = vec![Vec::new(); entries.len()];
    let mut steps = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        match entry.parent.as_deref().and_then(|parent| by_id.get(parent)) {
            Some(&parent) => children[parent].push(index),
            None => steps.push(Step::Enter(index)),
        }
    }
    let mut targets = vec![None; entries.len()];
    // The open calls of each response above the entry, the nearest last.
    let mut open: Vec<OpenCalls> = Vec::new();
    while let Some(step) = steps.pop() {
        match step {
            Step::Enter(index) => {
                let mut answered = None;
                match &entries[index].body {
                    Body::Assistant { calls, .. } => {
                        let mut response = OpenCalls::default();
                        response.respond(calls);
                        open.push(response);
                    }
                    Body::ToolResult { call, .. } => {
                        answered = open.last_mut().and_then(|calls| calls.answer(call));
                        targets[index] = answered.map(|(_, call)| pi::call_target(&call.arguments));
                    }
                    _ => {}
                }
                steps.push(Step::Leave(index, answered));
                steps.extend(children[index].iter().map(|&child| Step::Enter(child)));
            }
            // Another branch may continue from the entry's parent.
            Step::Leave(index, answered) => {
                if matches!(entries[index].body, Body::Assistant { .. }) {
                    open.pop();
                } else if let (Some((place, call)), Some(calls)) = (answered, open.last_mut()) {
                    calls.reopen(place, call);
                }
            }
        }
    }
    targets
}

fn summary(body: &Body, target: Option<String>) -> (EntryKind, String) {
    let line = |text: &str| first_line(text, PREVIEW_CHARS);
    match body {
        Body::User { text, .. } => (EntryKind::User, line(text)),
        Body::Assistant { text, .. } => (EntryKind::Assistant, line(text)),
        Body::ToolResult { tool, .. } => {
            let preview = match target {
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
