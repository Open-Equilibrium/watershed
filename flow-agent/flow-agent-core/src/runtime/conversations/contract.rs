pub(crate) use crate::runtime::types::MAX_SESSION_SEGMENT_BYTES as MAX_CONVERSATION_SEGMENT_BYTES;
use crate::runtime::{
    digest::{is_lowercase_sha256_hex, strip_sha256_prefix},
    fs_guards::AnchoredDirectoryIdentity,
    types::RuntimeError,
};
use proto::decode_lowercase_sha256_hex;

pub(crate) const MAX_CONVERSATION_RECORD_BYTES: usize = 256 * 1024;
pub(crate) const MAX_CONVERSATION_IO_BUFFER_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_CONVERSATION_STATUS_RECORDS: usize = 100;
pub(crate) const MAX_CONVERSATION_STATUS_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_CONVERSATION_SCAN_RECORDS: usize = 4_096;
pub(crate) const MAX_CONVERSATION_SCAN_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const CONVERSATION_HISTORY_LEAF: &str = "history.jsonl";
pub(crate) const CONVERSATION_RUNS_DIR: &str = "runs";
pub(crate) const CONVERSATION_STATUS_LEAF: &str = "status.json";
pub(crate) const RUN_CONTEXTS_LEAF: &str = "contexts.jsonl";
pub(crate) const RUN_CONTEXTS_STEM: &str = "contexts";
pub(crate) const RUN_EVENTS_LEAF: &str = "events.jsonl";
pub(crate) const RUN_EVENTS_STEM: &str = "events";
pub(crate) const RUN_LOG_LEAF: &str = "run-log.jsonl";
pub(crate) const RUN_OBJECTS_DIR: &str = "objects";
pub(crate) const RUN_RECOVERY_LEAF: &str = "recovery.jsonl";
pub(crate) const RUN_SESSION_LOCK_LEAF: &str = "session.lock";
pub(crate) const TOOL_RUN_LOG_PAGE_SCHEMA: &str = "flow-tool-run-log-page-v0";
pub(crate) const CONVERSATION_STATUS_PAGE_SCHEMA: &str = "flow-conversation-status-page-v0";

pub(crate) const RUN_LOG_RECORD_SCHEMA_V1: &str = "flow-run-log-record-v1";
pub(super) const UNPUBLISHED_PRODUCTIVE_RUN_MARKER: &str = ".unpublished-productive-run";
const RUN_CREATION_STAGE_MARKER_PREFIX: &str = ".run-creation-identity-";
const CONVERSATION_LIFECYCLE_MARKER_PREFIX: &str = ".conversation-lifecycle-identity-";

pub(super) fn conversation_lifecycle_identity_marker_name(
    identity: AnchoredDirectoryIdentity,
) -> String {
    format!(
        "{CONVERSATION_LIFECYCLE_MARKER_PREFIX}{:016x}-{:016x}",
        identity.device, identity.inode
    )
}

pub(super) fn is_conversation_lifecycle_identity_marker_name(marker: &str) -> bool {
    valid_identity_marker_name(marker, CONVERSATION_LIFECYCLE_MARKER_PREFIX)
}

pub(super) fn run_creation_identity_marker_name(identity: AnchoredDirectoryIdentity) -> String {
    format!(
        "{RUN_CREATION_STAGE_MARKER_PREFIX}{:016x}-{:016x}",
        identity.device, identity.inode
    )
}

pub(super) fn validate_run_creation_identity_marker_name(marker: &str) -> Result<(), RuntimeError> {
    if !valid_identity_marker_name(marker, RUN_CREATION_STAGE_MARKER_PREFIX) {
        return Err(protocol("run-creation identity marker name is invalid"));
    }
    Ok(())
}

fn valid_identity_marker_name(marker: &str, prefix: &str) -> bool {
    marker
        .strip_prefix(prefix)
        .and_then(|identity| identity.split_once('-'))
        .is_some_and(|(device, inode)| {
            device.len() == 16
                && inode.len() == 16
                && device
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                && inode
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
}

pub(super) fn validate_timestamp(timestamp: &str) -> Result<(), RuntimeError> {
    proto::parse_rfc3339_utc_timestamp(timestamp)
        .map(|_| ())
        .ok_or_else(|| protocol("record timestamp must be canonical RFC 3339 UTC"))
}

pub(super) fn validate_attempt_id(id: &str) -> Result<(), RuntimeError> {
    validate_id(id, "run attempt")
}

pub(super) fn validate_id(id: &str, kind: &str) -> Result<(), RuntimeError> {
    if proto::is_valid_session_id(id) {
        Ok(())
    } else {
        Err(RuntimeError::Usage(format!("invalid {kind} id")))
    }
}

pub(super) fn validate_hash(value: &str, kind: &str) -> Result<(), RuntimeError> {
    let Some(digest) = strip_sha256_prefix(value) else {
        return Err(protocol(format!("{kind} must use sha256:<digest>")));
    };
    if is_lowercase_sha256_hex(digest) {
        Ok(())
    } else {
        Err(protocol(format!(
            "{kind} must use a lowercase SHA-256 digest"
        )))
    }
}

pub(super) fn validate_digest(value: &str, kind: &str) -> Result<(), RuntimeError> {
    if !is_lowercase_sha256_hex(value) {
        return Err(protocol(format!(
            "{kind} must be a lowercase SHA-256 digest"
        )));
    }
    Ok(())
}

pub(super) fn parse_run_object_digest(value: &str) -> Result<[u8; 32], RuntimeError> {
    decode_lowercase_sha256_hex(value)
        .ok_or_else(|| protocol("run object must be a lowercase SHA-256 digest"))
}

pub(super) fn validate_record_schema(schema: &str) -> Result<(), RuntimeError> {
    if schema == RUN_LOG_RECORD_SCHEMA_V1 {
        Ok(())
    } else {
        Err(protocol("run log record has an unsupported schema"))
    }
}

pub(super) fn protocol(message: impl Into<String>) -> RuntimeError {
    RuntimeError::Protocol(message.into())
}
