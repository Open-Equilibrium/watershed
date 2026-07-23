fn emit_runtime_failure(
    flow_block: &core_script::FlowBlock,
    invocation: &FlowInvocation,
    failure: &RuntimeFailure,
    builder: &mut RuntimeEventBuilder<'_>,
) -> Result<(), RuntimeError> {
    if failure.emit_tool_failed {
        emit_runtime_tool_failure(invocation, failure, builder)?;
    }
    emit_runtime_error(invocation, failure, builder)?;
    emit_runtime_flow_failure(flow_block, invocation, &failure.reason, builder)
}

fn emit_runtime_error(
    invocation: &FlowInvocation,
    failure: &RuntimeFailure,
    builder: &mut RuntimeEventBuilder<'_>,
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

fn emit_runtime_flow_failure(
    flow_block: &core_script::FlowBlock,
    invocation: &FlowInvocation,
    reason: &str,
    builder: &mut RuntimeEventBuilder<'_>,
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

fn emit_runtime_tool_failure(
    invocation: &FlowInvocation,
    failure: &RuntimeFailure,
    builder: &mut RuntimeEventBuilder<'_>,
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

fn emit_runtime_error_failure(
    flow_block: &core_script::FlowBlock,
    invocation: &FlowInvocation,
    err: &RuntimeError,
    builder: &mut RuntimeEventBuilder<'_>,
) -> Result<(), RuntimeError> {
    let failure = runtime_failure_for_unhandled_error(err);
    complete_active_step(invocation, builder)?;
    emit_runtime_error(invocation, &failure, builder)?;
    emit_runtime_flow_failure(flow_block, invocation, &failure.reason, builder)
}

fn complete_active_step(
    invocation: &FlowInvocation,
    builder: &mut RuntimeEventBuilder<'_>,
) -> Result<(), RuntimeError> {
    let payload = builder
        .active_step_payloads
        .get(&invocation.flow_id)
        .cloned();
    match payload {
        Some(payload) => builder.emit(Some(invocation), EventType::StepCompleted, payload),
        None => Ok(()),
    }
}

fn sandbox_tool_dispatch_failure(
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

fn sandbox_out_of_phase_failure(
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    phase: &core_script::PhaseBlock,
    stub_model_fixture_profile: bool,
) -> Option<RuntimeFailure> {
    if !stub_model_fixture_profile
        || !phase.tool_refs.is_empty()
        || !phase.identity.id.starts_with("negative-")
        || !phase.identity.id.contains("no-tools")
    {
        return None;
    }
    let unavailable_sentinel = registry
        .tool_blocks()
        .filter(|tool| {
            sandbox_negative_operation_for_tool(tool).is_some()
                && !policy_phase_contains_tool(policy, &phase.identity.id, &tool.identity.id)
        })
        .min_by_key(|tool| {
            if sandbox_negative_operation_for_tool(tool) == Some("write") {
                0
            } else {
                1
            }
        })?;
    Some(runtime_out_of_phase_failure(
        phase.identity.id.clone(),
        unavailable_sentinel.identity.id.clone(),
    ))
}

fn sandbox_negative_operation_for_tool(tool: &core_script::ToolBlock) -> Option<&str> {
    let [operation] = sandbox_negative_arguments(tool)? else {
        return None;
    };
    sandbox_negative_reason_for_operation(operation).map(|_| operation.as_str())
}

fn sandbox_negative_reason_for_tool(
    tool: &core_script::ToolBlock,
) -> Result<Option<core_policy::DenyReasonCode>, RuntimeError> {
    let Some(arguments) = sandbox_negative_arguments(tool) else {
        return Ok(None);
    };
    let [operation] = arguments else {
        return Err(RuntimeError::Protocol(format!(
            "tool {} agent-negative command must declare one denied operation",
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

fn sandbox_negative_arguments(tool: &core_script::ToolBlock) -> Option<&[String]> {
    match (&tool.tool_kind, &tool.command) {
        (
            core_script::ToolKind::PredefinedCommand,
            core_script::ToolCommand::Predefined { command_id, argv },
        ) if command_id == "agent-negative" => Some(argv),
        _ => None,
    }
}

fn sandbox_negative_reason_for_operation(operation: &str) -> Option<core_policy::DenyReasonCode> {
    match operation {
        "environment" => Some(core_policy::DenyReasonCode::EnvironmentDenied),
        "interpreter" => Some(core_policy::DenyReasonCode::InterpreterEscapeDenied),
        "network" => Some(core_policy::DenyReasonCode::NetworkDenied),
        "protected-path" => Some(core_policy::DenyReasonCode::ProtectedPathDenied),
        "symlink" => Some(core_policy::DenyReasonCode::SymlinkEscapeDenied),
        "write" => Some(core_policy::DenyReasonCode::WriteDenied),
        _ => None,
    }
}

fn runtime_denied(reason: core_policy::DenyReasonCode, message: String) -> RuntimeError {
    RuntimeError::Denied { reason, message }
}

fn runtime_protocol_or_denied(
    denied_reason: Option<core_policy::DenyReasonCode>,
    message: String,
) -> RuntimeError {
    match denied_reason {
        Some(reason) => runtime_denied(reason, message),
        None => RuntimeError::Protocol(message),
    }
}

fn runtime_failure_for_reason(
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

fn runtime_failure_for_unhandled_error(err: &RuntimeError) -> RuntimeFailure {
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

fn runtime_failure_for_tool_error(err: &RuntimeError, tool_id: &str) -> Option<RuntimeFailure> {
    let reason = match err {
        RuntimeError::Denied { reason, .. } => reason.clone(),
        RuntimeError::Io { source, .. } if source.kind() == io::ErrorKind::PermissionDenied => {
            core_policy::DenyReasonCode::WriteDenied
        }
        RuntimeError::Io { .. } => return None,
        RuntimeError::Json(_)
        | RuntimeError::Policy(_)
        | RuntimeError::Registry(_)
        | RuntimeError::Protocol(_)
        | RuntimeError::ContextBudgetExceeded { .. }
        | RuntimeError::EventWriter(_)
        | RuntimeError::SessionFailed { .. }
        | RuntimeError::ActiveSession { .. }
        | RuntimeError::SessionLogExists(_)
        | RuntimeError::TerminalSession(_)
        | RuntimeError::Usage(_) => {
            return None;
        }
    };
    Some(runtime_failure_for_reason(reason, Some(tool_id.to_owned())))
}

fn runtime_out_of_phase_failure(phase_id: String, tool_id: String) -> RuntimeFailure {
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

fn runtime_io_error_kind(kind: io::ErrorKind) -> &'static str {
    match kind {
        io::ErrorKind::NotFound => "not_found",
        io::ErrorKind::PermissionDenied => "permission_denied",
        io::ErrorKind::AlreadyExists => "already_exists",
        io::ErrorKind::InvalidInput => "invalid_input",
        io::ErrorKind::InvalidData => "invalid_data",
        io::ErrorKind::TimedOut => "timed_out",
        io::ErrorKind::WriteZero => "write_zero",
        io::ErrorKind::StorageFull => "storage_full",
        io::ErrorKind::ReadOnlyFilesystem => "read_only_filesystem",
        io::ErrorKind::FileTooLarge => "file_too_large",
        io::ErrorKind::ResourceBusy => "resource_busy",
        io::ErrorKind::Interrupted => "interrupted",
        io::ErrorKind::UnexpectedEof => "unexpected_eof",
        io::ErrorKind::OutOfMemory => "out_of_memory",
        _ => "other",
    }
}

fn policy_phase_contains_tool(
    policy: &core_policy::PolicyArtifact,
    phase_id: &str,
    tool_id: &str,
) -> bool {
    policy
        .phase_scope
        .iter()
        .any(|phase| phase.phase_id == phase_id && phase.tool_ids.iter().any(|id| id == tool_id))
}

fn denial_message(reason: core_policy::DenyReasonCode) -> &'static str {
    match reason {
        core_policy::DenyReasonCode::WriteDenied => "write outside declared roots denied",
        core_policy::DenyReasonCode::NetworkDenied => "network egress denied by default",
        core_policy::DenyReasonCode::EnvironmentDenied => "secret environment read denied",
        core_policy::DenyReasonCode::ToolOutOfPhase => "tool is not available in the active phase",
        core_policy::DenyReasonCode::ProtectedPathDenied => "protected path access denied",
        core_policy::DenyReasonCode::SymlinkEscapeDenied => "symlink escape denied",
        core_policy::DenyReasonCode::InterpreterEscapeDenied => "interpreter escape denied",
    }
}

fn canonical_event_stream(events: &[EventEnvelope]) -> Result<String, RuntimeError> {
    let mut stream = String::new();
    for event in events {
        stream.push_str(&event.canonical_jsonl().map_err(|err| {
            RuntimeError::Protocol(format!("failed to serialize runtime event: {err}"))
        })?);
    }
    Ok(stream)
}

fn policy_tool_kind_name(kind: &core_policy::ToolKind) -> &'static str {
    match kind {
        core_policy::ToolKind::PredefinedCommand => "predefined-command",
        core_policy::ToolKind::OwnScript => "own-script",
    }
}

fn tool_network_access_name(policy: &core_script::NetworkPolicy) -> &'static str {
    match policy {
        core_script::NetworkPolicy::Deny(_) => "deny",
        core_script::NetworkPolicy::Declared { .. } => "declared",
    }
}

fn connection_kind_name(kind: &core_script::ConnectionKind) -> &'static str {
    match kind {
        core_script::ConnectionKind::Data => "data",
        core_script::ConnectionKind::Trigger => "trigger",
        core_script::ConnectionKind::Refresh => "refresh",
    }
}
