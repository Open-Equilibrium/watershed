use super::codec::resolved_policy_digest_v0;
use super::stream::decode_executor_stream_v0;
use super::{
    EXECUTOR_PROTECTED_DESCRIPTOR_BASE_V0, EXECUTOR_REQUEST_SCHEMA_V0, EnforcementReceiptV0,
    ExecutorExecVectorErrorV0, ExecutorProtocolError, ExecutorRequestV0,
    ExecutorToolClassificationV0, ExecutorToolResultV0, ExecutorToolStatusV0,
    MAX_ENVIRONMENT_ENTRIES, MAX_EXECUTOR_PROTECTED_OBJECTS_V0, MAX_EXECUTOR_TOOL_STREAM_BYTES_V0,
    MAX_ID_CHARS, MAX_NAME_CHARS, MAX_PATH_CHARS, validate_executor_exec_vector_v0,
};
use crate::session_object::decode_lowercase_sha256_hex;

pub(super) fn validate_request(request: &ExecutorRequestV0) -> Result<(), ExecutorProtocolError> {
    validate_schema(&request.schema, EXECUTOR_REQUEST_SCHEMA_V0, "request")?;
    validate_text(&request.request_id, "request_id", MAX_ID_CHARS)?;
    let policy = &request.resolved_policy;
    validate_text(&policy.tool_id, "tool_id", MAX_ID_CHARS)?;
    validate_text(&policy.tool_kind, "tool_kind", MAX_NAME_CHARS)?;
    validate_text(&policy.executable, "executable", MAX_PATH_CHARS)?;
    validate_absolute_path(&policy.executable, "executable")?;
    validate_text(
        &policy.working_directory,
        "working_directory",
        MAX_PATH_CHARS,
    )?;
    validate_absolute_path(&policy.working_directory, "working_directory")?;
    if policy.environment.len() > MAX_ENVIRONMENT_ENTRIES {
        return Err(ExecutorProtocolError::new(
            "Executor environment has too many entries",
        ));
    }
    for (name, value) in &policy.environment {
        validate_text(name, "environment name", MAX_NAME_CHARS)?;
        validate_text(value, "environment value", MAX_PATH_CHARS)?;
    }
    if let Err(error) =
        validate_executor_exec_vector_v0(&policy.executable, &policy.argv, &policy.environment)
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
    if policy.protected_objects.is_empty() {
        return Err(ExecutorProtocolError::new(
            "Executor protected object list must be nonempty",
        ));
    }
    if policy.protected_objects.len() > MAX_EXECUTOR_PROTECTED_OBJECTS_V0 {
        return Err(ExecutorProtocolError::new(
            "Executor protected object list exceeds its limit",
        ));
    }
    let mut paths = std::collections::BTreeSet::new();
    for (index, object) in policy.protected_objects.iter().enumerate() {
        let expected_descriptor = EXECUTOR_PROTECTED_DESCRIPTOR_BASE_V0
            .checked_add(u32::try_from(index).expect("protected object limit fits u32"))
            .expect("protected descriptor range is bounded");
        if object.descriptor != expected_descriptor {
            return Err(ExecutorProtocolError::new(
                "Executor protected object descriptor is invalid",
            ));
        }
        validate_text(&object.path, "protected object path", MAX_PATH_CHARS)?;
        validate_absolute_path(&object.path, "protected object path")?;
        if !paths.insert(object.path.as_str()) {
            return Err(ExecutorProtocolError::new(
                "Executor protected object path is duplicated",
            ));
        }
    }
    validate_digest(&request.policy_digest, "policy_digest")?;
    if resolved_policy_digest_v0(&request.resolved_policy)? != request.policy_digest {
        return Err(ExecutorProtocolError::new(
            "Executor request policy digest does not match",
        ));
    }
    if policy.limits.timeout_ms == 0
        || policy.limits.max_stdout_bytes == 0
        || policy.limits.max_stderr_bytes == 0
    {
        return Err(ExecutorProtocolError::new(
            "Executor limits must be nonzero",
        ));
    }
    if policy.limits.max_stdout_bytes > MAX_EXECUTOR_TOOL_STREAM_BYTES_V0 as u64
        || policy.limits.max_stderr_bytes > MAX_EXECUTOR_TOOL_STREAM_BYTES_V0 as u64
    {
        return Err(ExecutorProtocolError::new(
            "Executor stream limits exceed the protocol bound",
        ));
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
