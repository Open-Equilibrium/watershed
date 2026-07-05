struct RuntimeExecution {
    events: Vec<EventEnvelope>,
    failed: bool,
    terminal_error: Option<RuntimeError>,
}

#[derive(Clone, Debug)]
struct LoopInvocation {
    loop_id: String,
    parent_loop_id: Option<String>,
}

struct RuntimeFailure {
    reason: String,
    message: &'static str,
    tool_id: Option<String>,
    phase_id: Option<String>,
    emit_tool_failed: bool,
}

#[derive(Clone, Copy)]
struct RuntimeToolPolicy<'a> {
    command: &'a core_policy::CommandPolicy,
    protected_path_match_mode: ProtectedPathMatchMode,
    stub_model_fixture_profile: bool,
    target: &'a core_policy::PolicyTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolSideEffectMode {
    ApplyAll,
    DryRun,
    PreflightResume { prefix_event_count: u64 },
    Resume { prefix_event_count: u64 },
}

impl ToolSideEffectMode {
    fn should_execute_tool(self, completed_sequence: u64) -> bool {
        match self {
            Self::ApplyAll => true,
            Self::DryRun => false,
            Self::PreflightResume { .. } => false,
            Self::Resume { prefix_event_count } => completed_sequence > prefix_event_count,
        }
    }

    fn should_preflight_tool(self, completed_sequence: u64) -> bool {
        match self {
            Self::PreflightResume { prefix_event_count } => completed_sequence > prefix_event_count,
            Self::ApplyAll | Self::DryRun | Self::Resume { .. } => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct SideEffectRecorder<'a> {
    reservation: Option<&'a SessionReservation>,
}

impl<'a> SideEffectRecorder<'a> {
    fn none() -> Self {
        Self { reservation: None }
    }

    fn for_reservation(reservation: &'a SessionReservation) -> Self {
        Self {
            reservation: Some(reservation),
        }
    }

    fn mark_applied(self) {
        if let Some(reservation) = self.reservation {
            reservation.mark_side_effects_applied();
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct LoopExecutionOptions<'a> {
    clock: EventClock,
    side_effect_mode: ToolSideEffectMode,
    side_effect_recorder: SideEffectRecorder<'a>,
    stub_model_fixture_profile: bool,
}

impl<'a> LoopExecutionOptions<'a> {
    #[cfg(test)]
    fn new(
        clock: EventClock,
        side_effect_mode: ToolSideEffectMode,
        side_effect_recorder: SideEffectRecorder<'a>,
    ) -> Self {
        Self {
            clock,
            side_effect_mode,
            side_effect_recorder,
            stub_model_fixture_profile: true,
        }
    }

    fn with_stub_model_fixture_profile(
        clock: EventClock,
        side_effect_mode: ToolSideEffectMode,
        side_effect_recorder: SideEffectRecorder<'a>,
        stub_model_fixture_profile: bool,
    ) -> Self {
        Self {
            clock,
            side_effect_mode,
            side_effect_recorder,
            stub_model_fixture_profile,
        }
    }
}

fn runtime_policy_artifact(
    artifacts: &[core_policy::PolicyArtifact],
) -> Result<&core_policy::PolicyArtifact, RuntimeError> {
    let target = runtime_policy_target();
    runtime_policy_artifact_for_target(artifacts, &target)
}

#[cfg(target_os = "macos")]
fn runtime_policy_target() -> core_policy::PolicyTarget {
    core_policy::PolicyTarget::MacosSeatbelt
}

#[cfg(not(target_os = "macos"))]
fn runtime_policy_target() -> core_policy::PolicyTarget {
    core_policy::PolicyTarget::LinuxLandlockSeccomp
}

#[cfg(windows)]
fn runtime_protected_path_match_mode(target: &core_policy::PolicyTarget) -> ProtectedPathMatchMode {
    let _policy_mode = protected_path_match_mode_for_policy_target(target);
    ProtectedPathMatchMode::CaseInsensitive
}

#[cfg(not(windows))]
fn runtime_protected_path_match_mode(target: &core_policy::PolicyTarget) -> ProtectedPathMatchMode {
    protected_path_match_mode_for_policy_target(target)
}

fn runtime_policy_artifact_for_target<'a>(
    artifacts: &'a [core_policy::PolicyArtifact],
    target: &core_policy::PolicyTarget,
) -> Result<&'a core_policy::PolicyArtifact, RuntimeError> {
    artifacts
        .iter()
        .find(|artifact| &artifact.target == target)
        .ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "missing {} runtime policy artifact",
                policy_target_name(target)
            ))
        })
}

