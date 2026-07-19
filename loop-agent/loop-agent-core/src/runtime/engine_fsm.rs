struct RuntimeExecution {
    context_manifests: RuntimeStreamSignature,
    events: RuntimeStreamSignature,
    failed: bool,
    failure_status: Option<String>,
    terminal_error: Option<RuntimeError>,
}

impl RuntimeExecution {
    fn matches_plan(&self, planned: &Self) -> bool {
        self.events == planned.events
            && self.context_manifests == planned.context_manifests
            && self.failed == planned.failed
            && self.failure_status == planned.failure_status
    }
}

const EVENT_PLAN_DOMAIN: &[u8] = b"watershed.runtime.event-plan.v1";
const CONTEXT_PLAN_DOMAIN: &[u8] = b"watershed.runtime.context-plan.v1";
static LIVE_LOOP_INVOCATIONS: LiveInvocationCounter = LiveInvocationCounter::new();

struct LiveInvocationCounter {
    count: std::sync::atomic::AtomicUsize,
}

impl LiveInvocationCounter {
    const fn new() -> Self {
        Self {
            count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn acquire(&self) -> Result<LiveInvocationGuard<'_>, RuntimeError> {
        let mut observed = self.count.load(std::sync::atomic::Ordering::Acquire);
        loop {
            if observed >= MAX_LIVE_LOOP_INVOCATIONS {
                return Err(RuntimeError::Protocol(format!(
                    "global live loop invocation limit reached: max {MAX_LIVE_LOOP_INVOCATIONS}"
                )));
            }
            match self.count.compare_exchange_weak(
                observed,
                observed + 1,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            ) {
                Ok(_) => return Ok(LiveInvocationGuard { counter: self }),
                Err(actual) => observed = actual,
            }
        }
    }

    fn acquire_for(
        &self,
        mode: ToolSideEffectMode,
        terminal_in_prefix: bool,
    ) -> Result<Option<LiveInvocationGuard<'_>>, RuntimeError> {
        mode.occupies_live_invocation_slot(terminal_in_prefix)
            .then(|| self.acquire())
            .transpose()
    }
}

struct LiveInvocationGuard<'a> {
    counter: &'a LiveInvocationCounter,
}

