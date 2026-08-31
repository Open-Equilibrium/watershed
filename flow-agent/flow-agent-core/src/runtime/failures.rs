use crate::runtime::{
    event_construction::RuntimeEventBuilder,
    execution_plan::RuntimeFailure,
    stream_signature::FlowInvocation,
    types::{RUNTIME_ERROR_REASON, RuntimeError},
};
#[cfg(test)]
use proto::EventEnvelope;
use proto::EventType;
use std::{io, path::PathBuf};

pub(crate) const RUNTIME_IO_ERROR_KINDS: [(io::ErrorKind, &str); 15] = [
    (io::ErrorKind::NotFound, "not_found"),
    (io::ErrorKind::PermissionDenied, "permission_denied"),
    (io::ErrorKind::AlreadyExists, "already_exists"),
    (io::ErrorKind::InvalidInput, "invalid_input"),
    (io::ErrorKind::InvalidData, "invalid_data"),
    (io::ErrorKind::TimedOut, "timed_out"),
    (io::ErrorKind::WriteZero, "write_zero"),
    (io::ErrorKind::StorageFull, "storage_full"),
    (io::ErrorKind::ReadOnlyFilesystem, "read_only_filesystem"),
    (io::ErrorKind::FileTooLarge, "file_too_large"),
    (io::ErrorKind::ResourceBusy, "resource_busy"),
    (io::ErrorKind::Interrupted, "interrupted"),
    (io::ErrorKind::UnexpectedEof, "unexpected_eof"),
    (io::ErrorKind::OutOfMemory, "out_of_memory"),
    (io::ErrorKind::Other, "other"),
];

pub fn emit_runtime_failure(
    flow_block: &core_script::FlowBlock,
    invocation: &FlowInvocation,
    failure: &RuntimeFailure,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    if failure.emit_tool_failed {
        emit_runtime_tool_failure(invocation, failure, builder)?;
    }
    emit_runtime_error(invocation, failure, builder)?;
    emit_runtime_flow_failure(flow_block, invocation, &failure.reason, builder)
}

pub fn emit_runtime_error(
    invocation: &FlowInvocation,
    failure: &RuntimeFailure,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    let mut error_payload = serde_json::json!({
        "code": failure.reason,
        "message": failure.message,
    });
    let mut error_data = failure.data.clone();
    if let Some(phase_id) = &failure.phase_id {
        error_data.insert("phase_id".to_owned(), serde_json::json!(phase_id));
        if let Some(tool_id) = &failure.tool_id {
            error_data.insert("tool_id".to_owned(), serde_json::json!(tool_id));
        }
    }
    if !error_data.is_empty() {
        let object = error_payload
            .as_object_mut()
            .expect("error payload is constructed as an object");
        object.insert("data".to_owned(), serde_json::Value::Object(error_data));
    }
    builder.emit(Some(invocation), EventType::Error, error_payload)
}

pub fn emit_runtime_flow_failure(
    flow_block: &core_script::FlowBlock,
    invocation: &FlowInvocation,
    reason: &str,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    builder.emit(
        Some(invocation),
        EventType::FlowFailed,
        serde_json::json!({
            "error": reason,
            "flow_definition_id": flow_block.identity.id,
        }),
    )
}

pub fn emit_runtime_tool_failure(
    invocation: &FlowInvocation,
    failure: &RuntimeFailure,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    if let Some(tool_id) = &failure.tool_id {
        builder.emit(
            Some(invocation),
            EventType::ToolFailed,
            serde_json::json!({
                "error": failure.reason,
                "tool_id": tool_id,
            }),
        )?;
    }
    Ok(())
}

pub fn emit_runtime_error_failure(
    flow_block: &core_script::FlowBlock,
    invocation: &FlowInvocation,
    err: &RuntimeError,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    let failure = runtime_failure_for_unhandled_error(err);
    complete_active_phase(invocation, builder, &failure.reason)?;
    emit_runtime_error(invocation, &failure, builder)?;
    emit_runtime_flow_failure(flow_block, invocation, &failure.reason, builder)
}

pub fn complete_active_phase(
    invocation: &FlowInvocation,
    builder: &mut RuntimeEventBuilder,
    reason: &str,
) -> Result<(), RuntimeError> {
    let payload = builder
        .active_phase_payloads
        .get(&invocation.flow_id)
        .and_then(|phases| phases.last())
        .map(|entered| {
            serde_json::json!({
                "error": reason,
                "iteration": entered.get("iteration").cloned().unwrap_or_default(),
                "phase_execution_id": entered.get("phase_execution_id").cloned().unwrap_or_default(),
                "phase_id": entered.get("phase_id").cloned().unwrap_or_default(),
                "phase_kind": entered.get("phase_kind").cloned().unwrap_or_default(),
            })
        });
    match payload {
        Some(payload) => builder.emit(Some(invocation), EventType::PhaseFailed, payload),
        None => Ok(()),
    }
}