fn policy_target_name(target: &core_policy::PolicyTarget) -> &'static str {
    match target {
        core_policy::PolicyTarget::LinuxLandlockSeccomp => "linux",
        core_policy::PolicyTarget::MacosSeatbelt => "macos",
    }
}

struct RuntimeEventBuilder {
    clock: EventClock,
    events: Vec<EventEnvelope>,
    loop_counter: u64,
    message_counter: u64,
    sequence: u64,
    session_id: String,
    stream_bytes: usize,
}

impl RuntimeEventBuilder {
    fn with_clock(session_id: String, clock: EventClock) -> Self {
        Self {
            clock,
            events: Vec::new(),
            loop_counter: 0,
            message_counter: 0,
            sequence: 0,
            session_id,
            stream_bytes: 0,
        }
    }

    fn next_loop_invocation(
        &mut self,
        parent_loop_id: Option<String>,
    ) -> Result<LoopInvocation, RuntimeError> {
        let next_loop_counter = self.loop_counter + 1;
        // WHY: loop invocation budgets preserve duplicate subloop execution semantics while
        // bounding the total runtime work one session can request.
        if next_loop_counter > MAX_LOOP_INVOCATIONS {
            return Err(RuntimeError::Protocol(format!(
                "loop invocation budget exceeded: next invocation {next_loop_counter} exceeds max {MAX_LOOP_INVOCATIONS}"
            )));
        }
        self.loop_counter = next_loop_counter;
        Ok(LoopInvocation {
            loop_id: format!("loop-{:03}", self.loop_counter),
            parent_loop_id,
        })
    }

    fn next_message_id(&mut self) -> String {
        self.message_counter += 1;
        format!("msg-{:03}", self.message_counter)
    }

    fn emit(
        &mut self,
        invocation: Option<&LoopInvocation>,
        event_type: EventType,
        payload: serde_json::Value,
    ) -> Result<(), RuntimeError> {
        let sequence = self.sequence + 1;
        // WHY: enforce event budgets before storing the event so oversized in-cap loops
        // cannot accumulate unbounded memory.
        if sequence > MAX_LOOP_EVENTS {
            return Err(RuntimeError::Protocol(format!(
                "runtime event budget exceeded: next event {sequence} exceeds max {MAX_LOOP_EVENTS}"
            )));
        }
        let mut event = EventEnvelope::new(
            format!("evt-{:03}", sequence),
            event_type,
            self.session_id.clone(),
            sequence,
            self.clock.timestamp(sequence),
            "loop-agent-cli",
            payload,
        );
        if let Some(invocation) = invocation {
            event.loop_id = Some(invocation.loop_id.clone());
            event.parent_loop_id = invocation.parent_loop_id.clone();
        }
        event.normalize_strings_to_nfc();
        let event_bytes = event.canonical_jsonl().map_err(|err| {
            RuntimeError::Protocol(format!("failed to serialize runtime event: {err}"))
        })?;
        let next_stream_bytes = self
            .stream_bytes
            .checked_add(event_bytes.len())
            .unwrap_or(usize::MAX);
        if next_stream_bytes > MAX_LOOP_EVENT_STREAM_BYTES {
            return Err(RuntimeError::Protocol(format!(
                "event stream budget exceeded: next event would use {next_stream_bytes} bytes, max {MAX_LOOP_EVENT_STREAM_BYTES}"
            )));
        }
        self.sequence = sequence;
        self.stream_bytes = next_stream_bytes;
        self.events.push(event);
        Ok(())
    }
}

