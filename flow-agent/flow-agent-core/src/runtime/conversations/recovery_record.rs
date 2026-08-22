use super::contract::{
    MAX_CONVERSATION_RECORD_BYTES, protocol, validate_attempt_id, validate_hash, validate_id,
    validate_timestamp,
};
use crate::runtime::{run_attempts::RunAttemptOutcome, types::RuntimeError};
use serde::{Deserialize, Serialize};

pub(super) const PRODUCTIVE_RECOVERY_SCHEMA_V0: &str = "flow-productive-recovery-v0";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "record_type", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum ProductiveRecoveryRecord {
    Header {
        schema: String,
        conversation_id: String,
        run_session_id: String,
        flow_definition_id: String,
        registry_hash: String,
        flow_definition_hash: String,
        root_input: serde_json::Value,
        parent_entry_id: Option<String>,
        event_clock_base_unix_seconds: i64,
        prior_history_object: String,
        prior_event_count: u64,
    },
    Provider {
        schema: String,
        attempt_id: String,
        request_hash: String,
        outcome: RunAttemptOutcome,
        classification: Option<String>,
        exit_code: Option<i32>,
        timestamp: String,
        durable_output: serde_json::Value,
    },
    Tool {
        schema: String,
        attempt_id: String,
        request_hash: String,
        tool_id: String,
        outcome: RunAttemptOutcome,
        classification: Option<String>,
        exit_code: Option<i32>,
        timestamp: String,
        durable_output: serde_json::Value,
    },
    Phase {
        schema: String,
        flow_execution_id: String,
        phase_execution_id: String,
        phase_id: String,
        iteration: u32,
        result_object: String,
        will_repeat: bool,
    },
    Transition {
        schema: String,
        flow_execution_id: String,
        from_phase_id: String,
        to_phase_id: Option<String>,
    },
    Flow {
        schema: String,
        flow_execution_id: String,
        result_object: String,
    },
    Terminal {
        schema: String,
        failed: bool,
        history_object: String,
        cumulative_event_count: u64,
    },
}

pub(super) fn parse_productive_recovery_records(
    bytes: &[u8],
) -> Result<Vec<ProductiveRecoveryRecord>, RuntimeError> {
    if bytes.is_empty() || !bytes.ends_with(b"\n") || bytes.contains(&b'\r') {
        return Err(protocol(
            "productive recovery snapshot must use non-empty LF-framed JSONL",
        ));
    }
    let mut records = Vec::new();
    for raw in bytes[..bytes.len() - 1].split(|byte| *byte == b'\n') {
        let record = parse_productive_recovery_record(raw)?;
        if matches!(&record, ProductiveRecoveryRecord::Header { .. }) && !records.is_empty() {
            return Err(protocol(
                "productive recovery snapshot contains more than one header",
            ));
        }
        if records
            .iter()
            .any(|record| matches!(record, ProductiveRecoveryRecord::Terminal { .. }))
        {
            return Err(protocol("productive recovery terminal record must be last"));
        }
        records.push(record);
    }
    Ok(records)
}

pub(super) fn parse_productive_recovery_record(
    raw: &[u8],
) -> Result<ProductiveRecoveryRecord, RuntimeError> {
    if raw.is_empty() || raw.len() > MAX_CONVERSATION_RECORD_BYTES || raw.contains(&b'\r') {
        return Err(protocol("productive recovery record has invalid framing"));
    }
    let value: serde_json::Value = serde_json::from_slice(raw).map_err(RuntimeError::Json)?;
    let canonical = proto::canonical_json(&value)
        .map_err(|error| protocol(format!("productive recovery record is invalid: {error}")))?;
    if canonical.as_bytes() != raw {
        return Err(protocol(
            "productive recovery record must be canonical JSON",
        ));
    }
    let record = serde_json::from_value(value).map_err(RuntimeError::Json)?;
    validate_productive_recovery_record(&record)?;
    Ok(record)
}

