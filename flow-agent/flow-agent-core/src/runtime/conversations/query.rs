use super::storage::{CONVERSATION_MIGRATIONS_DIR, migration_transaction_id};
use super::{
    contract::{
        CONVERSATION_STATUS_PAGE_SCHEMA, MAX_CONVERSATION_STATUS_BYTES,
        MAX_CONVERSATION_STATUS_RECORDS, protocol, validate_id,
    },
    legacy_migration::{
        legacy_log_source_id, legacy_session_is_terminal, legacy_session_source_id,
        migrate_legacy_session_if_present,
    },
    lifecycle::reconcile_releases,
    status::{read_status_summary, recover_status_transaction, status_recovery_is_required},
    storage::{canonical_json, existing_anchored_conversation},
};
use crate::runtime::{
    fs_guards::{
        AnchoredDir, AnchoredWorkspace, DirectoryErrorMode, ensure_anchored_runtime_dirs,
        path_io_error,
    },
    session_authority::{SessionOwnershipLease, conversation_ownership_key},
    session_bundle::SessionBundlePaths,
    stage_results::reconcile_controlled_stages,
    types::{EmitMode, RuntimeError},
};
use serde::Serialize;
use std::{collections::BTreeSet, path::Path};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct ConversationStatus {
    pub(crate) conversation_id: String,
    pub(crate) latest_entry_id: Option<String>,
    pub(crate) run_count: usize,
    pub(crate) uncertain_attempts: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct ConversationStatusPage {
    pub(crate) schema: String,
    pub(crate) conversations: Vec<ConversationStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) continuation_token: Option<String>,
}

struct ConversationStatusCandidate {
    id: String,
    legacy_source_present: bool,
    migration_present: bool,
}

struct StatusInventoryBudget {
    entries: usize,
}

impl StatusInventoryBudget {
    fn new() -> Self {
        Self { entries: 0 }
    }

    fn admit(&mut self) -> Result<(), RuntimeError> {
        if self.entries == super::contract::MAX_CONVERSATION_SCAN_RECORDS {
            return Err(protocol(
                "conversation status inventory exceeds one scan quantum",
            ));
        }
        self.entries += 1;
        Ok(())
    }
}

pub(crate) fn conversation_status_page(
    workspace: &Path,
    continuation_token: Option<&str>,
) -> Result<ConversationStatusPage, RuntimeError> {
    let after = continuation_token.map(parse_status_token).transpose()?;
    let anchored_workspace = AnchoredWorkspace::open(workspace)?;
    let roots = ensure_anchored_runtime_dirs(&anchored_workspace)?;
    let sessions = &roots.sessions;
    let (candidates, inventory_has_more) =
        conversation_id_page(sessions, &roots.logs, after.as_deref())?;
    let mut conversations =
        Vec::with_capacity(candidates.len().min(MAX_CONVERSATION_STATUS_RECORDS));
    let mut conversation_bytes = 0usize;
    let mut has_more = inventory_has_more;
    let mut processed_cursor = None;
    let page_candidate_count = candidates.len().min(MAX_CONVERSATION_STATUS_RECORDS);
    for (index, candidate) in candidates.iter().take(page_candidate_count).enumerate() {
        let id = &candidate.id;
        let target_present = sessions
            .child(id, false, DirectoryErrorMode::Protocol)?
            .is_some();
        if candidate.legacy_source_present
            && !target_present
            && !candidate.migration_present
            && legacy_session_is_terminal(workspace, id)? == Some(false)
        {
            processed_cursor = Some(id);
            continue;
        }
        if candidate.legacy_source_present || candidate.migration_present {
            migrate_legacy_session_if_present(workspace, id)?;
        }
        let status = conversation_status_summary(workspace, id)?;
        let status_bytes = canonical_json(&status)?.len();
        conversations.push(status);
        let candidate_has_more = inventory_has_more || index + 1 < page_candidate_count;
        let candidate_token = candidate_has_more.then(|| status_token(id));
        let candidate_conversation_bytes = conversation_bytes.saturating_add(status_bytes);
        if conversation_status_page_bytes(
            candidate_conversation_bytes,
            conversations.len(),
            candidate_token.as_deref(),
        )? > MAX_CONVERSATION_STATUS_BYTES
        {
            conversations.pop();
            has_more = true;
            break;
        }
        conversation_bytes = candidate_conversation_bytes;
        processed_cursor = Some(id);
    }
    let continuation_token = has_more
        .then(|| processed_cursor.map(|id| status_token(id)))
        .flatten();
    let page = ConversationStatusPage {
        schema: CONVERSATION_STATUS_PAGE_SCHEMA.to_owned(),
        conversations,
        continuation_token,
    };
    debug_assert!(canonical_json(&page)?.len() <= MAX_CONVERSATION_STATUS_BYTES);
    Ok(page)
}