fn execute_loop(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    root_loop: &core_script::LoopBlock,
    session_id: &str,
    options: LoopExecutionOptions<'_>,
) -> Result<RuntimeExecution, RuntimeError> {
    let mut builder = RuntimeEventBuilder::with_clock(session_id.to_owned(), options.clock);
    builder.emit(
        None,
        EventType::SessionStarted,
        serde_json::json!({"reason":"fixture-start"}),
    )?;

    let context = LoopEmitContext {
        workspace,
        registry,
        policy,
        side_effect_mode: options.side_effect_mode,
        side_effect_recorder: options.side_effect_recorder,
        stub_model_fixture_profile: options.stub_model_fixture_profile,
    };
    let failed = match emit_loop_block(&context, root_loop, None, &mut builder) {
        Ok(failed) => failed,
        Err(err) if should_terminalize_runtime_error(options.side_effect_mode) => {
            builder.emit(
                None,
                EventType::SessionFailed,
                serde_json::json!({"reason":RUNTIME_ERROR_REASON}),
            )?;
            return Ok(RuntimeExecution {
                events: builder.events,
                failed: true,
                terminal_error: Some(err),
            });
        }
        Err(err) => return Err(err),
    };
    if let Some(failure) = failed {
        builder.emit(
            None,
            EventType::SessionFailed,
            serde_json::json!({"reason":failure.reason}),
        )?;
        Ok(RuntimeExecution {
            events: builder.events,
            failed: true,
            terminal_error: None,
        })
    } else {
        builder.emit(None, EventType::SessionCompleted, serde_json::json!({}))?;
        Ok(RuntimeExecution {
            events: builder.events,
            failed: false,
            terminal_error: None,
        })
    }
}

fn should_terminalize_runtime_error(side_effect_mode: ToolSideEffectMode) -> bool {
    matches!(
        side_effect_mode,
        ToolSideEffectMode::ApplyAll | ToolSideEffectMode::Resume { .. }
    )
}

fn preflight_loop_tools(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    loop_block: &core_script::LoopBlock,
) -> Result<(), RuntimeError> {
    preflight_loop_tools_at_depth(workspace, registry, policy, loop_block, 1)
}

fn preflight_loop_tools_at_depth(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    loop_block: &core_script::LoopBlock,
    depth: usize,
) -> Result<(), RuntimeError> {
    if depth > core_script::MAX_LOOP_NESTING_DEPTH {
        return Err(RuntimeError::Protocol(format!(
            "loop nesting depth {depth} for {} exceeds max {}",
            loop_block.identity.id,
            core_script::MAX_LOOP_NESTING_DEPTH
        )));
    }

    for phase_ref in &loop_block.phase_refs {
        let phase = registry.phase_block(phase_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing phase {phase_ref}"))
        })?;
        preflight_phase_tools(workspace, registry, policy, phase)?;
    }

    for subloop_ref in &loop_block.subloop_refs {
        let subloop = registry.loop_block(subloop_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing loop {subloop_ref}"))
        })?;
        preflight_loop_tools_at_depth(workspace, registry, policy, subloop, depth + 1)?;
    }

    Ok(())
}

fn preflight_phase_tools(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    phase: &core_script::PhaseBlock,
) -> Result<(), RuntimeError> {
    for tool_ref in &phase.tool_refs {
        let tool = registry.tool_block(tool_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing tool {tool_ref}"))
        })?;
        let command_policy = command_policy_for_phase(policy, &phase.identity.id, tool)?;
        ensure_tool_matches_policy(tool, &policy.target, command_policy)?;
        tool_dispatch_progress(
            tool,
            runtime_protected_path_match_mode(&policy.target),
            command_policy,
            ToolDispatchMode::Preflight { workspace },
        )?;
    }
    Ok(())
}

fn emit_loop_block(
    context: &LoopEmitContext<'_>,
    loop_block: &core_script::LoopBlock,
    parent_loop_id: Option<String>,
    builder: &mut RuntimeEventBuilder,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    emit_loop_block_at_depth(context, loop_block, parent_loop_id, builder, 1)
}