pub(super) fn validate_productive_recovery_record(
    record: &ProductiveRecoveryRecord,
) -> Result<(), RuntimeError> {
    let schema = match record {
        ProductiveRecoveryRecord::Header {
            schema,
            conversation_id,
            run_session_id,
            flow_definition_id,
            registry_hash,
            flow_definition_hash,
            root_input,
            parent_entry_id,
            prior_history_object,
            ..
        } => {
            validate_id(conversation_id, "recovery conversation")?;
            validate_id(run_session_id, "recovery run session")?;
            if !core_script::is_valid_block_id(flow_definition_id) {
                return Err(protocol("recovery Flow definition id is invalid"));
            }
            validate_hash(registry_hash, "recovery registry hash")?;
            validate_hash(flow_definition_hash, "recovery Flow definition hash")?;
            if let Some(parent_entry_id) = parent_entry_id {
                validate_id(parent_entry_id, "recovery parent entry")?;
            }
            if !root_input.is_null() {
                core_script::parse_flow_value_v0(root_input.clone()).map_err(|error| {
                    protocol(format!("recovery root input is invalid: {error}"))
                })?;
            }
            validate_recovery_object_uri(prior_history_object)?;
            schema
        }
        ProductiveRecoveryRecord::Provider {
            schema,
            attempt_id,
            request_hash,
            timestamp,
            ..
        } => {
            validate_attempt_id(attempt_id)?;
            validate_hash(request_hash, "provider request hash")?;
            validate_timestamp(timestamp)?;
            schema
        }
        ProductiveRecoveryRecord::Tool {
            schema,
            attempt_id,
            request_hash,
            tool_id,
            timestamp,
            ..
        } => {
            validate_attempt_id(attempt_id)?;
            validate_hash(request_hash, "Tool request hash")?;
            validate_timestamp(timestamp)?;
            if !core_script::is_valid_block_id(tool_id) {
                return Err(protocol("recovery Tool id is invalid"));
            }
            schema
        }
        ProductiveRecoveryRecord::Phase {
            schema,
            flow_execution_id,
            phase_execution_id,
            phase_id,
            iteration,
            result_object,
            ..
        } => {
            validate_id(flow_execution_id, "recovery Flow execution")?;
            validate_id(phase_execution_id, "recovery Phase execution")?;
            if !core_script::is_valid_block_id(phase_id) || *iteration == 0 {
                return Err(protocol("recovery Phase boundary is invalid"));
            }
            validate_recovery_object_uri(result_object)?;
            schema
        }
        ProductiveRecoveryRecord::Transition {
            schema,
            flow_execution_id,
            from_phase_id,
            to_phase_id,
        } => {
            validate_id(flow_execution_id, "recovery Flow execution")?;
            if !core_script::is_valid_block_id(from_phase_id)
                || to_phase_id
                    .as_deref()
                    .is_some_and(|phase_id| !core_script::is_valid_block_id(phase_id))
            {
                return Err(protocol("recovery Transition boundary is invalid"));
            }
            schema
        }
        ProductiveRecoveryRecord::Flow {
            schema,
            flow_execution_id,
            result_object,
            ..
        } => {
            validate_id(flow_execution_id, "recovery Flow execution")?;
            validate_recovery_object_uri(result_object)?;
            schema
        }
        ProductiveRecoveryRecord::Terminal {
            schema,
            history_object,
            ..
        } => {
            validate_recovery_object_uri(history_object)?;
            schema
        }
    };
    if schema != PRODUCTIVE_RECOVERY_SCHEMA_V0 {
        return Err(protocol(
            "productive recovery record has an unsupported schema",
        ));
    }
    Ok(())
}

pub(super) fn validate_recovery_object_uri(uri: &str) -> Result<&str, RuntimeError> {
    core_script::parse_session_object_uri(uri)
        .map_err(|_| protocol("productive recovery object URI is invalid"))
}
