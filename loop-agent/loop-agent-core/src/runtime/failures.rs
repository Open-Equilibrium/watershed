fn emit_runtime_failure(
    loop_block: &core_script::LoopBlock,
    invocation: &LoopInvocation,
    failure: &RuntimeFailure,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    if failure.emit_tool_failed {
        emit_runtime_tool_failure(invocation, failure, builder)?;
    }
    emit_runtime_error(invocation, failure, builder)?;
    emit_runtime_loop_failure(loop_block, invocation, &failure.reason, builder)
}

fn emit_runtime_error(
    invocation: &LoopInvocation,
    failure: &RuntimeFailure,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    let mut error_payload = serde_json::json!({
        "code": failure.reason,
        "message": failure.message,
    });
    if let Some(phase_id) = &failure.phase_id {
        let mut error_data = serde_json::Map::new();
        error_data.insert("phase_id".to_owned(), serde_json::json!(phase_id));
        if let Some(tool_id) = &failure.tool_id {
            error_data.insert("tool_id".to_owned(), serde_json::json!(tool_id));
        }
        let object = error_payload
            .as_object_mut()
            .expect("error payload is constructed as an object");
        object.insert("data".to_owned(), serde_json::Value::Object(error_data));
    }
    builder.emit(Some(invocation), EventType::Error, error_payload)
}

fn emit_runtime_loop_failure(
    loop_block: &core_script::LoopBlock,
    invocation: &LoopInvocation,
    reason: &str,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    builder.emit(
        Some(invocation),
        EventType::LoopFailed,
        serde_json::json!({
            "error": reason,
            "loop_definition_id": loop_block.identity.id,
        }),
    )
}

fn emit_runtime_tool_failure(
    invocation: &LoopInvocation,
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

fn emit_propagated_runtime_failure(
    loop_block: &core_script::LoopBlock,
    invocation: &LoopInvocation,
    failure: &RuntimeFailure,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    emit_runtime_loop_failure(loop_block, invocation, &failure.reason, builder)
}

fn emit_runtime_error_failure(
    loop_block: &core_script::LoopBlock,
    invocation: &LoopInvocation,
    err: &RuntimeError,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    let failure = runtime_failure_for_unhandled_error(err);
    emit_runtime_error(invocation, &failure, builder)?;
    emit_runtime_loop_failure(loop_block, invocation, &failure.reason, builder)
}

fn emit_propagated_runtime_error_failure(
    loop_block: &core_script::LoopBlock,
    invocation: &LoopInvocation,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    emit_runtime_loop_failure(loop_block, invocation, RUNTIME_ERROR_REASON, builder)
}

fn sandbox_tool_dispatch_failure(
    tool: &core_script::ToolBlock,
    target: &core_policy::PolicyTarget,
    command_policy: &core_policy::CommandPolicy,
    stub_model_fixture_profile: bool,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    ensure_tool_matches_policy(tool, target, command_policy)?;
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
        .tools
        .values()
        .filter(|tool| {
            is_sandbox_negative_sentinel_tool(tool)
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

fn is_sandbox_negative_sentinel_tool(tool: &core_script::ToolBlock) -> bool {
    sandbox_negative_operation_for_tool(tool).is_some()
}

fn sandbox_negative_operation_for_tool(tool: &core_script::ToolBlock) -> Option<&str> {
    let (
        core_script::ToolKind::PredefinedCommand,
        core_script::ToolCommand::Predefined { command_id, argv },
    ) = (&tool.tool_kind, &tool.command)
    else {
        return None;
    };
    if command_id != "agent-negative" {
        return None;
    }
    let [operation] = argv.as_slice() else {
        return None;
    };
    sandbox_negative_reason_for_operation(operation).map(|_| operation.as_str())
}

fn sandbox_negative_reason_for_tool(
    tool: &core_script::ToolBlock,
) -> Result<Option<core_policy::DenyReasonCode>, RuntimeError> {
    let (
        core_script::ToolKind::PredefinedCommand,
        core_script::ToolCommand::Predefined { command_id, argv },
    ) = (&tool.tool_kind, &tool.command)
    else {
        return Ok(None);
    };
    if command_id != "agent-negative" {
        return Ok(None);
    }
    let [operation] = argv.as_slice() else {
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
        tool_id,
        phase_id: None,
        emit_tool_failed,
    }
}

fn runtime_failure_for_unhandled_error(err: &RuntimeError) -> RuntimeFailure {
    let (reason, message) = match err {
        RuntimeError::ContextBudgetExceeded { .. } => {
            ("context_budget_exceeded", "mandatory context exceeds the model input budget")
        }
        _ => (RUNTIME_ERROR_REASON, runtime_error_message(err)),
    };
    RuntimeFailure {
        reason: reason.to_owned(),
        message,
        tool_id: None,
        phase_id: None,
        emit_tool_failed: false,
    }
}

fn runtime_error_message(_err: &RuntimeError) -> &'static str {
    "runtime execution failed"
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
        tool_id: Some(tool_id),
        phase_id: Some(phase_id),
        emit_tool_failed: false,
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

fn session_id_for_loop(loop_id: &str) -> String {
    if matches!(loop_id, "smoke-loop" | "hello-loop") {
        let base = loop_id
            .strip_suffix("-loop")
            .expect("fixture loop id ends with -loop");
        return session_id_from_token(base, loop_id);
    }
    if let Some(operation) = loop_id.strip_prefix("sandbox-negative-") {
        return session_id_from_token(&sandbox_negative_session_token(operation), loop_id);
    }
    session_id_from_token(loop_id, loop_id)
}

fn sandbox_negative_session_token(operation: &str) -> String {
    let mut token = String::from("neg");
    for word in operation.split('-') {
        match word {
            "environment" => token.push_str("env"),
            "interpreter" => token.push_str("interp"),
            "network" => token.push_str("net"),
            "path" | "symlink" | "write" => token.push_str(word),
            "phase" => token.push_str("phase"),
            "of" | "out" | "protected" | "tool" => {}
            other => token.push_str(other),
        }
    }
    token
}

fn session_id_from_token(token: &str, stable_source: &str) -> String {
    let mut token = token.to_ascii_lowercase();
    token.retain(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-');
    if token.is_empty() {
        token.push_str("session");
    }
    let suffix = if token.len() <= 125 {
        "001".to_owned()
    } else {
        format!("-{:016x}001", stable_hash64(stable_source.as_bytes()))
    };
    token.truncate(128 - suffix.len());
    token.push_str(&suffix);
    token
}

fn stable_hash64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn tool_kind_name(kind: &core_script::ToolKind) -> &'static str {
    match kind {
        core_script::ToolKind::PredefinedCommand => "predefined-command",
        core_script::ToolKind::OwnScript => "own-script",
    }
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