pub fn sandbox_tool_dispatch_failure(
    tool: &core_script::ToolBlock,
    stub_model_fixture_profile: bool,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    if !stub_model_fixture_profile {
        return Ok(None);
    }
    let Some(reason_code) = sandbox_negative_reason_for_tool(tool)? else {
        return Ok(None);
    };
    Ok(Some(runtime_failure_for_reason(
        reason_code,
        Some(tool.identity.id.clone()),
    )))
}

pub fn sandbox_out_of_phase_failure(
    _registry: &core_script::ResolvedRegistry,
    _policy: &core_policy::PolicyArtifact,
    phase: &core_script::PhaseBlock,
    stub_model_fixture_profile: bool,
) -> Option<RuntimeFailure> {
    if !stub_model_fixture_profile
        || !phase.tool_refs.is_empty()
        || !phase.identity.id.starts_with("negative-")
        || !phase.identity.id.contains("no-tools")
        || !phase
            .instruction_refs
            .iter()
            .any(|instruction_ref| instruction_ref == "deny-attempt")
    {
        return None;
    }
    Some(runtime_out_of_phase_failure(
        phase.identity.id.clone(),
        "negative-tool".to_owned(),
    ))
}

pub fn sandbox_negative_reason_for_tool(
    tool: &core_script::ToolBlock,
) -> Result<Option<core_policy::DenyReasonCode>, RuntimeError> {
    let Some(arguments) = sandbox_negative_arguments(tool) else {
        return Ok(None);
    };
    let [operation] = arguments else {
        return Err(RuntimeError::Protocol(format!(
            "tool {} negative fixture command must declare one denied operation",
            tool.identity.id
        )));
    };
    sandbox_negative_reason_for_operation(operation)
        .map(Some)
        .ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "tool {} declares unsupported sandbox-negative operation {operation:?}",
                tool.identity.id
            ))
        })
}

pub fn sandbox_negative_arguments(tool: &core_script::ToolBlock) -> Option<&[String]> {
    match (&tool.tool_kind, &tool.command) {
        (
            core_script::ToolKind::PredefinedCommand,
            core_script::ToolCommand::Predefined { command_id, argv },
        ) if core_policy::TrustedPredefinedCommand::parse(command_id)
            == Some(core_policy::TrustedPredefinedCommand::Negative) =>
        {
            Some(argv)
        }
        _ => None,
    }
}

pub fn sandbox_negative_reason_for_operation(
    operation: &str,
) -> Option<core_policy::DenyReasonCode> {
    match operation {
        "environment" => Some(core_policy::DenyReasonCode::EnvironmentDenied),
        "interpreter" => Some(core_policy::DenyReasonCode::InterpreterEscapeDenied),
        "network" => Some(core_policy::DenyReasonCode::NetworkDenied),
        "symlink" => Some(core_policy::DenyReasonCode::SymlinkEscapeDenied),
        "write" => Some(core_policy::DenyReasonCode::WriteDenied),
        _ => None,
    }
}

pub fn runtime_failure_for_reason(
    reason_code: core_policy::DenyReasonCode,
    tool_id: Option<String>,
) -> RuntimeFailure {
    let emit_tool_failed = tool_id.is_some();
    RuntimeFailure {
        reason: reason_code.as_str().to_owned(),
        message: denial_message(reason_code),
        data: serde_json::Map::new(),
        tool_id,
        phase_id: None,
        emit_tool_failed,
    }
}

pub fn runtime_failure_for_unhandled_error(err: &RuntimeError) -> RuntimeFailure {
    let (reason, message, data) = match err {
        RuntimeError::ContextBudgetExceeded {
            input_budget_tokens,
            required_bytes,
        } => (
            "context_budget_exceeded",
            "mandatory context exceeds the model input budget",
            serde_json::Map::from_iter([
                (
                    "input_budget_tokens".to_owned(),
                    (*input_budget_tokens).into(),
                ),
                ("required_bytes".to_owned(), (*required_bytes).into()),
            ]),
        ),
        RuntimeError::Io { source, .. } => (
            RUNTIME_ERROR_REASON,
            "runtime execution failed",
            serde_json::Map::from_iter([(
                "io_kind".to_owned(),
                serde_json::json!(runtime_io_error_kind(source.kind())),
            )]),
        ),
        RuntimeError::Denied { reason, .. } => (
            reason.as_str(),
            denial_message(reason.clone()),
            serde_json::Map::new(),
        ),
        _ => (
            RUNTIME_ERROR_REASON,
            "runtime execution failed",
            serde_json::Map::new(),
        ),
    };
    RuntimeFailure {
        reason: reason.to_owned(),
        message,
        data,
        tool_id: None,
        phase_id: None,
        emit_tool_failed: false,
    }
}

