use super::codec::resolved_policy_digest_v0;
use super::stream::decode_executor_stream_v0;
use super::{
    EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0, EXECUTOR_REQUEST_SCHEMA_V0, EnforcementReceiptV0,
    ExecutorExecVectorErrorV0, ExecutorMountAccessV0, ExecutorMountOriginV0, ExecutorProtocolError,
    ExecutorRequestV0, ExecutorToolClassificationV0, ExecutorToolResultV0, ExecutorToolStatusV0,
    MAX_ENVIRONMENT_ENTRIES, MAX_EXECUTOR_MOUNTS_V0, MAX_EXECUTOR_RUNTIME_MOUNTS_V0,
    MAX_EXECUTOR_TOOL_STREAM_BYTES_V0, MAX_EXECUTOR_WORKSPACE_MOUNTS_V0, MAX_ID_CHARS,
    MAX_NAME_CHARS, MAX_PATH_CHARS, validate_executor_exec_vector_v0,
};
use crate::session_object::decode_lowercase_sha256_hex;

pub(super) fn validate_request(request: &ExecutorRequestV0) -> Result<(), ExecutorProtocolError> {
    validate_schema(&request.schema, EXECUTOR_REQUEST_SCHEMA_V0, "request")?;
    validate_text(&request.request_id, "request_id", MAX_ID_CHARS)?;
    validate_text(&request.tool_id, "tool_id", MAX_ID_CHARS)?;
    validate_text(&request.tool_kind, "tool_kind", MAX_NAME_CHARS)?;
    validate_text(&request.executable, "executable", MAX_PATH_CHARS)?;
    validate_absolute_path(&request.executable, "executable")?;
    validate_text(
        &request.working_directory,
        "working_directory",
        MAX_PATH_CHARS,
    )?;
    validate_absolute_path(&request.working_directory, "working_directory")?;
    if request.environment.len() > MAX_ENVIRONMENT_ENTRIES {
        return Err(ExecutorProtocolError::new(
            "Executor environment has too many entries",
        ));
    }
    for (name, value) in &request.environment {
        validate_text(name, "environment name", MAX_NAME_CHARS)?;
        validate_text(value, "environment value", MAX_PATH_CHARS)?;
    }
    if let Err(error) =
        validate_executor_exec_vector_v0(&request.executable, &request.argv, &request.environment)
    {
        return Err(ExecutorProtocolError::new(match error {
            ExecutorExecVectorErrorV0::NulByte => "Executor argv is invalid",
            ExecutorExecVectorErrorV0::EntryBudget { .. } => "Executor argv entry bound is invalid",
            ExecutorExecVectorErrorV0::ByteBudget { actual: usize::MAX } => {
                "Executor argv byte count overflow"
            }
            ExecutorExecVectorErrorV0::ByteBudget { .. } => "Executor argv exceeds its byte limit",
        }));
    }
    if request.mounts.len() > MAX_EXECUTOR_MOUNTS_V0 {
        return Err(ExecutorProtocolError::new(
            "Executor mount list exceeds its limit",
        ));
    }
    let mut descriptors = std::collections::BTreeSet::new();
    let mut targets = std::collections::BTreeSet::new();
    let mut runtime_mounts = 0_usize;
    let mut workspace_mounts = 0_usize;
    for (index, mount) in request.mounts.iter().enumerate() {
        let expected_descriptor = EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0
            .checked_add(u32::try_from(index).expect("mount limit fits u32"))
            .expect("mount descriptor range is bounded");
        if mount.descriptor != expected_descriptor || !descriptors.insert(mount.descriptor) {
            return Err(ExecutorProtocolError::new(
                "Executor mount descriptor is invalid",
            ));
        }
        validate_text(&mount.target, "mount target", MAX_PATH_CHARS)?;
        validate_absolute_path(&mount.target, "mount target")?;
        if !targets.insert(mount.target.as_str()) {
            return Err(ExecutorProtocolError::new(
                "Executor mount target is invalid",
            ));
        }
        match mount.origin {
            ExecutorMountOriginV0::Workspace => {
                workspace_mounts += 1;
                if mount.target != "/workspace" && !mount.target.starts_with("/workspace/") {
                    return Err(ExecutorProtocolError::new(
                        "Executor workspace mount target is outside /workspace",
                    ));
                }
            }
            ExecutorMountOriginV0::Runtime => {
                runtime_mounts += 1;
                if mount.access != ExecutorMountAccessV0::ReadOnly
                    || mount.target == "/workspace"
                    || mount.target.starts_with("/workspace/")
                {
                    return Err(ExecutorProtocolError::new(
                        "Executor runtime mount capability is invalid",
                    ));
                }
            }
        }
    }
    if workspace_mounts > MAX_EXECUTOR_WORKSPACE_MOUNTS_V0
        || runtime_mounts > MAX_EXECUTOR_RUNTIME_MOUNTS_V0
    {
        return Err(ExecutorProtocolError::new(
            "Executor mount provenance bounds are invalid",
        ));
    }
    validate_resolved_policy(request)?;
    validate_digest(&request.policy_digest, "policy_digest")?;
    if resolved_policy_digest_v0(&request.resolved_policy)? != request.policy_digest {
        return Err(ExecutorProtocolError::new(
            "Executor request policy digest does not match",
        ));
    }
    if request.limits.timeout_ms == 0
        || request.limits.max_stdout_bytes == 0
        || request.limits.max_stderr_bytes == 0
        || request.limits.max_concurrent_processes_and_threads == 0
    {
        return Err(ExecutorProtocolError::new(
            "Executor limits must be nonzero",
        ));
    }
    if request.limits.max_stdout_bytes > MAX_EXECUTOR_TOOL_STREAM_BYTES_V0 as u64
        || request.limits.max_stderr_bytes > MAX_EXECUTOR_TOOL_STREAM_BYTES_V0 as u64
    {
        return Err(ExecutorProtocolError::new(
            "Executor stream limits exceed the protocol bound",
        ));
    }
    Ok(())
}