fn conversation_status_page_bytes(
    record_bytes: usize,
    record_count: usize,
    continuation_token: Option<&str>,
) -> Result<usize, RuntimeError> {
    let empty = ConversationStatusPage {
        schema: CONVERSATION_STATUS_PAGE_SCHEMA.to_owned(),
        conversations: Vec::new(),
        continuation_token: continuation_token.map(str::to_owned),
    };
    Ok(canonical_json(&empty)?
        .len()
        .saturating_add(record_bytes)
        .saturating_add(record_count.saturating_sub(1)))
}

fn conversation_status_summary(
    workspace: &Path,
    conversation_id: &str,
) -> Result<ConversationStatus, RuntimeError> {
    let conversation = existing_anchored_conversation(workspace, conversation_id)?;
    if status_recovery_is_required(&conversation)? {
        let legacy =
            SessionOwnershipLease::acquire(workspace, conversation_id, &conversation.path)?;
        let key = conversation_ownership_key(conversation_id);
        let lease = match SessionOwnershipLease::acquire(workspace, &key, &conversation.path) {
            Ok(lease) => lease,
            Err(error) => {
                let release = legacy.release();
                return reconcile_controlled_stages(Err(error), Ok(()), release);
            }
        };
        let operation = recover_status_transaction(&conversation, conversation_id);
        let release = reconcile_releases(Ok(()), lease.release(), legacy.release());
        reconcile_controlled_stages(operation, Ok(()), release)?;
    }
    let summary = read_status_summary(&conversation, conversation_id)?;
    Ok(ConversationStatus {
        conversation_id: summary.conversation_id,
        latest_entry_id: summary.latest_entry_id,
        run_count: usize::try_from(summary.run_count)
            .map_err(|_| protocol("conversation Run count exceeds this platform"))?,
        uncertain_attempts: usize::try_from(summary.uncertain_attempts)
            .map_err(|_| protocol("uncertain attempt count exceeds this platform"))?,
    })
}

/// Renders one bounded deterministic page of conversation status.
pub fn conversation_status(
    workspace: impl AsRef<Path>,
    continuation_token: Option<&str>,
    emit: EmitMode,
) -> Result<String, RuntimeError> {
    let page = conversation_status_page(workspace.as_ref(), continuation_token)?;
    match emit {
        EmitMode::Jsonl => Ok(format!("{}\n", canonical_json(&page)?)),
        EmitMode::Human => {
            let mut output = String::new();
            for conversation in &page.conversations {
                output.push_str(&format!(
                    "conversation {}: {} runs, {} uncertain attempts",
                    conversation.conversation_id,
                    conversation.run_count,
                    conversation.uncertain_attempts
                ));
                if let Some(entry) = &conversation.latest_entry_id {
                    output.push_str(&format!(", latest entry {entry}"));
                }
                output.push('\n');
            }
            if let Some(token) = page.continuation_token {
                output.push_str(&format!(
                    "more conversations available; continue with flow sessions status --emit jsonl --continuation-token {token}\n"
                ));
            }
            Ok(output)
        }
    }
}

