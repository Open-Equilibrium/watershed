fn emit_tool(
    workspace: &Path,
    tool: &core_script::ToolBlock,
    policy: RuntimeToolPolicy<'_>,
    invocation: &LoopInvocation,
    side_effect_mode: ToolSideEffectMode,
    builder: &mut RuntimeEventBuilder<'_>,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    let planned_progress = tool_dispatch_progress(
        tool,
        policy.protected_path_match_mode,
        policy.command,
        ToolDispatchMode::Plan,
    )?;
    builder.emit(
        Some(invocation),
        EventType::ToolStarted,
        serde_json::json!({
            "allowed_parameters": policy.command.allowed_parameters.iter().map(|parameter| parameter.name.clone()).collect::<Vec<_>>(),
            "network_access": tool_network_access_name(&tool.network),
            "read_scope": policy.command.filesystem.read_roots,
            "tool_id": tool.identity.id,
            "tool_kind": policy_tool_kind_name(&policy.command.tool_kind),
            "tool_name": tool.identity.name,
            "write_scope": policy.command.filesystem.write_roots,
        }),
    )?;

    if let Some(failure) =
        sandbox_tool_dispatch_failure(tool, policy.stub_model_fixture_profile)?
    {
        return Ok(Some(failure));
    }

    let side_effect_sequence = builder.sequence + 1;
    let completed_sequence = side_effect_sequence + u64::from(planned_progress.is_some());
    let replay_guard_sequence = if planned_progress.is_some() {
        side_effect_sequence
    } else {
        completed_sequence
    };
    let progress = if side_effect_mode.should_execute_tool(replay_guard_sequence) {
        match tool_dispatch_progress(
            tool,
            policy.protected_path_match_mode,
            policy.command,
            ToolDispatchMode::Execute { workspace },
        ) {
            Ok(progress) => progress,
            Err(err) => {
                if matches!(side_effect_mode, ToolSideEffectMode::ApplyAll)
                    && let Some(failure) = runtime_failure_for_tool_error(&err, &tool.identity.id)
                {
                    return Ok(Some(failure));
                }
                return Err(err);
            }
        }
    } else if side_effect_mode.should_preflight_tool(replay_guard_sequence) {
        tool_dispatch_progress(
            tool,
            policy.protected_path_match_mode,
            policy.command,
            ToolDispatchMode::Preflight { workspace },
        )?
    } else {
        planned_progress
    };

    if let Some(message) = progress {
        emit_tool_progress(message, tool, invocation, builder)?;
    }

    builder.emit(
        Some(invocation),
        EventType::ToolCompleted,
        serde_json::json!({
            "exit_code": 0,
            "tool_id": tool.identity.id,
        }),
    )?;
    Ok(None)
}

enum ToolDispatchMode<'a> {
    Plan,
    Preflight {
        workspace: &'a Path,
    },
    Execute { workspace: &'a Path },
}

fn tool_dispatch_progress(
    tool: &core_script::ToolBlock,
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
    mode: ToolDispatchMode<'_>,
) -> Result<Option<&'static str>, RuntimeError> {
    match (&tool.tool_kind, &tool.command) {
        (
            core_script::ToolKind::PredefinedCommand,
            core_script::ToolCommand::Predefined { command_id, argv },
        ) => execute_predefined_command(policy, command_id, argv),
        (core_script::ToolKind::OwnScript, core_script::ToolCommand::OwnScript(_)) => {
            match mode {
                ToolDispatchMode::Plan => {
                    plan_own_script(tool, protected_path_match_mode, policy)?;
                }
                ToolDispatchMode::Preflight { workspace } => {
                    let write = plan_own_script(tool, protected_path_match_mode, policy)?;
                    preflight_own_script_outputs(
                        workspace,
                        write.as_ref(),
                        protected_path_match_mode,
                        policy,
                    )?;
                }
                ToolDispatchMode::Execute { workspace } => {
                    execute_own_script(workspace, tool, protected_path_match_mode, policy)?
                }
            }
            Ok(Some("stub write completed"))
        }
        _ => Err(RuntimeError::Protocol(format!(
            "tool command shape does not match {}",
            tool.identity.id
        ))),
    }
}

fn execute_predefined_command(
    policy: &core_policy::CommandPolicy,
    command_id: &str,
    argv: &[String],
) -> Result<Option<&'static str>, RuntimeError> {
    let progress = trusted_predefined_command_progress(command_id)?;
    let executable = format!("registry:{command_id}");
    if policy.executable != executable || policy.argv != argv {
        return Err(RuntimeError::Protocol(format!(
            "runtime policy executable does not match trusted command {command_id:?}"
        )));
    }
    Ok(progress)
}

fn trusted_predefined_command_progress(
    command_id: &str,
) -> Result<Option<&'static str>, RuntimeError> {
    if !core_policy::is_trusted_predefined_command_id(command_id) {
        return Err(RuntimeError::Protocol(format!(
            "unsupported predefined command {command_id:?}"
        )));
    }
    Ok((command_id == "agent-read").then_some("stub read completed"))
}
