use crate::runtime::types::RuntimeError;
use proto::EventEnvelope;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Clone)]
pub(crate) struct LifecycleTracker<K: Ord> {
    active: BTreeSet<K>,
    terminal: BTreeMap<K, usize>,
}

impl<K: Ord> Default for LifecycleTracker<K> {
    fn default() -> Self {
        Self {
            active: BTreeSet::new(),
            terminal: BTreeMap::new(),
        }
    }
}

impl<K: Ord> LifecycleTracker<K> {
    pub(crate) fn start(&mut self, key: K) {
        self.active.insert(key);
    }

    pub(crate) fn finish(&mut self, key: K, line_number: usize) {
        self.active.remove(&key);
        self.terminal.insert(key, line_number);
    }

    pub(crate) fn is_started(&self, key: &K) -> bool {
        self.active.contains(key) || self.terminal.contains_key(key)
    }

    pub(crate) fn terminal_line(&self, key: &K) -> Option<usize> {
        self.terminal.get(key).copied()
    }

    pub(crate) fn active_keys(&self) -> impl Iterator<Item = &K> {
        self.active.iter()
    }

    #[cfg(test)]
    pub(super) fn keys(&self) -> impl Iterator<Item = &K> {
        self.active.iter().chain(self.terminal.keys())
    }
}

pub(super) fn open_child_lifecycle_error(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
    child_kind: &str,
    child_id: &str,
) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "{} line {line_number} {} requires no active {child_kind} {child_id:?}",
        path.display(),
        event.event_type.as_str()
    ))
}

pub(super) fn terminal_lifecycle_error(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
    kind: &str,
    id: &str,
    terminal_line: usize,
) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "{} line {line_number} {} appears after terminal {kind} {id:?} on line {terminal_line}",
        path.display(),
        event.event_type.as_str()
    ))
}

pub(super) fn require_lifecycle_flow_id(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
) -> Result<String, RuntimeError> {
    event.flow_id.clone().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "{} line {line_number} {} must include flow_id",
            path.display(),
            event.event_type.as_str()
        ))
    })
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct MessageLifecycleKey {
    pub(crate) flow_id: String,
    pub(crate) message_id: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ToolLifecycleKey {
    pub(crate) flow_id: Option<String>,
    pub(crate) phase_execution_id: Option<String>,
    pub(crate) phase_id: Option<String>,
    pub(crate) step_id: Option<String>,
    pub(crate) tool_id: String,
    pub(crate) attempt_id: Option<String>,
}

pub(crate) fn lifecycle_payload_string(event: &EventEnvelope, field: &str) -> String {
    event
        .payload
        .get(field)
        .and_then(serde_json::Value::as_str)
        .expect("payload contract validation ensures lifecycle key fields are strings")
        .to_owned()
}