fn validate_resolved_policy(request: &ExecutorRequestV0) -> Result<(), ExecutorProtocolError> {
    let policy = &request.resolved_policy;
    validate_text(&policy.tool_id, "resolved policy tool_id", MAX_ID_CHARS)?;
    validate_text(
        &policy.tool_kind,
        "resolved policy tool_kind",
        MAX_NAME_CHARS,
    )?;
    if !policy.artifact.is_object() || !policy.command.is_object() {
        return Err(ExecutorProtocolError::new(
            "Executor resolved policy artifacts must be objects",
        ));
    }
    if policy.tool_id != request.tool_id
        || policy.tool_kind != request.tool_kind
        || policy.runtime_profile != request.runtime_profile
        || policy.limits != request.limits
        || policy.mounts.len() != request.mounts.len()
    {
        return Err(ExecutorProtocolError::new(
            "Executor resolved policy does not match the request",
        ));
    }
    for (resolved, requested) in policy.mounts.iter().zip(&request.mounts) {
        validate_text(&resolved.source, "resolved mount source", MAX_PATH_CHARS)?;
        let source_is_valid = match resolved.origin {
            ExecutorMountOriginV0::Workspace => validate_workspace_source(&resolved.source).is_ok(),
            ExecutorMountOriginV0::Runtime => {
                validate_absolute_path(&resolved.source, "resolved runtime mount source").is_ok()
            }
        };
        if !source_is_valid
            || resolved.access != requested.access
            || resolved.descriptor != requested.descriptor
            || resolved.origin != requested.origin
            || resolved.source_identity != requested.source_identity
            || resolved.target != requested.target
        {
            return Err(ExecutorProtocolError::new(
                "Executor resolved mount does not match the inherited capability",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_receipt(
    receipt: &EnforcementReceiptV0,
) -> Result<(), ExecutorProtocolError> {
    validate_digest(&receipt.applied_policy_digest, "applied_policy_digest")?;
    for (name, value) in [
        ("executor", &receipt.executor),
        ("executor_version", &receipt.executor_version),
        ("backend", &receipt.backend),
        ("backend_version", &receipt.backend_version),
        ("platform", &receipt.platform),
    ] {
        validate_text(value, name, MAX_NAME_CHARS)?;
    }
    if receipt.max_concurrent_processes_and_threads == 0 {
        return Err(ExecutorProtocolError::new(
            "Executor receipt process capacity must be nonzero",
        ));
    }
    Ok(())
}

pub(super) fn validate_tool_result(
    result: &ExecutorToolResultV0,
) -> Result<(), ExecutorProtocolError> {
    use ExecutorToolClassificationV0 as Classification;
    use ExecutorToolStatusV0 as Status;

    let valid_terminal = match (result.status, result.classification, result.exit_code) {
        (Status::Completed, None, Some(0)) => true,
        (Status::Failed, Some(Classification::NonzeroExit), Some(code)) => code != 0,
        (Status::Failed, Some(Classification::SignalTermination), None) => true,
        (Status::Failed, Some(Classification::ProcessCapacityExceeded), _) => true,
        (
            Status::Failed,
            Some(
                Classification::StderrCapExceeded
                | Classification::StdoutCapExceeded
                | Classification::StdoutStderrCapExceeded
                | Classification::OutputCollectorFailed
                | Classification::OutputDrainTimeout,
            ),
            _,
        ) => true,
        (Status::TimedOut, Some(Classification::ToolTimedOut), None) => true,
        (Status::Cancelled, Some(Classification::Cancelled), None) => true,
        _ => false,
    };
    if !valid_terminal {
        return Err(ExecutorProtocolError::new(
            "Executor Tool result has an invalid terminal state",
        ));
    }
    for (name, encoded) in [
        ("stdout", &result.stdout_base64),
        ("stderr", &result.stderr_base64),
    ] {
        let decoded = decode_executor_stream_v0(encoded)?;
        if decoded.len() > MAX_EXECUTOR_TOOL_STREAM_BYTES_V0 {
            return Err(ExecutorProtocolError::new(format!(
                "Executor Tool {name} exceeds its byte limit"
            )));
        }
    }
    Ok(())
}

pub(super) fn validate_schema(
    actual: &str,
    expected: &str,
    kind: &str,
) -> Result<(), ExecutorProtocolError> {
    if actual == expected {
        Ok(())
    } else {
        Err(ExecutorProtocolError::new(format!(
            "unsupported Executor {kind} schema"
        )))
    }
}

fn validate_digest(value: &str, name: &str) -> Result<(), ExecutorProtocolError> {
    if decode_lowercase_sha256_hex(value).is_some() {
        Ok(())
    } else {
        Err(ExecutorProtocolError::new(format!(
            "Executor {name} is not lowercase SHA-256"
        )))
    }
}

pub(super) fn validate_text(
    value: &str,
    name: &str,
    max_chars: usize,
) -> Result<(), ExecutorProtocolError> {
    let count = value.chars().count();
    if count == 0 || count > max_chars || value.chars().any(char::is_control) {
        Err(ExecutorProtocolError::new(format!(
            "Executor {name} is invalid"
        )))
    } else {
        Ok(())
    }
}

pub(super) fn validate_absolute_path(value: &str, name: &str) -> Result<(), ExecutorProtocolError> {
    if value == "/"
        || !value.starts_with('/')
        || value.ends_with('/')
        || value.contains('\\')
        || value[1..]
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        Err(ExecutorProtocolError::new(format!(
            "Executor {name} is not a canonical absolute path"
        )))
    } else {
        Ok(())
    }
}

fn validate_workspace_source(value: &str) -> Result<(), ExecutorProtocolError> {
    if value == "workspace"
        || value.strip_prefix("workspace/").is_some_and(|relative| {
            !relative.is_empty()
                && !relative.contains('\\')
                && relative
                    .split('/')
                    .all(|component| !component.is_empty() && !matches!(component, "." | ".."))
        })
    {
        Ok(())
    } else {
        Err(ExecutorProtocolError::new(
            "Executor workspace mount source is not canonical",
        ))
    }
}