fn conversation_id_page(
    sessions: &AnchoredDir,
    logs: &AnchoredDir,
    after: Option<&str>,
) -> Result<(Vec<ConversationStatusCandidate>, bool), RuntimeError> {
    let mut names = BTreeSet::new();
    let mut legacy_source_ids = BTreeSet::new();
    let mut migration_ids = BTreeSet::new();
    let mut inventory_budget = StatusInventoryBudget::new();
    for entry in sessions
        .dir
        .entries()
        .map_err(|source| path_io_error(&sessions.path, source))?
    {
        let entry = entry.map_err(|source| path_io_error(&sessions.path, source))?;
        inventory_budget.admit()?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| protocol("conversation directory name must be UTF-8"))?;
        let path = sessions.path.join(&name);
        let metadata = sessions
            .dir
            .symlink_metadata(&name)
            .map_err(|source| path_io_error(&path, source))?;
        if metadata.file_type().is_symlink() {
            return Err(protocol("conversation inventory must not contain symlinks"));
        }
        if let Some(id) =
            legacy_session_source_id(&name).filter(|id| proto::is_valid_session_id(id))
        {
            legacy_source_ids.insert(id.to_owned());
        }
        let candidate = if metadata.is_dir() && proto::is_valid_session_id(&name) {
            Some(name)
        } else if metadata.is_file() {
            SessionBundlePaths::split_events_leaf(&name)
                .filter(|id| proto::is_valid_session_id(id))
                .map(str::to_owned)
        } else {
            None
        };
        if let Some(candidate) = candidate {
            retain_status_candidate(&mut names, candidate, after);
        }
    }
    if let Some(migrations) = sessions.child(
        CONVERSATION_MIGRATIONS_DIR,
        false,
        DirectoryErrorMode::Protocol,
    )? {
        for entry in migrations
            .dir
            .entries()
            .map_err(|source| path_io_error(&migrations.path, source))?
        {
            let entry = entry.map_err(|source| path_io_error(&migrations.path, source))?;
            inventory_budget.admit()?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| protocol("migration transaction name must be UTF-8"))?;
            let path = migrations.path.join(&name);
            let metadata = migrations
                .dir
                .symlink_metadata(&name)
                .map_err(|source| path_io_error(&path, source))?;
            if metadata.file_type().is_symlink() {
                return Err(protocol("migration transaction must be a real file"));
            }
            if metadata.is_file()
                && let Some(id) =
                    migration_transaction_id(&name).filter(|id| proto::is_valid_session_id(id))
            {
                let id = id.to_owned();
                if let Some(evicted) = retain_status_candidate(&mut names, id.clone(), after) {
                    migration_ids.remove(&evicted);
                }
                if names.contains(&id) {
                    migration_ids.insert(id);
                }
            }
        }
    }
    legacy_source_ids.retain(|candidate| names.contains(candidate));
    for entry in logs
        .dir
        .entries()
        .map_err(|source| path_io_error(&logs.path, source))?
    {
        let entry = entry.map_err(|source| path_io_error(&logs.path, source))?;
        inventory_budget.admit()?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if let Some(id) = legacy_log_source_id(&name).filter(|id| names.contains(*id)) {
            legacy_source_ids.insert(id.to_owned());
        }
    }
    debug_assert!(names.len() <= MAX_CONVERSATION_STATUS_RECORDS + 1);
    debug_assert!(migration_ids.len() <= names.len());
    debug_assert!(legacy_source_ids.len() <= names.len());
    let has_more = names.len() > MAX_CONVERSATION_STATUS_RECORDS;
    let candidates = names
        .into_iter()
        .map(|id| ConversationStatusCandidate {
            legacy_source_present: legacy_source_ids.contains(&id),
            migration_present: migration_ids.contains(&id),
            id,
        })
        .collect();
    Ok((candidates, has_more))
}

fn retain_status_candidate(
    names: &mut BTreeSet<String>,
    candidate: String,
    after: Option<&str>,
) -> Option<String> {
    if after.is_none_or(|after| candidate.as_str() > after) {
        names.insert(candidate);
        if names.len() > MAX_CONVERSATION_STATUS_RECORDS + 1 {
            return names.pop_last();
        }
    }
    None
}

const STATUS_TOKEN_PREFIX: &str = "flow-status-v0:";

fn status_token(conversation_id: &str) -> String {
    format!("{STATUS_TOKEN_PREFIX}{conversation_id}")
}

fn parse_status_token(token: &str) -> Result<String, RuntimeError> {
    let id = token
        .strip_prefix(STATUS_TOKEN_PREFIX)
        .ok_or_else(|| RuntimeError::Usage("invalid continuation token".to_owned()))?;
    validate_id(id, "continuation token")?;
    Ok(id.to_owned())
}
