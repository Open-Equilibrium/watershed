use super::validation::{
    validate_preflight, validate_probe, validate_receipt, validate_request, validate_response,
    validate_start,
};
use super::{
    EnforcementReceiptV0, ExecutorPreflightV0, ExecutorProbeV0, ExecutorProtocolError,
    ExecutorRequestV0, ExecutorResponseV0, ExecutorStartV0, MAX_EXECUTOR_CONTROL_BYTES_V0,
    MAX_EXECUTOR_PROBE_BYTES_V0, MAX_EXECUTOR_REQUEST_BYTES_V0, MAX_EXECUTOR_RESPONSE_BYTES_V0,
};
use crate::{
    canonical::{nfc_json_string_values, nfc_string},
    canonical_json, parse_unique_json,
};
use serde::{Serialize, de::DeserializeOwned};

/// Validates active self-protection and the digest of the complete policy Flow requested.
pub fn validate_enforcement_receipt_v0(
    receipt: &EnforcementReceiptV0,
    expected_policy_digest: &str,
) -> Result<(), ExecutorProtocolError> {
    validate_receipt(receipt, expected_policy_digest)
}

/// Serializes and validates one canonical Executor request plus LF.
pub fn canonical_executor_request_v0(
    request: &ExecutorRequestV0,
) -> Result<Vec<u8>, ExecutorProtocolError> {
    validate_request(request)?;
    let (bytes, value) = canonical_document(request, MAX_EXECUTOR_REQUEST_BYTES_V0, "request")?;
    let mut normalized: ExecutorRequestV0 =
        decode_document(nfc_json_string_values(value), "request")?;
    // Canonical serialization already rejects colliding normalized environment names.
    normalized.resolved_policy.environment = normalized
        .resolved_policy
        .environment
        .into_iter()
        .map(|(name, value)| (nfc_string(name), value))
        .collect();
    validate_request(&normalized)?;
    Ok(bytes)
}

/// Serializes and validates one canonical Executor preflight response plus LF.
pub fn canonical_executor_preflight_v0(
    preflight: &ExecutorPreflightV0,
) -> Result<Vec<u8>, ExecutorProtocolError> {
    let request_id = match preflight {
        ExecutorPreflightV0::Ready { request_id, .. }
        | ExecutorPreflightV0::Error { request_id, .. } => request_id,
    };
    let (bytes, value) = canonical_document(preflight, MAX_EXECUTOR_CONTROL_BYTES_V0, "preflight")?;
    let preflight = decode_document(nfc_json_string_values(value), "preflight")?;
    validate_preflight(&preflight, request_id)?;
    Ok(bytes)
}

/// Serializes and validates one canonical Executor start record plus LF.
pub fn canonical_executor_start_v0(
    start: &ExecutorStartV0,
) -> Result<Vec<u8>, ExecutorProtocolError> {
    let (bytes, value) = canonical_document(start, MAX_EXECUTOR_CONTROL_BYTES_V0, "start")?;
    let normalized = decode_document(nfc_json_string_values(value), "start")?;
    validate_start(&normalized, &start.request_id)?;
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
    let (bytes, value) = canonical_document(response, MAX_EXECUTOR_RESPONSE_BYTES_V0, "response")?;
    let response = decode_document(nfc_json_string_values(value), "response")?;
    validate_response(&response, request_id, policy_digest)?;
    Ok(bytes)
}

/// Serializes and validates one canonical Executor probe plus LF.
pub fn canonical_executor_probe_v0(
    probe: &ExecutorProbeV0,
) -> Result<Vec<u8>, ExecutorProtocolError> {
    let (bytes, value) = canonical_document(probe, MAX_EXECUTOR_PROBE_BYTES_V0, "probe")?;
    let probe = decode_document(nfc_json_string_values(value), "probe")?;
    validate_probe(&probe)?;
    Ok(bytes)
}

/// Parses one exact canonical LF-terminated Executor request.
pub fn parse_executor_request_v0(bytes: &[u8]) -> Result<ExecutorRequestV0, ExecutorProtocolError> {
    let value = parse_canonical_document(bytes, MAX_EXECUTOR_REQUEST_BYTES_V0, "request")?;
    let request = decode_document(value, "request")?;
    validate_request(&request)?;
    Ok(request)
}

/// Parses one exact canonical Executor preflight response for the expected request.
pub fn parse_executor_preflight_v0(
    bytes: &[u8],
    expected_request_id: &str,
) -> Result<ExecutorPreflightV0, ExecutorProtocolError> {
    let value = parse_canonical_document(bytes, MAX_EXECUTOR_CONTROL_BYTES_V0, "preflight")?;
    let preflight = decode_document(value, "preflight")?;
    validate_preflight(&preflight, expected_request_id)?;
    Ok(preflight)
}

/// Parses one exact canonical Executor start record for the expected request.
pub fn parse_executor_start_v0(
    bytes: &[u8],
    expected_request_id: &str,
) -> Result<ExecutorStartV0, ExecutorProtocolError> {
    let value = parse_canonical_document(bytes, MAX_EXECUTOR_CONTROL_BYTES_V0, "start")?;
    let start = decode_document(value, "start")?;
    validate_start(&start, expected_request_id)?;
    Ok(start)
}

/// Parses and validates one canonical Executor terminal response.
pub fn parse_executor_response_v0(
    bytes: &[u8],
    expected_request_id: &str,
    expected_policy_digest: &str,
) -> Result<ExecutorResponseV0, ExecutorProtocolError> {
    let value = parse_canonical_document(bytes, MAX_EXECUTOR_RESPONSE_BYTES_V0, "response")?;
    let response = decode_document(value, "response")?;
    validate_response(&response, expected_request_id, expected_policy_digest)?;
    Ok(response)
}

/// Parses and validates one canonical Executor readiness response.
pub fn parse_executor_probe_v0(bytes: &[u8]) -> Result<ExecutorProbeV0, ExecutorProtocolError> {
    let value = parse_canonical_document(bytes, MAX_EXECUTOR_PROBE_BYTES_V0, "probe")?;
    let probe = decode_document(value, "probe")?;
    validate_probe(&probe)?;
    Ok(probe)
}

fn decode_document<T: DeserializeOwned>(
    value: serde_json::Value,
    kind: &str,
) -> Result<T, ExecutorProtocolError> {
    serde_json::from_value(value)
        .map_err(|error| ExecutorProtocolError::new(format!("invalid Executor {kind}: {error}")))
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
) -> Result<(Vec<u8>, serde_json::Value), ExecutorProtocolError> {
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
    Ok((bytes, value))
}