struct LoopEmitContext<'a> {
    workspace: &'a Path,
    registry: &'a core_script::ResolvedRegistry,
    policy: &'a core_policy::PolicyArtifact,
    side_effect_mode: ToolSideEffectMode,
    side_effect_recorder: SideEffectRecorder<'a>,
    stub_model_fixture_profile: bool,
}

fn emit_loop_block_at_depth(
    context: &LoopEmitContext<'_>,
    loop_block: &core_script::LoopBlock,
    parent_loop_id: Option<String>,
    builder: &mut RuntimeEventBuilder,
    depth: usize,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    if depth > core_script::MAX_LOOP_NESTING_DEPTH {
        return Err(RuntimeError::Protocol(format!(
            "loop nesting depth {depth} for {} exceeds max {}",
            loop_block.identity.id,
            core_script::MAX_LOOP_NESTING_DEPTH
        )));
    }

    let invocation = builder.next_loop_invocation(parent_loop_id)?;
    builder.emit(
        Some(&invocation),
        EventType::LoopStarted,
        serde_json::json!({
            "loop_definition_id": loop_block.identity.id,
            "loop_name": loop_block.identity.name,
        }),
    )?;

    for phase_ref in &loop_block.phase_refs {
        let phase = context.registry.phase_block(phase_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing phase {phase_ref}"))
        })?;
        match emit_phase(context, phase, &invocation, builder) {
            Ok(Some(failure)) => {
                emit_runtime_failure(loop_block, &invocation, &failure, builder)?;
                return Ok(Some(failure));
            }
            Ok(None) => {}
            Err(err) if should_terminalize_runtime_error(context.side_effect_mode) => {
                emit_runtime_error_failure(loop_block, &invocation, &err, builder)?;
                return Err(err);
            }
            Err(err) => return Err(err),
        }
    }

    for subloop_ref in &loop_block.subloop_refs {
        let subloop = context.registry.loop_block(subloop_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing loop {subloop_ref}"))
        })?;
        match emit_loop_block_at_depth(
            context,
            subloop,
            Some(invocation.loop_id.clone()),
            builder,
            depth + 1,
        ) {
            Ok(Some(failure)) => {
                emit_propagated_runtime_failure(loop_block, &invocation, &failure, builder)?;
                return Ok(Some(failure));
            }
            Ok(None) => {}
            Err(err) if should_terminalize_runtime_error(context.side_effect_mode) => {
                emit_propagated_runtime_error_failure(loop_block, &invocation, builder)?;
                return Err(err);
            }
            Err(err) => return Err(err),
        }
    }

    builder.emit(
        Some(&invocation),
        EventType::LoopCompleted,
        serde_json::json!({
            "loop_definition_id": loop_block.identity.id,
            "loop_name": loop_block.identity.name,
        }),
    )?;
    Ok(None)
}

