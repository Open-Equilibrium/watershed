use super::validation::{
    validate_absolute_path, validate_receipt, validate_request, validate_schema, validate_text,
    validate_tool_result,
};
use super::{
    EXECUTOR_EXACT_EXECUTABLES_V0, EXECUTOR_PROBE_SCHEMA_V0, EXECUTOR_RESPONSE_SCHEMA_V0,
    EnforcementReceiptV0, ExecutorProbeV0, ExecutorProtocolError, ExecutorRequestV0,
    ExecutorResolvedPolicyV0, ExecutorResponseV0, MAX_ERROR_MESSAGE_CHARS,
    MAX_EXECUTOR_PROBE_BYTES_V0, MAX_EXECUTOR_REQUEST_BYTES_V0, MAX_EXECUTOR_RESPONSE_BYTES_V0,
    MAX_EXECUTOR_RUNTIME_MOUNTS_V0, MAX_FEATURES, MAX_ID_CHARS, MAX_NAME_CHARS, MAX_PATH_CHARS,
    RuntimeReadProfileV0,
};
use crate::{canonical_json, parse_unique_json};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Returns the lowercase SHA-256 of the canonical resolved target policy plus its required LF.
pub fn resolved_policy_digest_v0(
    policy: &ExecutorResolvedPolicyV0,
) -> Result<String, ExecutorProtocolError> {
    let value = serde_json::to_value(policy).map_err(|error| {
        ExecutorProtocolError::new(format!("invalid resolved Executor policy: {error}"))
    })?;
    let mut canonical = canonical_json(&value).map_err(|error| {
        ExecutorProtocolError::new(format!("invalid resolved Executor policy: {error}"))
    })?;
    canonical.push('\n');
    let digest = Sha256::digest(canonical.as_bytes());
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Validates one receipt against the policy, runtime profile, and capacity Flow requested.
pub fn validate_enforcement_receipt_v0(
    receipt: &EnforcementReceiptV0,
    expected_policy_digest: &str,
    expected_runtime_profile: RuntimeReadProfileV0,
    expected_max_concurrent_processes_and_threads: u32,
) -> Result<(), ExecutorProtocolError> {
    validate_receipt(receipt)?;
    if !receipt.isolation_active
        || receipt.applied_policy_digest != expected_policy_digest
        || receipt.runtime_profile != expected_runtime_profile
        || receipt.max_concurrent_processes_and_threads
            != expected_max_concurrent_processes_and_threads
    {
        return Err(ExecutorProtocolError::new(
            "Executor enforcement receipt does not match the requested isolation policy",
        ));
    }
    Ok(())
}

/// Serializes and validates one canonical Executor request plus LF.
pub fn canonical_executor_request_v0(
    request: &ExecutorRequestV0,
) -> Result<Vec<u8>, ExecutorProtocolError> {
    validate_request(request)?;
    let value = serde_json::to_value(request).map_err(|error| {
        ExecutorProtocolError::new(format!("invalid Executor request: {error}"))
    })?;
    let mut bytes = canonical_json(&value)
        .map_err(|error| ExecutorProtocolError::new(format!("invalid Executor request: {error}")))?
        .into_bytes();
    bytes.push(b'\n');
    if bytes.len() > MAX_EXECUTOR_REQUEST_BYTES_V0 {
        return Err(ExecutorProtocolError::new(
            "Executor request exceeds its byte limit",
        ));
    }
    Ok(bytes)
}

/// Serializes and validates one canonical Executor response plus LF.
pub fn canonical_executor_response_v0(
    response: &ExecutorResponseV0,
) -> Result<Vec<u8>, ExecutorProtocolError> {
    let (request_id, policy_digest) = match response {
        ExecutorResponseV0::Completed {
            request_id,
            enforcement,
            ..
        } => (
            request_id.as_str(),
            enforcement.applied_policy_digest.as_str(),
        ),
        ExecutorResponseV0::Error { request_id, .. } => (request_id.as_str(), ""),
    };
    let bytes = canonical_document(response, MAX_EXECUTOR_RESPONSE_BYTES_V0, "response")?;
    parse_executor_response_v0(&bytes, request_id, policy_digest)?;
    Ok(bytes)
}

/// Serializes and validates one canonical Executor probe plus LF.
pub fn canonical_executor_probe_v0(
    probe: &ExecutorProbeV0,
) -> Result<Vec<u8>, ExecutorProtocolError> {
    let bytes = canonical_document(probe, MAX_EXECUTOR_PROBE_BYTES_V0, "probe")?;
    parse_executor_probe_v0(&bytes)?;
    Ok(bytes)
}

/// Parses one exact canonical LF-terminated Executor request.
pub fn parse_executor_request_v0(bytes: &[u8]) -> Result<ExecutorRequestV0, ExecutorProtocolError> {
    let value = parse_canonical_document(bytes, MAX_EXECUTOR_REQUEST_BYTES_V0, "request")?;
    let request: ExecutorRequestV0 = serde_json::from_value(value).map_err(|error| {
        ExecutorProtocolError::new(format!("invalid Executor request: {error}"))
    })?;
    validate_request(&request)?;
    Ok(request)
}

/// Parses and validates one canonical Executor terminal response.
pub fn parse_executor_response_v0(
    bytes: &[u8],
    expected_request_id: &str,
    expected_policy_digest: &str,
) -> Result<ExecutorResponseV0, ExecutorProtocolError> {
    let value = parse_canonical_document(bytes, MAX_EXECUTOR_RESPONSE_BYTES_V0, "response")?;
    let response: ExecutorResponseV0 = serde_json::from_value(value).map_err(|error| {
        ExecutorProtocolError::new(format!("invalid Executor response: {error}"))
    })?;
    let (schema, request_id) = match &response {
        ExecutorResponseV0::Completed {
            schema, request_id, ..
        }
        | ExecutorResponseV0::Error {
            schema, request_id, ..
        } => (schema, request_id),
    };
    validate_schema(schema, EXECUTOR_RESPONSE_SCHEMA_V0, "response")?;
    validate_text(request_id, "request_id", MAX_ID_CHARS)?;
    if request_id != expected_request_id {
        return Err(ExecutorProtocolError::new(
            "Executor response request id does not match",
        ));
    }
    match &response {
        ExecutorResponseV0::Completed {
            enforcement,
            tool_result,
            ..
        } => {
            validate_receipt(enforcement)?;
            validate_tool_result(tool_result)?;
            if enforcement.applied_policy_digest != expected_policy_digest {
                return Err(ExecutorProtocolError::new(
                    "Executor applied the wrong policy digest",
                ));
            }
            if !enforcement.isolation_active {
                return Err(ExecutorProtocolError::new(
                    "Executor isolation was not active",
                ));
            }
        }
        ExecutorResponseV0::Error { message, .. } => {
            validate_text(message, "error message", MAX_ERROR_MESSAGE_CHARS)?;
        }
    }
    Ok(response)
}

/// Parses and validates one canonical Executor readiness response.
pub fn parse_executor_probe_v0(bytes: &[u8]) -> Result<ExecutorProbeV0, ExecutorProtocolError> {
    let value = parse_canonical_document(bytes, MAX_EXECUTOR_PROBE_BYTES_V0, "probe")?;
    let probe: ExecutorProbeV0 = serde_json::from_value(value)
        .map_err(|error| ExecutorProtocolError::new(format!("invalid Executor probe: {error}")))?;
    validate_schema(&probe.schema, EXECUTOR_PROBE_SCHEMA_V0, "probe")?;
    for (name, value) in [
        ("executor", &probe.executor),
        ("executor_version", &probe.executor_version),
        ("backend", &probe.backend),
        ("backend_version", &probe.backend_version),
        ("platform", &probe.platform),
    ] {
        validate_text(value, name, MAX_NAME_CHARS)?;
    }
    if probe.protocol_versions.is_empty()
        || probe.protocol_versions.len() > MAX_FEATURES
        || probe.supported_policy_features.len() > MAX_FEATURES
        || probe.runtime_mounts.len() > MAX_EXECUTOR_RUNTIME_MOUNTS_V0
    {
        return Err(ExecutorProtocolError::new(
            "Executor probe list bounds are invalid",
        ));
    }
    for value in probe
        .protocol_versions
        .iter()
        .chain(&probe.supported_policy_features)
    {
        validate_text(value, "probe feature", MAX_NAME_CHARS)?;
    }
    let mut mount_keys = std::collections::BTreeSet::new();
    let mut mount_targets = std::collections::BTreeSet::new();
    for mount in &probe.runtime_mounts {
        validate_text(&mount.source, "runtime mount source", MAX_PATH_CHARS)?;
        validate_text(&mount.target, "runtime mount target", MAX_PATH_CHARS)?;
        validate_absolute_path(&mount.source, "runtime mount source")?;
        validate_absolute_path(&mount.target, "runtime mount target")?;
        if let Some(executable) = &mount.executable {
            validate_text(executable, "runtime mount executable", MAX_PATH_CHARS)?;
            validate_absolute_path(executable, "runtime mount executable")?;
            if !EXECUTOR_EXACT_EXECUTABLES_V0.contains(&executable.as_str()) {
                return Err(ExecutorProtocolError::new(
                    "Executor runtime mount names an unsupported executable",
                ));
            }
        } else if mount.runtime_profile == RuntimeReadProfileV0::Exact {
            return Err(ExecutorProtocolError::new(
                "exact Executor runtime mounts must name an executable",
            ));
        }
        let key = (
            mount.runtime_profile as u8,
            mount.executable.as_deref(),
            mount.source.as_str(),
        );
        let target_key = (
            mount.runtime_profile as u8,
            mount.executable.as_deref(),
            mount.target.as_str(),
        );
        if !mount_keys.insert(key) || !mount_targets.insert(target_key) {
            return Err(ExecutorProtocolError::new(
                "Executor runtime mount manifest contains duplicates",
            ));
        }
    }
    Ok(probe)
}

fn parse_canonical_document(
    bytes: &[u8],
    limit: usize,
    kind: &str,
) -> Result<serde_json::Value, ExecutorProtocolError> {
    if bytes.len() > limit {
        return Err(ExecutorProtocolError::new(format!(
            "Executor {kind} exceeds its byte limit"
        )));
    }
    if !bytes.ends_with(b"\n") || bytes[..bytes.len().saturating_sub(1)].contains(&b'\n') {
        return Err(ExecutorProtocolError::new(format!(
            "Executor {kind} must be one LF-terminated canonical JSON document"
        )));
    }
    let body = std::str::from_utf8(&bytes[..bytes.len() - 1])
        .map_err(|_| ExecutorProtocolError::new(format!("Executor {kind} is not UTF-8")))?;
    let value = parse_unique_json(body)
        .map_err(|error| ExecutorProtocolError::new(format!("invalid Executor {kind}: {error}")))?;
    let canonical = canonical_json(&value)
        .map_err(|error| ExecutorProtocolError::new(format!("invalid Executor {kind}: {error}")))?;
    if canonical.as_bytes() != body.as_bytes() {
        return Err(ExecutorProtocolError::new(format!(
            "Executor {kind} is not canonical JSON"
        )));
    }
    Ok(value)
}

fn canonical_document<T: Serialize>(
    document: &T,
    limit: usize,
    kind: &str,
) -> Result<Vec<u8>, ExecutorProtocolError> {
    let value = serde_json::to_value(document)
        .map_err(|error| ExecutorProtocolError::new(format!("invalid Executor {kind}: {error}")))?;
    let mut bytes = canonical_json(&value)
        .map_err(|error| ExecutorProtocolError::new(format!("invalid Executor {kind}: {error}")))?
        .into_bytes();
    bytes.push(b'\n');
    if bytes.len() > limit {
        return Err(ExecutorProtocolError::new(format!(
            "Executor {kind} exceeds its byte limit"
        )));
    }
    Ok(bytes)
}