impl Drop for LiveInvocationGuard<'_> {
    fn drop(&mut self) {
        self.counter
            .count
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeStreamSignature {
    byte_count: usize,
    digest: [u8; 32],
    record_count: usize,
}

#[derive(Clone)]
struct RuntimeStreamSignatureBuilder {
    byte_count: usize,
    hasher: Sha256,
    record_count: usize,
}

impl RuntimeStreamSignatureBuilder {
    fn new(domain: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(
            u64::try_from(domain.len())
                .unwrap_or(u64::MAX)
                .to_be_bytes(),
        );
        hasher.update(domain);
        Self {
            byte_count: 0,
            hasher,
            record_count: 0,
        }
    }

    fn push(&mut self, record: &[u8]) {
        self.hasher.update(
            u64::try_from(record.len())
                .unwrap_or(u64::MAX)
                .to_be_bytes(),
        );
        self.hasher.update(record);
        self.byte_count = self.byte_count.saturating_add(record.len());
        self.record_count = self.record_count.saturating_add(1);
    }

    fn signature(&self) -> RuntimeStreamSignature {
        RuntimeStreamSignature {
            byte_count: self.byte_count,
            digest: self.hasher.clone().finalize().into(),
            record_count: self.record_count,
        }
    }
}

#[derive(Clone, Debug)]
struct LoopInvocation {
    loop_id: String,
    parent_loop_id: Option<String>,
}

struct RuntimeFailure {
    reason: String,
    message: &'static str,
    data: serde_json::Map<String, serde_json::Value>,
    tool_id: Option<String>,
    phase_id: Option<String>,
    emit_tool_failed: bool,
}

#[derive(Clone, Copy)]
struct RuntimeToolPolicy<'a> {
    command: &'a core_policy::CommandPolicy,
    protected_path_match_mode: ProtectedPathMatchMode,
    stub_model_fixture_profile: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolSideEffectMode {
    ApplyAll,
    DryRun,
    PreflightResume { prefix_event_count: u64 },
    Resume { prefix_event_count: u64 },
}

impl ToolSideEffectMode {
    fn occupies_live_invocation_slot(self, terminal_in_prefix: bool) -> bool {
        matches!(self, Self::ApplyAll)
            || matches!(self, Self::Resume { .. }) && !terminal_in_prefix
    }

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

#[derive(Clone, Debug)]
struct LoopExecutionOptions {
    clock: EventClock,
    side_effect_mode: ToolSideEffectMode,
    stub_model_fixture_profile: bool,
    terminal_loop_ids: BTreeSet<String>,
}

impl LoopExecutionOptions {
    fn with_stub_model_fixture_profile(
        clock: EventClock,
        side_effect_mode: ToolSideEffectMode,
        stub_model_fixture_profile: bool,
    ) -> Self {
        Self {
            clock,
            side_effect_mode,
            stub_model_fixture_profile,
            terminal_loop_ids: BTreeSet::new(),
        }
    }

    fn with_terminal_loop_ids(mut self, terminal_loop_ids: BTreeSet<String>) -> Self {
        self.terminal_loop_ids = terminal_loop_ids;
        self
    }
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
fn runtime_protected_path_match_mode(
    _target: &core_policy::PolicyTarget,
) -> ProtectedPathMatchMode {
    ProtectedPathMatchMode::CaseInsensitive
}

#[cfg(not(windows))]
fn runtime_protected_path_match_mode(target: &core_policy::PolicyTarget) -> ProtectedPathMatchMode {
    core_policy::protected_path_match_mode_for_policy_target(target)
}

struct RuntimeEventBuilder<'a> {
    active_step_payloads: BTreeMap<String, serde_json::Value>,
    clock: EventClock,
    context_manifest_count: usize,
    context_manifests: RuntimeStreamSignatureBuilder,
    events: RuntimeStreamSignatureBuilder,
    failure_messages: BTreeMap<String, String>,
    failure_status: Option<String>,
    history: ContextHistory,
    loop_counter: u64,
    message_counter: u64,
    sequence: u64,
    session_id: String,
    sink: Option<&'a mut dyn RuntimeEventSink>,
    pending_context_manifest: Option<(ContextManifest, Vec<ContextObject>)>,
    validation: Option<SessionAppendValidationState>,
}

impl<'a> RuntimeEventBuilder<'a> {
    fn with_clock(session_id: String, clock: EventClock, validate_plan: bool) -> Self {
        let validation = validate_plan.then(|| SessionAppendValidationState::empty(&session_id));
        Self {
            active_step_payloads: BTreeMap::new(),
            clock,
            context_manifest_count: 0,
            context_manifests: RuntimeStreamSignatureBuilder::new(CONTEXT_PLAN_DOMAIN),
            events: RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN),
            failure_messages: BTreeMap::new(),
            failure_status: None,
            history: ContextHistory::default(),
            loop_counter: 0,
            message_counter: 0,
            sequence: 0,
            session_id,
            sink: None,
            pending_context_manifest: None,
            validation,
        }
    }

    fn with_sink(
        session_id: String,
        clock: EventClock,
        validate_plan: bool,
        sink: &'a mut dyn RuntimeEventSink,
    ) -> Self {
        let mut builder = Self::with_clock(session_id, clock, validate_plan);
        builder.sink = Some(sink);
        builder
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

    fn record_context_manifest(
        &mut self,
        manifest: ContextManifest,
        objects: Vec<ContextObject>,
    ) -> Result<(), RuntimeError> {
        ensure_context_manifest_growth_within_limit(
            Path::new("runtime.contexts.jsonl"),
            self.context_manifests.byte_count,
            manifest.line.len(),
        )?;
        self.context_manifest_count += 1;
        self.pending_context_manifest = Some((manifest, objects));
        Ok(())
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
        let measurement_started_at = self
            .sink
            .as_deref()
            .and_then(RuntimeEventSink::measurement_started_at);
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
        let event_bytes = event.canonical_jsonl().map_err(|err| {
            RuntimeError::Protocol(format!("failed to serialize runtime event: {err}"))
        })?;
        let context_manifest = if event.event_type == EventType::MessageCompleted {
            let (manifest, objects) = self.pending_context_manifest.take().ok_or_else(|| {
                RuntimeError::Protocol(
                    "message.completed has no compiled context manifest".to_owned(),
                )
            })?;
            Some(ContextManifestCheckpoint {
                manifest,
                objects,
                ordinal: self.context_manifest_count,
            })
        } else {
            None
        };
        if let Some(validation) = self.validation.as_mut() {
            validation.validate_constructed_event(
                Path::new("runtime.jsonl"),
                &event,
                event_bytes.len(),
            )?;
        }
        match event.event_type {
            EventType::Error => {
                if let (Some(code), Some(message)) = (
                    event
                        .payload
                        .get("code")
                        .and_then(serde_json::Value::as_str),
                    event
                        .payload
                        .get("message")
                        .and_then(serde_json::Value::as_str),
                ) {
                    self.failure_messages
                        .insert(code.to_owned(), message.to_owned());
                }
            }
            EventType::SessionFailed => {
                if let Some(reason) = event
                    .payload
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                {
                    self.failure_status = Some(render_human_failure_status(
                        reason,
                        self.failure_messages.get(reason).map(String::as_str),
                    ));
                }
            }
            _ => {}
        }
        self.events.push(event_bytes.as_bytes());
        if let Some(checkpoint) = context_manifest.as_ref() {
            self.context_manifests
                .push(checkpoint.manifest.line.as_bytes());
        }
        if let Some(sink) = self.sink.as_deref_mut() {
            sink.commit(
                &event,
                &event_bytes,
                context_manifest,
                measurement_started_at,
            )?;
        }
        self.sequence = sequence;
        self.history.record(&event);
        if let Some(invocation) = invocation {
            match event.event_type {
                EventType::StepStarted => {
                    self.active_step_payloads
                        .insert(invocation.loop_id.clone(), event.payload.clone());
                }
                EventType::StepCompleted => {
                    self.active_step_payloads.remove(&invocation.loop_id);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn into_execution(
        self,
        failed: bool,
        terminal_error: Option<RuntimeError>,
    ) -> RuntimeExecution {
        RuntimeExecution {
            context_manifests: self.context_manifests.signature(),
            events: self.events.signature(),
            failed,
            failure_status: self.failure_status,
            terminal_error,
        }
    }
}

fn execute_loop(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    root_loop: &core_script::LoopBlock,
    session_id: &str,
    options: LoopExecutionOptions,
) -> Result<RuntimeExecution, RuntimeError> {
    execute_loop_with_sink(
        workspace, registry, policy, root_loop, session_id, options, None,
    )
}

fn execute_loop_with_sink(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    root_loop: &core_script::LoopBlock,
    session_id: &str,
    options: LoopExecutionOptions,
    sink: Option<&mut dyn RuntimeEventSink>,
) -> Result<RuntimeExecution, RuntimeError> {
    let validate_plan = options.side_effect_mode == ToolSideEffectMode::DryRun;
    let mut builder = match sink {
        Some(sink) => RuntimeEventBuilder::with_sink(
            session_id.to_owned(),
            options.clock,
            validate_plan,
            sink,
        ),
        None => {
            RuntimeEventBuilder::with_clock(session_id.to_owned(), options.clock, validate_plan)
        }
    };
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
        stub_model_fixture_profile: options.stub_model_fixture_profile,
        terminal_loop_ids: &options.terminal_loop_ids,
    };
    let failed = match emit_loop_block(&context, root_loop, None, &mut builder) {
        Ok(failed) => failed,
        Err(err) if should_terminalize_error(options.side_effect_mode, &err) => {
            let reason = runtime_failure_for_unhandled_error(&err).reason;
            builder.emit(
                None,
                EventType::SessionFailed,
                serde_json::json!({"reason":reason}),
            )?;
            return Ok(builder.into_execution(true, Some(err)));
        }
        Err(err) => return Err(err),
    };
    if let Some(failure) = failed {
        builder.emit(
            None,
            EventType::SessionFailed,
            serde_json::json!({"reason":failure.reason}),
        )?;
        Ok(builder.into_execution(true, None))
    } else {
        builder.emit(None, EventType::SessionCompleted, serde_json::json!({}))?;
        Ok(builder.into_execution(false, None))
    }
}

fn should_terminalize_runtime_error(side_effect_mode: ToolSideEffectMode) -> bool {
    matches!(
        side_effect_mode,
        ToolSideEffectMode::ApplyAll | ToolSideEffectMode::Resume { .. }
    )
}

fn should_terminalize_error(side_effect_mode: ToolSideEffectMode, err: &RuntimeError) -> bool {
    !matches!(err, RuntimeError::EventWriter(_))
        && (should_terminalize_runtime_error(side_effect_mode)
            || matches!(err, RuntimeError::ContextBudgetExceeded { .. }))
}

fn preflight_loop_tools(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    loop_block: &core_script::LoopBlock,
) -> Result<(), RuntimeError> {
    let mut invocation_count = 0;
    preflight_loop_tools_at_depth(
        workspace,
        registry,
        policy,
        loop_block,
        1,
        &mut invocation_count,
    )
}

fn preflight_loop_tools_at_depth(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    loop_block: &core_script::LoopBlock,
    depth: usize,
    invocation_count: &mut u64,
) -> Result<(), RuntimeError> {
    *invocation_count = invocation_count.checked_add(1).ok_or_else(|| {
        RuntimeError::Protocol("loop invocation budget counter overflowed".to_owned())
    })?;
    if *invocation_count > MAX_LOOP_INVOCATIONS {
        return Err(RuntimeError::Protocol(format!(
            "loop invocation budget exceeded: next invocation {invocation_count} exceeds max {MAX_LOOP_INVOCATIONS}"
        )));
    }
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
        preflight_loop_tools_at_depth(
            workspace,
            registry,
            policy,
            subloop,
            depth + 1,
            invocation_count,
        )?;
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
    builder: &mut RuntimeEventBuilder<'_>,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    emit_loop_block_at_depth(context, loop_block, parent_loop_id, builder, 1)
}

struct LoopEmitContext<'a> {
    workspace: &'a Path,
    registry: &'a core_script::ResolvedRegistry,
    policy: &'a core_policy::PolicyArtifact,
    side_effect_mode: ToolSideEffectMode,
    stub_model_fixture_profile: bool,
    terminal_loop_ids: &'a BTreeSet<String>,
}

fn emit_loop_block_at_depth(
    context: &LoopEmitContext<'_>,
    loop_block: &core_script::LoopBlock,
    parent_loop_id: Option<String>,
    builder: &mut RuntimeEventBuilder<'_>,
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
    // A parent remains live while it waits for a nested invocation; queued but not started,
    // terminal and fully paused invocations do not hold this process-wide slot.
    let _live_invocation = LIVE_LOOP_INVOCATIONS.acquire_for(
        context.side_effect_mode,
        context.terminal_loop_ids.contains(&invocation.loop_id),
    )?;
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
        match emit_phase(context, loop_block, phase, &invocation, builder) {
            Ok(Some(failure)) => {
                emit_runtime_failure(loop_block, &invocation, &failure, builder)?;
                return Ok(Some(failure));
            }
            Ok(None) => {}
            Err(err) if should_terminalize_error(context.side_effect_mode, &err) => {
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
                emit_runtime_loop_failure(loop_block, &invocation, &failure.reason, builder)?;
                return Ok(Some(failure));
            }
            Ok(None) => {}
            Err(err) if should_terminalize_error(context.side_effect_mode, &err) => {
                let reason = runtime_failure_for_unhandled_error(&err).reason;
                emit_runtime_loop_failure(loop_block, &invocation, &reason, builder)?;
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
    loop_block: &core_script::LoopBlock,
    phase: &core_script::PhaseBlock,
    invocation: &LoopInvocation,
    builder: &mut RuntimeEventBuilder<'_>,
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

        if phase_uses_stub_model(context.registry, phase) {
            let compiled = compile_provider_turn_context(
                context.registry,
                loop_block,
                phase,
                step,
                invocation,
                &builder.session_id,
                &builder.history,
            )?;
            let content = stub_message_content(context.registry, phase, &compiled.provider_bytes)?;
            builder.record_context_manifest(compiled.manifest, compiled.objects)?;
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
                };
                match emit_tool(
                    context.workspace,
                    tool,
                    tool_policy,
                    invocation,
                    context.side_effect_mode,
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
        let connections = step
            .connection_refs
            .iter()
            .map(|connection_ref| {
                let connection = registry.connection_block(connection_ref).ok_or_else(|| {
                    RuntimeError::Protocol(format!(
                        "resolved registry missing connection {connection_ref}"
                    ))
                })?;
                Ok((
                    connection.identity.id.clone(),
                    connection_kind_name(&connection.connection_kind),
                ))
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;
        let (connection_ids, connection_kinds): (Vec<_>, Vec<_>) = connections.into_iter().unzip();
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

fn phase_uses_stub_model(
    registry: &core_script::ResolvedRegistry,
    phase: &core_script::PhaseBlock,
) -> bool {
    phase.tool_refs.iter().any(|tool_ref| {
        registry
            .tool_block(tool_ref)
            .is_some_and(|tool| tool.tool_kind == core_script::ToolKind::PredefinedCommand)
    })
}

fn stub_message_content(
    registry: &core_script::ResolvedRegistry,
    phase: &core_script::PhaseBlock,
    provider_context: &[u8],
) -> Result<&'static str, RuntimeError> {
    if provider_context.is_empty() {
        return Err(RuntimeError::Protocol(
            "stub model received empty compiled context".to_owned(),
        ));
    }

    for instruction_ref in &phase.instruction_refs {
        let instruction = registry.instruction_block(instruction_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "resolved registry missing instruction {instruction_ref}"
            ))
        })?;
        if instruction.prompt.to_ascii_lowercase().contains("smoke") {
            return Ok("smoke");
        }
    }

    Ok("hello")
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