fn emit_phase(
    context: &LoopEmitContext<'_>,
    phase: &core_script::PhaseBlock,
    invocation: &LoopInvocation,
    builder: &mut RuntimeEventBuilder,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    let instruction_ids = phase
        .instruction_refs
        .iter()
        .map(|instruction_ref| {
            context
                .registry
                .instruction_block(instruction_ref)
                .map(|instruction| instruction.identity.id.clone())
                .ok_or_else(|| {
                    RuntimeError::Protocol(format!(
                        "resolved registry missing instruction {instruction_ref}"
                    ))
                })
        })
        .collect::<Result<Vec<_>, RuntimeError>>()?;
    let tool_ids = phase
        .tool_refs
        .iter()
        .map(|tool_ref| {
            context
                .registry
                .tool_block(tool_ref)
                .map(|tool| tool.identity.id.clone())
                .ok_or_else(|| {
                    RuntimeError::Protocol(format!("resolved registry missing tool {tool_ref}"))
                })
        })
        .collect::<Result<Vec<_>, RuntimeError>>()?;
    builder.emit(
        Some(invocation),
        EventType::PhaseEntered,
        serde_json::json!({
            "instruction_ids": instruction_ids,
            "phase_id": phase.identity.id,
            "phase_name": phase.identity.name,
            "tool_ids": tool_ids,
        }),
    )?;

    for (step_index, step) in phase.steps.iter().enumerate() {
        let step_payload = step_payload(context.registry, phase, step)?;
        builder.emit(
            Some(invocation),
            EventType::StepStarted,
            step_payload.clone(),
        )?;

        if let Some(content) = stub_message_content(context.registry, phase)? {
            let message_id = builder.next_message_id();
            builder.emit(
                Some(invocation),
                EventType::MessageDelta,
                serde_json::json!({
                    "content_delta": content,
                    "message_id": message_id,
                    "role": "assistant",
                }),
            )?;
            builder.emit(
                Some(invocation),
                EventType::MessageCompleted,
                serde_json::json!({
                    "message_id": message_id,
                    "role": "assistant",
                }),
            )?;
        }

        if step_index == 0 {
            if let Some(failure) = sandbox_out_of_phase_failure(
                context.registry,
                context.policy,
                phase,
                context.stub_model_fixture_profile,
            ) {
                builder.emit(Some(invocation), EventType::StepCompleted, step_payload)?;
                return Ok(Some(failure));
            }

            for tool_ref in &phase.tool_refs {
                let tool = context.registry.tool_block(tool_ref).ok_or_else(|| {
                    RuntimeError::Protocol(format!("resolved registry missing tool {tool_ref}"))
                })?;
                let command_policy =
                    command_policy_for_phase(context.policy, &phase.identity.id, tool)?;
                let tool_policy = RuntimeToolPolicy {
                    command: command_policy,
                    protected_path_match_mode: runtime_protected_path_match_mode(
                        &context.policy.target,
                    ),
                    stub_model_fixture_profile: context.stub_model_fixture_profile,
                    target: &context.policy.target,
                };
                match emit_tool(
                    context.workspace,
                    tool,
                    tool_policy,
                    invocation,
                    context.side_effect_mode,
                    context.side_effect_recorder,
                    builder,
                ) {
                    Ok(Some(mut failure)) => {
                        emit_runtime_tool_failure(invocation, &failure, builder)?;
                        failure.emit_tool_failed = false;
                        builder.emit(Some(invocation), EventType::StepCompleted, step_payload)?;
                        return Ok(Some(failure));
                    }
                    Ok(None) => {}
                    Err(err) if should_terminalize_runtime_error(context.side_effect_mode) => {
                        let mut failure = runtime_failure_for_unhandled_error(&err);
                        failure.tool_id = Some(tool.identity.id.clone());
                        emit_runtime_tool_failure(invocation, &failure, builder)?;
                        builder.emit(Some(invocation), EventType::StepCompleted, step_payload)?;
                        return Err(err);
                    }
                    Err(err) => return Err(err),
                }
            }
        }

        builder.emit(Some(invocation), EventType::StepCompleted, step_payload)?;
    }

    Ok(None)
}

fn step_payload(
    registry: &core_script::ResolvedRegistry,
    phase: &core_script::PhaseBlock,
    step: &core_script::StepBlock,
) -> Result<serde_json::Value, RuntimeError> {
    let mut payload = serde_json::json!({
        "phase_id": phase.identity.id,
        "step_id": step.id,
        "step_name": step.name,
    });
    if !step.connection_refs.is_empty() {
        let connection_kinds = step
            .connection_refs
            .iter()
            .map(|connection_ref| {
                let connection = registry.connection_block(connection_ref).ok_or_else(|| {
                    RuntimeError::Protocol(format!(
                        "resolved registry missing connection {connection_ref}"
                    ))
                })?;
                Ok(connection_kind_name(&connection.connection_kind))
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;
        let connection_ids = step
            .connection_refs
            .iter()
            .map(|connection_ref| {
                registry
                    .connection_block(connection_ref)
                    .map(|connection| connection.identity.id.clone())
                    .ok_or_else(|| {
                        RuntimeError::Protocol(format!(
                            "resolved registry missing connection {connection_ref}"
                        ))
                    })
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;
        let object = payload
            .as_object_mut()
            .expect("step payload is constructed as an object");
        object.insert(
            "connection_ids".to_owned(),
            serde_json::json!(connection_ids),
        );
        object.insert(
            "connection_kinds".to_owned(),
            serde_json::json!(connection_kinds),
        );
    }
    Ok(payload)
}

fn stub_message_content(
    registry: &core_script::ResolvedRegistry,
    phase: &core_script::PhaseBlock,
) -> Result<Option<&'static str>, RuntimeError> {
    let has_predefined_tool = phase.tool_refs.iter().any(|tool_ref| {
        registry
            .tool_block(tool_ref)
            .is_some_and(|tool| tool.tool_kind == core_script::ToolKind::PredefinedCommand)
    });
    if !has_predefined_tool {
        return Ok(None);
    }

    for instruction_ref in &phase.instruction_refs {
        let instruction = registry.instruction_block(instruction_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "resolved registry missing instruction {instruction_ref}"
            ))
        })?;
        if instruction.prompt.to_ascii_lowercase().contains("smoke") {
            return Ok(Some("smoke"));
        }
    }

    Ok(Some("hello"))
}

