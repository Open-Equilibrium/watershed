use super::{PROVIDER_ERROR_SCHEMA_V0, PROVIDER_OUTPUT_SCHEMA_V1, PROVIDER_OUTPUT_SCHEMA_V2};
use crate::runtime::{
    context::ContextObject,
    digest::sha256_hex,
    openai_codex::{ProviderTokenUsage, ProviderToolCall, ProviderTurn},
    run_attempts::ProductiveRecovery,
    types::{MAX_PROVIDER_ERROR_MESSAGE_CHARS, RuntimeError},
};
use proto::parse_unique_json;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub(crate) const MAX_ACCUMULATED_PROVIDER_INPUT_BYTES: usize = 64 * 1024 * 1024;

pub(crate) struct ProviderInput {
    canonical_bytes: usize,
    items: Vec<serde_json::Value>,
}

impl ProviderInput {
    pub(crate) fn new() -> Self {
        Self {
            canonical_bytes: 2,
            items: Vec::new(),
        }
    }

    pub(crate) fn items(&self) -> &[serde_json::Value] {
        &self.items
    }

    pub(crate) fn push(&mut self, item: serde_json::Value) -> Result<(), RuntimeError> {
        let canonical = proto::canonical_json(&item).map_err(|error| {
            RuntimeError::Protocol(format!("provider input serialization failed: {error}"))
        })?;
        let candidate = self
            .canonical_bytes
            .checked_add(usize::from(!self.items.is_empty()))
            .and_then(|bytes| bytes.checked_add(canonical.len()))
            .ok_or_else(|| {
                RuntimeError::Protocol(
                    "accumulated provider input byte count overflowed".to_owned(),
                )
            })?;
        if candidate > MAX_ACCUMULATED_PROVIDER_INPUT_BYTES {
            return Err(RuntimeError::Protocol(format!(
                "accumulated provider input is {candidate} bytes; maximum is {MAX_ACCUMULATED_PROVIDER_INPUT_BYTES}"
            )));
        }
        self.items.push(item);
        self.canonical_bytes = candidate;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn canonical_bytes(&self) -> usize {
        self.canonical_bytes
    }
}

pub(crate) struct DurableProviderOutput {
    pub(crate) objects: Vec<ContextObject>,
    pub(crate) reference: serde_json::Value,
}

pub(crate) const MAX_DURABLE_PROVIDER_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderTurnSnapshot {
    response_id: String,
    output_text: String,
    retained_items: Vec<serde_json::Value>,
    tool_calls: Vec<ProviderToolCallSnapshot>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderToolCallSnapshot {
    call_id: String,
    name: String,
    arguments: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProviderOutputReference {
    schema: String,
    provider_output_objects: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token_usage: Option<ProviderTokenUsage>,
}

pub(crate) fn durable_provider_output(
    turn: &ProviderTurn,
) -> Result<DurableProviderOutput, RuntimeError> {
    let snapshot = ProviderTurnSnapshot {
        response_id: turn.response_id.clone(),
        output_text: turn.output_text.clone(),
        retained_items: turn.retained_items.clone(),
        tool_calls: turn
            .tool_calls
            .iter()
            .map(|call| ProviderToolCallSnapshot {
                call_id: call.call_id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
            })
            .collect(),
    };
    let bytes = proto::canonical_json(&serde_json::to_value(snapshot).map_err(RuntimeError::Json)?)
        .map_err(|error| {
            RuntimeError::Protocol(format!("provider output serialization failed: {error}"))
        })?
        .into_bytes();
    if bytes.len() > MAX_DURABLE_PROVIDER_OUTPUT_BYTES {
        return Err(RuntimeError::Protocol(
            "provider output exceeds its durable recovery byte limit".to_owned(),
        ));
    }
    let mut objects = Vec::new();
    let mut uris = Vec::new();
    let chunk_size =
        usize::try_from(crate::runtime::types::MAX_SESSION_OBJECT_BYTES).unwrap_or(usize::MAX);
    for chunk in bytes.chunks(chunk_size) {
        let digest = sha256_hex(chunk);
        uris.push(
            core_script::build_session_object_uri(&digest).map_err(|error| {
                RuntimeError::Protocol(format!("provider output object URI is invalid: {error}"))
            })?,
        );
        objects.push(ContextObject {
            bytes: chunk.to_vec(),
            digest,
        });
    }
    Ok(DurableProviderOutput {
        objects,
        reference: serde_json::to_value(ProviderOutputReference {
            schema: PROVIDER_OUTPUT_SCHEMA_V2.to_owned(),
            provider_output_objects: uris,
            token_usage: turn.token_usage.clone(),
        })
        .map_err(RuntimeError::Json)?,
    })
}

pub(crate) fn provider_turn_from_durable_output(
    durable_output: &serde_json::Value,
    recovery: &dyn ProductiveRecovery,
) -> Result<ProviderTurn, RuntimeError> {
    let reference: ProviderOutputReference =
        serde_json::from_value(durable_output.clone()).map_err(RuntimeError::Json)?;
    let token_usage = match reference.schema.as_str() {
        PROVIDER_OUTPUT_SCHEMA_V1 if reference.token_usage.is_none() => None,
        PROVIDER_OUTPUT_SCHEMA_V2 => reference.token_usage,
        _ => {
            return Err(RuntimeError::Protocol(
                "recovered provider output has an unsupported schema".to_owned(),
            ));
        }
    };
    if reference.provider_output_objects.is_empty() {
        return Err(RuntimeError::Protocol(
            "recovered provider output has an unsupported schema".to_owned(),
        ));
    }
    let chunk_size =
        usize::try_from(crate::runtime::types::MAX_SESSION_OBJECT_BYTES).unwrap_or(usize::MAX);
    let max_objects = MAX_DURABLE_PROVIDER_OUTPUT_BYTES
        .checked_add(chunk_size.saturating_sub(1))
        .map(|bytes| bytes / chunk_size)
        .unwrap_or(usize::MAX);
    if reference.provider_output_objects.len() > max_objects {
        return Err(RuntimeError::Protocol(
            "recovered provider output has too many objects".to_owned(),
        ));
    }
    let mut bytes = Vec::new();
    for uri in reference.provider_output_objects {
        let chunk =
            read_verified_session_object(recovery, &uri, "recovered provider output object")?;
        let next = bytes.len().checked_add(chunk.len()).ok_or_else(|| {
            RuntimeError::Protocol("recovered provider output byte count overflow".to_owned())
        })?;
        if next > MAX_DURABLE_PROVIDER_OUTPUT_BYTES {
            return Err(RuntimeError::Protocol(
                "recovered provider output exceeds its byte limit".to_owned(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(RuntimeError::Json)?;
    let canonical = proto::canonical_json(&value).map_err(|error| {
        RuntimeError::Protocol(format!("recovered provider output is invalid: {error}"))
    })?;
    if canonical.as_bytes() != bytes {
        return Err(RuntimeError::Protocol(
            "recovered provider output must be canonical JSON".to_owned(),
        ));
    }
    let snapshot: ProviderTurnSnapshot =
        serde_json::from_value(value).map_err(RuntimeError::Json)?;
    Ok(ProviderTurn {
        response_id: snapshot.response_id,
        output_text: snapshot.output_text,
        retained_items: snapshot.retained_items,
        tool_calls: snapshot
            .tool_calls
            .into_iter()
            .map(|call| ProviderToolCall {
                call_id: call.call_id,
                name: call.name,
                arguments: call.arguments,
            })
            .collect(),
        token_usage,
    })
}

pub(crate) fn parse_provider_result(
    phase: &core_script::PhaseBlock,
    output_text: &str,
) -> Result<core_script::FlowValue, RuntimeError> {
    let json = parse_unique_json(output_text).map_err(|error| {
        RuntimeError::Protocol(format!(
            "Phase {} provider result is not duplicate-free JSON: {error}",
            phase.identity.id
        ))
    })?;
    let result = core_script::parse_flow_value_v0(json).map_err(|error| {
        RuntimeError::Protocol(format!(
            "Phase {} provider result is invalid: {error}",
            phase.identity.id
        ))
    })?;
    Ok(result)
}

pub(crate) fn verify_provider_result_session_objects(
    value: &core_script::FlowValue,
    recovery: &dyn ProductiveRecovery,
) -> Result<(), RuntimeError> {
    verify_provider_result_session_objects_at(value, recovery, &mut BTreeSet::new())
}

fn verify_provider_result_session_objects_at(
    value: &core_script::FlowValue,
    recovery: &dyn ProductiveRecovery,
    verified: &mut BTreeSet<String>,
) -> Result<(), RuntimeError> {
    match value {
        core_script::FlowValue::Boolean(_)
        | core_script::FlowValue::Integer(_)
        | core_script::FlowValue::String(_) => Ok(()),
        core_script::FlowValue::List(values) => {
            for value in values {
                verify_provider_result_session_objects_at(value, recovery, verified)?;
            }
            Ok(())
        }
        core_script::FlowValue::Map(values) => {
            for value in values.values() {
                verify_provider_result_session_objects_at(value, recovery, verified)?;
            }
            Ok(())
        }
        core_script::FlowValue::SessionObject(uri) => {
            if !verified.insert(uri.clone()) {
                return Ok(());
            }
            read_verified_session_object(recovery, uri, "provider result session object")?;
            Ok(())
        }
    }
}

pub(super) fn read_verified_session_object(
    recovery: &dyn ProductiveRecovery,
    uri: &str,
    description: &str,
) -> Result<Vec<u8>, RuntimeError> {
    let bytes = recovery.read_object(uri)?;
    let expected_uri =
        core_script::build_session_object_uri(&sha256_hex(&bytes)).map_err(|error| {
            RuntimeError::Protocol(format!("{description} URI is invalid: {error}"))
        })?;
    if expected_uri != uri {
        return Err(RuntimeError::Protocol(format!(
            "{description} does not match its URI digest"
        )));
    }
    Ok(bytes)
}

pub(super) fn durable_provider_error(
    error: &RuntimeError,
) -> Result<serde_json::Value, RuntimeError> {
    let failure = error.provider_failure().ok_or_else(|| {
        RuntimeError::Protocol("provider error durability received a non-provider error".to_owned())
    })?;
    let mut value = serde_json::json!({
        "message": failure.message(),
        "schema": PROVIDER_ERROR_SCHEMA_V0,
    });
    if let Some(status) = failure.http_status() {
        value
            .as_object_mut()
            .expect("provider error is an object")
            .insert("http_status".to_owned(), serde_json::json!(status));
    }
    Ok(value)
}

pub(super) fn provider_error_from_durable_output(
    value: &serde_json::Value,
) -> Result<RuntimeError, RuntimeError> {
    let object = value.as_object().ok_or_else(|| {
        RuntimeError::Protocol("recovered provider error is not an object".to_owned())
    })?;
    let expected_fields = if object.contains_key("http_status") {
        3
    } else {
        2
    };
    if object.len() != expected_fields
        || object.get("schema").and_then(serde_json::Value::as_str)
            != Some(PROVIDER_ERROR_SCHEMA_V0)
    {
        return Err(RuntimeError::Protocol(
            "recovered provider error has an invalid schema".to_owned(),
        ));
    }
    let message = object
        .get("message")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            RuntimeError::Protocol("recovered provider error lacks a message".to_owned())
        })?;
    if message.chars().count() > MAX_PROVIDER_ERROR_MESSAGE_CHARS {
        return Err(RuntimeError::Protocol(
            "recovered provider error message exceeds its character budget".to_owned(),
        ));
    }
    let http_status = object
        .get("http_status")
        .map(|value| {
            value
                .as_u64()
                .and_then(|status| u16::try_from(status).ok())
                .ok_or_else(|| {
                    RuntimeError::Protocol(
                        "recovered provider error HTTP status is invalid".to_owned(),
                    )
                })
        })
        .transpose()?;
    Ok(RuntimeError::definitive_provider_error(
        http_status,
        message,
    ))
}
