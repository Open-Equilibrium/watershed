use super::contract::{
    CONVERSATION_RUNS_DIR, MAX_CONVERSATION_SCAN_BYTES, MAX_CONVERSATION_SCAN_RECORDS, protocol,
    validate_id,
};
use crate::runtime::{
    fs_guards::{
        AnchoredDir, AnchoredWorkspace, DirectoryErrorMode, RuntimeDirs,
        ensure_anchored_runtime_dirs, open_anchored_file_for_read, path_io_error,
    },
    types::RuntimeError,
};
use serde::Serialize;
#[cfg(test)]
use std::fs;
use std::{collections::BTreeSet, path::Path};

#[cfg(any(test, feature = "m11-budget-evidence"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ConversationScanMetrics {
    pub(crate) entries: usize,
    pub(crate) stored_bytes: u64,
}

#[cfg(any(test, feature = "m11-budget-evidence"))]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ConversationOperationMetrics {
    pub(crate) max_read_request_bytes: usize,
    pub(crate) max_write_request_bytes: usize,
    pub(crate) quanta: Vec<ConversationScanMetrics>,
}

#[cfg(any(test, feature = "m11-budget-evidence"))]
thread_local! {
    static CONVERSATION_OPERATION_METRICS: std::cell::RefCell<Option<ConversationOperationMetrics>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(crate) fn measure_conversation_operation<T>(
    operation: impl FnOnce() -> Result<T, RuntimeError>,
) -> Result<(T, ConversationOperationMetrics), RuntimeError> {
    CONVERSATION_OPERATION_METRICS.with(|slot| {
        if slot.borrow().is_some() {
            return Err(protocol(
                "conversation operation measurement is already active",
            ));
        }
        slot.replace(Some(ConversationOperationMetrics::default()));
        let result = operation();
        let metrics = slot
            .replace(None)
            .expect("conversation operation measurement was installed above");
        result.map(|value| (value, metrics))
    })
}

pub(super) fn record_conversation_read_request(_bytes: usize) {
    #[cfg(any(test, feature = "m11-budget-evidence"))]
    CONVERSATION_OPERATION_METRICS.with(|slot| {
        if let Some(metrics) = slot.borrow_mut().as_mut() {
            metrics.max_read_request_bytes = metrics.max_read_request_bytes.max(_bytes);
        }
    });
}

pub(crate) struct ConversationScanQuantum {
    entries: usize,
    stored_bytes: u64,
}

impl ConversationScanQuantum {
    pub(crate) fn new() -> Self {
        Self {
            entries: 0,
            stored_bytes: 0,
        }
    }

    pub(crate) fn admit_record(&mut self, stored_bytes: usize) -> Result<(), RuntimeError> {
        let stored_bytes = u64::try_from(stored_bytes).unwrap_or(u64::MAX);
        if stored_bytes > MAX_CONVERSATION_SCAN_BYTES {
            return Err(protocol("conversation record exceeds one scan quantum"));
        }
        if self.entries == MAX_CONVERSATION_SCAN_RECORDS
            || self.stored_bytes.saturating_add(stored_bytes) > MAX_CONVERSATION_SCAN_BYTES
        {
            self.flush();
        }
        self.entries += 1;
        self.stored_bytes += stored_bytes;
        Ok(())
    }

    pub(crate) fn finish(&mut self) {
        self.flush();
    }

    fn flush(&mut self) {
        if self.entries == 0 && self.stored_bytes == 0 {
            return;
        }
        #[cfg(any(test, feature = "m11-budget-evidence"))]
        CONVERSATION_OPERATION_METRICS.with(|slot| {
            if let Some(metrics) = slot.borrow_mut().as_mut() {
                metrics.quanta.push(ConversationScanMetrics {
                    entries: self.entries,
                    stored_bytes: self.stored_bytes,
                });
            }
        });
        self.entries = 0;
        self.stored_bytes = 0;
    }
}

pub(super) fn ensure_runtime_roots(workspace: &Path) -> Result<RuntimeDirs, RuntimeError> {
    let workspace = AnchoredWorkspace::open(workspace)?;
    ensure_anchored_runtime_dirs(&workspace)
}

pub(super) fn ensure_anchored_sessions(workspace: &Path) -> Result<AnchoredDir, RuntimeError> {
    let workspace = AnchoredWorkspace::open(workspace)?;
    Ok(ensure_anchored_runtime_dirs(&workspace)?.sessions)
}

pub(super) fn required_child(
    parent: &AnchoredDir,
    leaf: &str,
    label: &str,
) -> Result<AnchoredDir, RuntimeError> {
    parent
        .child(leaf, false, DirectoryErrorMode::Protocol)?
        .ok_or_else(|| protocol(format!("{label} does not exist")))
}

pub(crate) fn existing_anchored_run(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
) -> Result<AnchoredDir, RuntimeError> {
    existing_anchored_run_with_parent(workspace, conversation_id, run_session_id)
        .map(|(_, run)| run)
}

pub(super) fn existing_anchored_conversation(
    workspace: &Path,
    conversation_id: &str,
) -> Result<AnchoredDir, RuntimeError> {
    existing_anchored_conversation_with_parent(workspace, conversation_id)
        .map(|(_, conversation)| conversation)
}

fn existing_anchored_conversation_with_parent(
    workspace: &Path,
    conversation_id: &str,
) -> Result<(AnchoredDir, AnchoredDir), RuntimeError> {
    validate_id(conversation_id, "conversation")?;
    let sessions = ensure_anchored_sessions(workspace)?;
    let conversation = sessions
        .child(conversation_id, false, DirectoryErrorMode::Protocol)?
        .ok_or_else(|| protocol("conversation does not exist"))?;
    Ok((sessions, conversation))
}

fn existing_anchored_run_with_parent(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
) -> Result<(AnchoredDir, AnchoredDir), RuntimeError> {
    validate_id(run_session_id, "run session")?;
    let conversation = existing_anchored_conversation(workspace, conversation_id)?;
    let runs = required_child(
        &conversation,
        CONVERSATION_RUNS_DIR,
        "conversation runs directory",
    )?;
    let run = runs
        .child(run_session_id, false, DirectoryErrorMode::Protocol)?
        .ok_or_else(|| protocol("conversation run does not exist"))?;
    Ok((runs, run))
}

pub(super) fn bounded_anchored_real_child_file_names(
    directory: &AnchoredDir,
    maximum: usize,
    name_kind: &str,
) -> Result<BTreeSet<String>, RuntimeError> {
    let mut names = BTreeSet::new();
    for entry in directory
        .dir
        .entries()
        .map_err(|source| path_io_error(&directory.path, source))?
    {
        let name = entry
            .map_err(|source| path_io_error(&directory.path, source))?
            .file_name()
            .into_string()
            .map_err(|_| protocol(format!("{name_kind} name must be UTF-8")))?;
        let (opened, _) = open_anchored_file_for_read(&directory.file(&name))?;
        drop(opened);
        names.insert(name);
        if names.len() > maximum {
            break;
        }
    }
    Ok(names)
}

#[cfg(test)]
pub(crate) fn real_child_file_names(
    path: &Path,
    maximum: usize,
) -> Result<BTreeSet<String>, RuntimeError> {
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(path).map_err(|source| path_io_error(path, source))? {
        let entry = entry.map_err(|source| path_io_error(path, source))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| protocol("run object name must be UTF-8"))?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|source| path_io_error(&entry.path(), source))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(protocol(
                "run object inventory must contain only real files",
            ));
        }
        names.insert(name);
        if names.len() > maximum {
            break;
        }
    }
    Ok(names)
}

pub(crate) fn canonical_json(value: &impl Serialize) -> Result<String, RuntimeError> {
    let value = serde_json::to_value(value).map_err(RuntimeError::Json)?;
    proto::canonical_json(&value)
        .map_err(|error| protocol(format!("record cannot be canonicalized: {error}")))
}
