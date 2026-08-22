use super::super::{
    contract::{CONVERSATION_STATUS_LEAF, protocol, validate_id},
    storage::canonical_json,
};
use crate::runtime::{
    fs_guards::{
        AnchoredDir, AnchoredFile, create_anchored_file, open_anchored_file_for_read, path_io_error,
    },
    types::RuntimeError,
};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

pub(crate) const MAX_CONVERSATION_STATUS_SUMMARY_BYTES: usize = 4 * 1024;
pub(crate) const STATUS_SUMMARY_SCHEMA: &str = "flow-conversation-status-summary-v0";
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConversationStatusSummary {
    pub(crate) schema: String,
    pub(crate) conversation_id: String,
    pub(crate) latest_entry_id: Option<String>,
    pub(crate) run_count: u64,
    pub(crate) uncertain_attempts: u64,
}

fn empty_status_summary(conversation_id: &str) -> ConversationStatusSummary {
    ConversationStatusSummary {
        schema: STATUS_SUMMARY_SCHEMA.to_owned(),
        conversation_id: conversation_id.to_owned(),
        latest_entry_id: None,
        run_count: 0,
        uncertain_attempts: 0,
    }
}

pub(super) fn validate_status_summary(
    summary: &ConversationStatusSummary,
    conversation_id: &str,
) -> Result<(), RuntimeError> {
    if summary.schema != STATUS_SUMMARY_SCHEMA {
        return Err(protocol(
            "conversation status summary has an unsupported schema",
        ));
    }
    if summary.conversation_id != conversation_id {
        return Err(protocol(
            "conversation status summary has the wrong conversation id",
        ));
    }
    validate_id(&summary.conversation_id, "conversation status summary")?;
    if summary
        .latest_entry_id
        .as_deref()
        .is_some_and(|id| !proto::is_valid_session_id(id))
    {
        return Err(protocol(
            "conversation status summary has an invalid latest entry id",
        ));
    }
    Ok(())
}

fn bounded_canonical_json_bytes(
    value: &impl Serialize,
    label: &str,
) -> Result<Vec<u8>, RuntimeError> {
    let mut bytes = canonical_json(value)?.into_bytes();
    bytes.push(b'\n');
    if bytes.len() > MAX_CONVERSATION_STATUS_SUMMARY_BYTES {
        return Err(protocol(format!("{label} exceeds its byte limit")));
    }
    Ok(bytes)
}

pub(in crate::runtime::conversations) fn create_bounded_canonical_json_file(
    path: &AnchoredFile,
    value: &impl Serialize,
    label: &str,
) -> Result<(), RuntimeError> {
    let bytes = bounded_canonical_json_bytes(value, label)?;
    let mut file = create_anchored_file(path)?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|source| path_io_error(path.diagnostic_path(), source))
}

pub(super) fn read_bounded_canonical_json_file<T>(
    path: &AnchoredFile,
    label: &str,
) -> Result<T, RuntimeError>
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    let (opened, metadata) = open_anchored_file_for_read(path)?;
    if metadata.len() > MAX_CONVERSATION_STATUS_SUMMARY_BYTES as u64 {
        return Err(protocol(format!("{label} exceeds its byte limit")));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    opened
        .take(MAX_CONVERSATION_STATUS_SUMMARY_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
    if bytes.len() > MAX_CONVERSATION_STATUS_SUMMARY_BYTES {
        return Err(protocol(format!("{label} exceeds its byte limit")));
    }
    if bytes.last() != Some(&b'\n')
        || bytes[..bytes.len().saturating_sub(1)].contains(&b'\n')
        || bytes.contains(&b'\r')
    {
        return Err(protocol(format!("{label} framing is invalid")));
    }
    let body = std::str::from_utf8(&bytes[..bytes.len() - 1])
        .map_err(|_| protocol(format!("{label} is not UTF-8")))?;
    let value: T = serde_json::from_str(body)
        .map_err(|error| protocol(format!("{label} is not valid JSON: {error}")))?;
    if canonical_json(&value)? != body {
        return Err(protocol(format!("{label} is not canonical JSON")));
    }
    Ok(value)
}

pub(in crate::runtime::conversations) fn status_summary_file(
    conversation: &AnchoredDir,
) -> AnchoredFile {
    conversation.file(CONVERSATION_STATUS_LEAF)
}

pub(in crate::runtime::conversations) fn read_status_summary(
    conversation: &AnchoredDir,
    conversation_id: &str,
) -> Result<ConversationStatusSummary, RuntimeError> {
    let path = status_summary_file(conversation);
    let summary = match read_bounded_canonical_json_file(&path, "conversation status summary") {
        Err(RuntimeError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(protocol("conversation status summary is missing"));
        }
        result => result?,
    };
    validate_status_summary(&summary, conversation_id)?;
    Ok(summary)
}

pub(in crate::runtime::conversations) fn create_initial_status_summary(
    conversation: &AnchoredDir,
    conversation_id: &str,
) -> Result<(), RuntimeError> {
    let summary = empty_status_summary(conversation_id);
    create_bounded_canonical_json_file(
        &status_summary_file(conversation),
        &summary,
        "conversation status summary",
    )
}