pub(crate) fn fixture_failure_capacity_candidates() -> Vec<RuntimeFailure> {
    let mut candidates = core_policy::DenyReasonCode::ALL
        .into_iter()
        .map(|reason| runtime_failure_for_reason(reason, None))
        .collect::<Vec<_>>();
    candidates.push(runtime_failure_for_unhandled_error(
        &RuntimeError::ContextBudgetExceeded {
            input_budget_tokens: usize::MAX,
            required_bytes: usize::MAX,
        },
    ));
    candidates.push(runtime_failure_for_unhandled_error(
        &RuntimeError::Protocol("capacity probe".to_owned()),
    ));
    candidates.extend(RUNTIME_IO_ERROR_KINDS.iter().map(|(kind, _)| {
        runtime_failure_for_unhandled_error(&RuntimeError::Io {
            path: PathBuf::from("capacity-probe"),
            source: io::Error::from(*kind),
        })
    }));
    candidates
}

pub fn runtime_failure_for_tool_error(err: &RuntimeError, tool_id: &str) -> Option<RuntimeFailure> {
    let reason = match err {
        RuntimeError::Denied { reason, .. } => reason.clone(),
        RuntimeError::Io { .. } => return None,
        RuntimeError::Json(_)
        | RuntimeError::Executor(_)
        | RuntimeError::Policy(_)
        | RuntimeError::Registry(_)
        | RuntimeError::Protocol(_)
        | RuntimeError::PersistedState(_)
        | RuntimeError::GlobalConfigAlreadyInitialized { .. }
        | RuntimeError::DefinitionExists { .. }
        | RuntimeError::InvalidDefinition { .. }
        | RuntimeError::InvalidReference { .. }
        | RuntimeError::ContextBudgetExceeded { .. }
        | RuntimeError::ReplayOutputLimitExceeded { .. }
        | RuntimeError::ExecutionBackendUnavailable
        | RuntimeError::ProductiveExecutionUnavailable
        | RuntimeError::Provider(_)
        | RuntimeError::Cancelled
        | RuntimeError::EventWriter(_)
        | RuntimeError::EventWriterFailures(_)
        | RuntimeError::TemporaryReplacementFailures { .. }
        | RuntimeError::PublishedOutputCleanupFailure { .. }
        | RuntimeError::PublishedOutputFinalizationFailure { .. }
        | RuntimeError::PublishedCredentialFinalizationFailure { .. }
        | RuntimeError::ControlledStageFailures { .. }
        | RuntimeError::SessionCleanupFailures(_)
        | RuntimeError::SessionFailed { .. }
        | RuntimeError::ActiveSession { .. }
        | RuntimeError::SessionLogExists(_)
        | RuntimeError::TerminalSession(_)
        | RuntimeError::Usage(_)
        | RuntimeError::AuthenticationRequired(_) => {
            return None;
        }
    };
    Some(runtime_failure_for_reason(reason, Some(tool_id.to_owned())))
}

pub fn runtime_out_of_phase_failure(phase_id: String, tool_id: String) -> RuntimeFailure {
    RuntimeFailure {
        reason: core_policy::DenyReasonCode::ToolOutOfPhase
            .as_str()
            .to_owned(),
        message: denial_message(core_policy::DenyReasonCode::ToolOutOfPhase),
        data: serde_json::Map::new(),
        tool_id: Some(tool_id),
        phase_id: Some(phase_id),
        emit_tool_failed: false,
    }
}

pub fn runtime_io_error_kind(kind: io::ErrorKind) -> &'static str {
    RUNTIME_IO_ERROR_KINDS
        .iter()
        .find_map(|(candidate, name)| (*candidate == kind).then_some(*name))
        .unwrap_or("other")
}

pub fn denial_message(reason: core_policy::DenyReasonCode) -> &'static str {
    match reason {
        core_policy::DenyReasonCode::WriteDenied => "write outside declared roots denied",
        core_policy::DenyReasonCode::NetworkDenied => "network egress denied by default",
        core_policy::DenyReasonCode::EnvironmentDenied => "environment read denied",
        core_policy::DenyReasonCode::ToolOutOfPhase => "tool is not available in the active phase",
        core_policy::DenyReasonCode::SymlinkEscapeDenied => "symlink escape denied",
        core_policy::DenyReasonCode::InterpreterEscapeDenied => "interpreter escape denied",
    }
}

#[cfg(test)]
pub fn canonical_event_stream(events: &[EventEnvelope]) -> Result<String, RuntimeError> {
    let mut stream = String::new();
    for event in events {
        stream.push_str(&event.canonical_jsonl().map_err(|err| {
            RuntimeError::Protocol(format!("failed to serialize runtime event: {err}"))
        })?);
    }
    Ok(stream)
}