fn command_policy_for_phase<'a>(
    policy: &'a core_policy::PolicyArtifact,
    phase_id: &str,
    tool: &core_script::ToolBlock,
) -> Result<&'a core_policy::CommandPolicy, RuntimeError> {
    let scoped = policy
        .phase_scope
        .iter()
        .find(|phase| phase.phase_id == phase_id)
        .is_some_and(|phase| {
            phase
                .tool_ids
                .iter()
                .any(|tool_id| tool_id == &tool.identity.id)
        });
    if !scoped {
        return Err(RuntimeError::Protocol(format!(
            "tool {} is not available in phase {phase_id}",
            tool.identity.id
        )));
    }
    policy
        .commands
        .iter()
        .find(|command| command.tool_id == tool.identity.id)
        .ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "runtime policy missing command for tool {}",
                tool.identity.id
            ))
        })
}

fn ensure_tool_matches_policy(
    tool: &core_script::ToolBlock,
    target: &core_policy::PolicyTarget,
    policy: &core_policy::CommandPolicy,
) -> Result<(), RuntimeError> {
    if policy.tool_id != tool.identity.id {
        return Err(RuntimeError::Protocol(format!(
            "runtime policy tool_id {} does not match tool {}",
            policy.tool_id, tool.identity.id
        )));
    }
    if policy_tool_kind_name(&policy.tool_kind) != tool_kind_name(&tool.tool_kind) {
        return Err(RuntimeError::Protocol(format!(
            "runtime policy kind does not match tool {}",
            tool.identity.id
        )));
    }
    if policy.network.default != core_policy::NetworkDefault::Deny {
        return Err(RuntimeError::Protocol(format!(
            "tool {} must use deny-all network policy",
            tool.identity.id
        )));
    }
    if matches!(target, core_policy::PolicyTarget::LinuxLandlockSeccomp)
        && !policy.network.allow.is_empty()
    {
        return Err(RuntimeError::Protocol(format!(
            "tool {} must use deny-all network policy",
            tool.identity.id
        )));
    }

    match (&tool.tool_kind, &tool.command) {
        (
            core_script::ToolKind::PredefinedCommand,
            core_script::ToolCommand::Predefined { command_id, argv },
        ) => {
            if policy.command_id != *command_id
                || policy.executable != format!("registry:{command_id}")
                || policy.argv != *argv
                || policy.script_runtime.is_some()
            {
                return Err(RuntimeError::Protocol(format!(
                    "runtime policy command does not match tool {}",
                    tool.identity.id
                )));
            }
        }
        (core_script::ToolKind::OwnScript, core_script::ToolCommand::OwnScript(command_id)) => {
            if policy.command_id != *command_id
                || policy.executable != "runner:posix-sh"
                || policy.script_runtime.as_deref() != Some("posix-sh")
                || !policy.argv.is_empty()
            {
                return Err(RuntimeError::Protocol(format!(
                    "runtime policy script command does not match tool {}",
                    tool.identity.id
                )));
            }
        }
        _ => {
            return Err(RuntimeError::Protocol(format!(
                "tool command shape does not match {}",
                tool.identity.id
            )));
        }
    }

    Ok(())
}

