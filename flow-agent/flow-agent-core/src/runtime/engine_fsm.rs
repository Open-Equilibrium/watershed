use super::*;

#[derive(Debug)]
pub struct RuntimeExecution {
    pub(crate) context_manifests: RuntimeStreamSignature,
    pub(crate) events: RuntimeStreamSignature,
    pub(crate) failed: bool,
    pub(crate) failure_status: Option<String>,
    pub(crate) terminal_error: Option<RuntimeError>,
    pub(crate) tool_intents: Vec<PlannedToolIntent>,
}

impl RuntimeExecution {
    pub(crate) fn matches_plan(&self, plan: &FlowExecutionPlan) -> bool {
        self.events == plan.execution.events
            && self.context_manifests == plan.execution.context_manifests
            && self.failed == plan.execution.failed
            && self.failure_status == plan.execution.failure_status
            && self.tool_intents == plan.execution.tool_intents
            && FlowExecutionPlan::signature_for(self) == plan.signature
    }
}

pub const EVENT_PLAN_DOMAIN: &[u8] = b"watershed.runtime.event-plan.v1";
pub const CONTEXT_PLAN_DOMAIN: &[u8] = b"watershed.runtime.context-plan.v1";
pub const FLOW_EXECUTION_PLAN_DOMAIN: &[u8] = b"watershed.runtime.flow-execution-plan.v1";
pub static LIVE_FLOW_INVOCATIONS: LiveInvocationCounter = LiveInvocationCounter::new();

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedToolIntent {
    pub(crate) canonical: String,
    pub(crate) flow_id: String,
    pub(crate) tool_id: String,
}

pub struct FlowExecutionPlan {
    pub(crate) execution: RuntimeExecution,
    pub(crate) signature: RuntimeStreamSignature,
}

impl FlowExecutionPlan {
    pub(crate) fn from_execution(execution: RuntimeExecution) -> Self {
        let signature = Self::signature_for(&execution);
        Self {
            execution,
            signature,
        }
    }

    pub(crate) fn signature_for(execution: &RuntimeExecution) -> RuntimeStreamSignature {
        let mut signature = RuntimeStreamSignatureBuilder::new(FLOW_EXECUTION_PLAN_DOMAIN);
        signature.push(&execution.events.digest);
        signature.push(&execution.context_manifests.digest);
        signature.push(&execution.events.record_count.to_be_bytes());
        signature.push(&execution.context_manifests.record_count.to_be_bytes());
        signature.push(&[u8::from(execution.failed)]);
        signature.push(
            execution
                .failure_status
                .as_deref()
                .unwrap_or_default()
                .as_bytes(),
        );
        for intent in &execution.tool_intents {
            signature.push(intent.canonical.as_bytes());
        }
        signature.signature()
    }

    pub(crate) fn validate_integrity(&self) -> Result<(), RuntimeError> {
        if Self::signature_for(&self.execution) != self.signature {
            return Err(RuntimeError::Protocol(
                "flow execution plan signature is invalid".to_owned(),
            ));
        }
        Ok(())
    }
}

pub struct LiveInvocationCounter {
    pub(crate) count: std::sync::atomic::AtomicUsize,
}

impl LiveInvocationCounter {
    pub(crate) const fn new() -> Self {
        Self {
            count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub(crate) fn acquire(&self) -> Result<LiveInvocationGuard<'_>, RuntimeError> {
        let mut observed = self.count.load(std::sync::atomic::Ordering::Acquire);
        loop {
            if observed >= MAX_LIVE_FLOW_INVOCATIONS {
                return Err(RuntimeError::Protocol(format!(
                    "global live flow invocation limit reached: max {MAX_LIVE_FLOW_INVOCATIONS}"
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
}

pub struct LiveInvocationGuard<'a> {
    pub(crate) counter: &'a LiveInvocationCounter,
}

impl Drop for LiveInvocationGuard<'_> {
    fn drop(&mut self) {
        self.counter
            .count
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeStreamSignature {
    pub(crate) byte_count: usize,
    pub(crate) digest: [u8; 32],
    pub(crate) record_count: usize,
}

#[derive(Clone)]
pub struct RuntimeStreamSignatureBuilder {
    pub(crate) byte_count: usize,
    pub(crate) hasher: Sha256,
    pub(crate) record_count: usize,
}

impl RuntimeStreamSignatureBuilder {
    pub(crate) fn new(domain: &[u8]) -> Self {
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

    pub(crate) fn push(&mut self, record: &[u8]) {
        self.hasher.update(
            u64::try_from(record.len())
                .unwrap_or(u64::MAX)
                .to_be_bytes(),
        );
        self.hasher.update(record);
        self.byte_count = self.byte_count.saturating_add(record.len());
        self.record_count = self.record_count.saturating_add(1);
    }

    pub(crate) fn signature(&self) -> RuntimeStreamSignature {
        RuntimeStreamSignature {
            byte_count: self.byte_count,
            digest: self.hasher.clone().finalize().into(),
            record_count: self.record_count,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FlowInvocation {
    pub(crate) flow_id: String,
    pub(crate) parent_flow_id: Option<String>,
}

pub struct RuntimeFailure {
    pub(crate) reason: String,
    pub(crate) message: &'static str,
    pub(crate) data: serde_json::Map<String, serde_json::Value>,
    pub(crate) tool_id: Option<String>,
    pub(crate) phase_id: Option<String>,
    pub(crate) emit_tool_failed: bool,
}

#[derive(Clone, Copy)]
pub struct RuntimeToolPolicy<'a> {
    pub(crate) command: &'a core_policy::CommandPolicy,
    pub(crate) protected_path_match_mode: ProtectedPathMatchMode,
    pub(crate) stub_model_fixture_profile: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolSideEffectMode {
    Apply,
    Plan,
    PreflightResume { prefix_event_count: u64 },
    Resume { prefix_event_count: u64 },
}

impl ToolSideEffectMode {
    pub(crate) fn occupies_live_invocation_slot(self, terminal_in_prefix: bool) -> bool {
        matches!(self, Self::Apply) || matches!(self, Self::Resume { .. }) && !terminal_in_prefix
    }

    pub(crate) fn should_execute_tool(self, completed_sequence: u64) -> bool {
        match self {
            Self::Apply => true,
            Self::Plan => false,
            Self::PreflightResume { .. } => false,
            Self::Resume { prefix_event_count } => completed_sequence > prefix_event_count,
        }
    }

    pub(crate) fn should_preflight_tool(self, completed_sequence: u64) -> bool {
        match self {
            Self::PreflightResume { prefix_event_count } => completed_sequence > prefix_event_count,
            Self::Apply | Self::Plan | Self::Resume { .. } => false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FlowExecutionOptions {
    pub(crate) clock: EventClock,
    pub(crate) side_effect_mode: ToolSideEffectMode,
    pub(crate) stub_model_fixture_profile: bool,
    pub(crate) terminal_flow_ids: BTreeSet<String>,
}

impl FlowExecutionOptions {
    pub(crate) fn with_stub_model_fixture_profile(
        clock: EventClock,
        side_effect_mode: ToolSideEffectMode,
        stub_model_fixture_profile: bool,
    ) -> Self {
        Self {
            clock,
            side_effect_mode,
            stub_model_fixture_profile,
            terminal_flow_ids: BTreeSet::new(),
        }
    }

    pub(crate) fn with_terminal_flow_ids(mut self, terminal_flow_ids: BTreeSet<String>) -> Self {
        self.terminal_flow_ids = terminal_flow_ids;
        self
    }
}

#[cfg(target_os = "macos")]
pub fn runtime_policy_target() -> core_policy::PolicyTarget {
    core_policy::PolicyTarget::MacosSeatbelt
}

#[cfg(not(target_os = "macos"))]
pub fn runtime_policy_target() -> core_policy::PolicyTarget {
    core_policy::PolicyTarget::LinuxLandlockSeccomp
}

#[cfg(windows)]
pub fn runtime_protected_path_match_mode(
    _target: &core_policy::PolicyTarget,
) -> ProtectedPathMatchMode {
    ProtectedPathMatchMode::CaseInsensitive
}

#[cfg(not(windows))]
pub fn runtime_protected_path_match_mode(
    target: &core_policy::PolicyTarget,
) -> ProtectedPathMatchMode {
    core_policy::protected_path_match_mode_for_policy_target(target)
}

pub struct RuntimeEventBuilder<'a> {
    pub(crate) active_step_payloads: BTreeMap<String, serde_json::Value>,
    pub(crate) clock: EventClock,
    pub(crate) context_manifests: RuntimeStreamSignatureBuilder,
    pub(crate) events: RuntimeStreamSignatureBuilder,
    pub(crate) failure_messages: BTreeMap<String, String>,
    pub(crate) failure_status: Option<String>,
    pub(crate) history: ContextHistory,
    pub(crate) flow_counter: u64,
    pub(crate) message_counter: u64,
    pub(crate) sequence: u64,
    pub(crate) session_id: String,
    pub(crate) sink: Option<&'a mut dyn RuntimeEventSink>,
    pub(crate) pending_context_manifest: Option<(ContextManifest, Vec<ContextObject>)>,
    pub(crate) tool_intents: Vec<PlannedToolIntent>,
    pub(crate) validation: Option<SessionAppendValidationState>,
}

impl<'a> RuntimeEventBuilder<'a> {
    pub(crate) fn with_clock(session_id: String, clock: EventClock, validate_plan: bool) -> Self {
        let validation = validate_plan.then(|| SessionAppendValidationState::empty(&session_id));
        Self {
            active_step_payloads: BTreeMap::new(),
            clock,
            context_manifests: RuntimeStreamSignatureBuilder::new(CONTEXT_PLAN_DOMAIN),
            events: RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN),
            failure_messages: BTreeMap::new(),
            failure_status: None,
            history: ContextHistory::default(),
            flow_counter: 0,
            message_counter: 0,
            sequence: 0,
            session_id,
            sink: None,
            pending_context_manifest: None,
            tool_intents: Vec::new(),
            validation,
        }
    }

    pub(crate) fn with_sink(
        session_id: String,
        clock: EventClock,
        validate_plan: bool,
        sink: &'a mut dyn RuntimeEventSink,
    ) -> Self {
        let mut builder = Self::with_clock(session_id, clock, validate_plan);
        builder.sink = Some(sink);
        builder
    }

    pub(crate) fn next_flow_invocation(
        &mut self,
        parent_flow_id: Option<String>,
    ) -> Result<FlowInvocation, RuntimeError> {
        let next_flow_counter = self.flow_counter + 1;
        // WHY: flow invocation budgets preserve duplicate subflow execution semantics while
        // bounding the total runtime work one session can request.
        if next_flow_counter > MAX_FLOW_INVOCATIONS {
            return Err(RuntimeError::Protocol(format!(
                "flow invocation budget exceeded: next invocation {next_flow_counter} exceeds max {MAX_FLOW_INVOCATIONS}"
            )));
        }
        self.flow_counter = next_flow_counter;
        Ok(FlowInvocation {
            flow_id: format!("flow-{:03}", self.flow_counter),
            parent_flow_id,
        })
    }

    pub(crate) fn next_message_id(&mut self) -> String {
        self.message_counter += 1;
        format!("msg-{:03}", self.message_counter)
    }

    pub(crate) fn record_tool_intent(
        &mut self,
        invocation: &FlowInvocation,
        tool: &core_script::ToolBlock,
        policy: RuntimeToolPolicy<'_>,
    ) -> Result<(), RuntimeError> {
        let canonical = proto::canonical_json(&serde_json::json!({
            "allowed_parameters": policy.command.allowed_parameters.iter().map(|parameter| parameter.name.clone()).collect::<Vec<_>>(),
            "command": tool.command,
            "flow_id": invocation.flow_id,
            "network_access": tool_network_access_name(&tool.network),
            "read_scope": policy.command.filesystem.read_roots,
            "tool_id": tool.identity.id,
            "tool_kind": policy_tool_kind_name(&policy.command.tool_kind),
            "write_scope": policy.command.filesystem.write_roots,
        }))
        .map_err(|error| {
            RuntimeError::Protocol(format!(
                "failed to serialize tool intent {}: {error}",
                tool.identity.id
            ))
        })?;
        self.tool_intents.push(PlannedToolIntent {
            canonical,
            flow_id: invocation.flow_id.clone(),
            tool_id: tool.identity.id.clone(),
        });
        Ok(())
    }

    pub(crate) fn record_context_manifest(
        &mut self,
        manifest: ContextManifest,
        objects: Vec<ContextObject>,
    ) -> Result<(), RuntimeError> {
        ensure_context_manifest_growth_within_limit(
            Path::new("runtime.contexts.jsonl"),
            self.context_manifests.byte_count,
            manifest.line.len(),
        )?;
        self.pending_context_manifest = Some((manifest, objects));
        Ok(())
    }

    pub(crate) fn emit(
        &mut self,
        invocation: Option<&FlowInvocation>,
        event_type: EventType,
        payload: serde_json::Value,
    ) -> Result<(), RuntimeError> {
        let sequence = self.sequence + 1;
        // WHY: enforce event budgets before storing the event so oversized in-cap flows
        // cannot accumulate unbounded memory.
        if sequence > MAX_FLOW_EVENTS {
            return Err(RuntimeError::Protocol(format!(
                "runtime event budget exceeded: next event {sequence} exceeds max {MAX_FLOW_EVENTS}"
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
            "flow-agent-cli",
            payload,
        );
        if let Some(invocation) = invocation {
            event.flow_id = Some(invocation.flow_id.clone());
            event.parent_flow_id = invocation.parent_flow_id.clone();
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
                ordinal: self.context_manifests.record_count.saturating_add(1),
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
                        .insert(invocation.flow_id.clone(), event.payload.clone());
                }
                EventType::StepCompleted => {
                    self.active_step_payloads.remove(&invocation.flow_id);
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(crate) fn into_execution(
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
            tool_intents: self.tool_intents,
        }
    }
}

pub fn plan_flow(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    root_flow: &core_script::FlowBlock,
    session_id: &str,
    options: FlowExecutionOptions,
) -> Result<FlowExecutionPlan, RuntimeError> {
    plan_flow_with_sink(
        workspace, registry, policy, root_flow, session_id, options, None,
    )
}

pub fn plan_flow_with_sink(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    root_flow: &core_script::FlowBlock,
    session_id: &str,
    options: FlowExecutionOptions,
    sink: Option<&mut dyn RuntimeEventSink>,
) -> Result<FlowExecutionPlan, RuntimeError> {
    if options.side_effect_mode != ToolSideEffectMode::Plan {
        return Err(RuntimeError::Protocol(
            "flow planning requires ToolSideEffectMode::Plan".to_owned(),
        ));
    }
    execute_flow_with_sink(
        workspace, registry, policy, root_flow, session_id, options, sink,
    )
    .map(FlowExecutionPlan::from_execution)
}

pub struct FlowApplication<'a> {
    pub(crate) workspace: &'a Path,
    pub(crate) registry: &'a core_script::ResolvedRegistry,
    pub(crate) policy: &'a core_policy::PolicyArtifact,
    pub(crate) root_flow: &'a core_script::FlowBlock,
    pub(crate) session_id: &'a str,
    pub(crate) options: FlowExecutionOptions,
    pub(crate) plan: &'a FlowExecutionPlan,
}

pub fn apply_flow_with_sink(
    application: FlowApplication<'_>,
    sink: Option<&mut dyn RuntimeEventSink>,
) -> Result<RuntimeExecution, RuntimeError> {
    if application.options.side_effect_mode == ToolSideEffectMode::Plan {
        return Err(RuntimeError::Protocol(
            "flow apply cannot use ToolSideEffectMode::Plan".to_owned(),
        ));
    }
    application.plan.validate_integrity()?;
    let execution = execute_flow_with_sink(
        application.workspace,
        application.registry,
        application.policy,
        application.root_flow,
        application.session_id,
        application.options,
        sink,
    )?;
    if !execution.failed && !execution.matches_plan(application.plan) {
        return Err(RuntimeError::Protocol(
            "flow apply did not match its execution plan".to_owned(),
        ));
    }
    Ok(execution)
}

pub fn execute_flow_with_sink(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    root_flow: &core_script::FlowBlock,
    session_id: &str,
    options: FlowExecutionOptions,
    sink: Option<&mut dyn RuntimeEventSink>,
) -> Result<RuntimeExecution, RuntimeError> {
    let validate_plan = options.side_effect_mode == ToolSideEffectMode::Plan;
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
    let start_payload = if options.stub_model_fixture_profile {
        serde_json::json!({"reason":"fixture-start"})
    } else {
        serde_json::json!({})
    };
    builder.emit(None, EventType::SessionStarted, start_payload)?;

    let context = FlowEmitContext {
        workspace,
        registry,
        policy,
        side_effect_mode: options.side_effect_mode,
        stub_model_fixture_profile: options.stub_model_fixture_profile,
        terminal_flow_ids: &options.terminal_flow_ids,
    };
    let failed = match emit_flow_block(&context, root_flow, None, &mut builder) {
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

pub fn should_terminalize_runtime_error(side_effect_mode: ToolSideEffectMode) -> bool {
    matches!(
        side_effect_mode,
        ToolSideEffectMode::Apply | ToolSideEffectMode::Resume { .. }
    )
}

pub fn should_terminalize_error(side_effect_mode: ToolSideEffectMode, err: &RuntimeError) -> bool {
    !matches!(err, RuntimeError::EventWriter(_))
        && (should_terminalize_runtime_error(side_effect_mode)
            || matches!(err, RuntimeError::ContextBudgetExceeded { .. }))
}

pub fn preflight_flow_tools(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    flow_block: &core_script::FlowBlock,
) -> Result<(), RuntimeError> {
    let mut invocation_count = 0;
    preflight_flow_tools_at_depth(
        workspace,
        registry,
        policy,
        flow_block,
        1,
        &mut invocation_count,
    )
}

pub fn preflight_flow_tools_at_depth(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    flow_block: &core_script::FlowBlock,
    depth: usize,
    invocation_count: &mut u64,
) -> Result<(), RuntimeError> {
    *invocation_count = invocation_count.checked_add(1).ok_or_else(|| {
        RuntimeError::Protocol("flow invocation budget counter overflowed".to_owned())
    })?;
    if *invocation_count > MAX_FLOW_INVOCATIONS {
        return Err(RuntimeError::Protocol(format!(
            "flow invocation budget exceeded: next invocation {invocation_count} exceeds max {MAX_FLOW_INVOCATIONS}"
        )));
    }
    if depth > core_script::MAX_FLOW_NESTING_DEPTH {
        return Err(RuntimeError::Protocol(format!(
            "flow nesting depth {depth} for {} exceeds max {}",
            flow_block.identity.id,
            core_script::MAX_FLOW_NESTING_DEPTH
        )));
    }

    for phase_ref in &flow_block.phase_refs {
        let phase = registry.phase_block(phase_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing phase {phase_ref}"))
        })?;
        preflight_phase_tools(workspace, registry, policy, phase)?;
    }

    for subflow_ref in &flow_block.subflow_refs {
        let subflow = registry.flow_block(subflow_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing flow {subflow_ref}"))
        })?;
        preflight_flow_tools_at_depth(
            workspace,
            registry,
            policy,
            subflow,
            depth + 1,
            invocation_count,
        )?;
    }

    Ok(())
}

pub fn preflight_phase_tools(
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

pub fn emit_flow_block(
    context: &FlowEmitContext<'_>,
    flow_block: &core_script::FlowBlock,
    parent_flow_id: Option<String>,
    builder: &mut RuntimeEventBuilder<'_>,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    emit_flow_block_at_depth(context, flow_block, parent_flow_id, builder, 1)
}

pub struct FlowEmitContext<'a> {
    pub(crate) workspace: &'a Path,
    pub(crate) registry: &'a core_script::ResolvedRegistry,
    pub(crate) policy: &'a core_policy::PolicyArtifact,
    pub(crate) side_effect_mode: ToolSideEffectMode,
    pub(crate) stub_model_fixture_profile: bool,
    pub(crate) terminal_flow_ids: &'a BTreeSet<String>,
}

pub fn emit_flow_block_at_depth(
    context: &FlowEmitContext<'_>,
    flow_block: &core_script::FlowBlock,
    parent_flow_id: Option<String>,
    builder: &mut RuntimeEventBuilder<'_>,
    depth: usize,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    if depth > core_script::MAX_FLOW_NESTING_DEPTH {
        return Err(RuntimeError::Protocol(format!(
            "flow nesting depth {depth} for {} exceeds max {}",
            flow_block.identity.id,
            core_script::MAX_FLOW_NESTING_DEPTH
        )));
    }

    let invocation = builder.next_flow_invocation(parent_flow_id)?;
    // A parent remains live while it waits for a nested invocation; queued but not started,
    // terminal and fully paused invocations do not hold this process-wide slot.
    let _live_invocation = context
        .side_effect_mode
        .occupies_live_invocation_slot(context.terminal_flow_ids.contains(&invocation.flow_id))
        .then(|| LIVE_FLOW_INVOCATIONS.acquire())
        .transpose()?;
    builder.emit(
        Some(&invocation),
        EventType::FlowStarted,
        serde_json::json!({
            "flow_definition_id": flow_block.identity.id,
            "flow_name": flow_block.identity.name,
        }),
    )?;

    for phase_ref in &flow_block.phase_refs {
        let phase = context.registry.phase_block(phase_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing phase {phase_ref}"))
        })?;
        match emit_phase(context, flow_block, phase, &invocation, builder) {
            Ok(Some(failure)) => {
                emit_runtime_failure(flow_block, &invocation, &failure, builder)?;
                return Ok(Some(failure));
            }
            Ok(None) => {}
            Err(err) if should_terminalize_error(context.side_effect_mode, &err) => {
                emit_runtime_error_failure(flow_block, &invocation, &err, builder)?;
                return Err(err);
            }
            Err(err) => return Err(err),
        }
    }

    for subflow_ref in &flow_block.subflow_refs {
        let subflow = context.registry.flow_block(subflow_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing flow {subflow_ref}"))
        })?;
        match emit_flow_block_at_depth(
            context,
            subflow,
            Some(invocation.flow_id.clone()),
            builder,
            depth + 1,
        ) {
            Ok(Some(failure)) => {
                emit_runtime_flow_failure(flow_block, &invocation, &failure.reason, builder)?;
                return Ok(Some(failure));
            }
            Ok(None) => {}
            Err(err) if should_terminalize_error(context.side_effect_mode, &err) => {
                let reason = runtime_failure_for_unhandled_error(&err).reason;
                emit_runtime_flow_failure(flow_block, &invocation, &reason, builder)?;
                return Err(err);
            }
            Err(err) => return Err(err),
        }
    }

    builder.emit(
        Some(&invocation),
        EventType::FlowCompleted,
        serde_json::json!({
            "flow_definition_id": flow_block.identity.id,
            "flow_name": flow_block.identity.name,
        }),
    )?;
    Ok(None)
}

pub fn emit_phase(
    context: &FlowEmitContext<'_>,
    flow_block: &core_script::FlowBlock,
    phase: &core_script::PhaseBlock,
    invocation: &FlowInvocation,
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

        if step_index == 0
            && let Some(failure) = sandbox_out_of_phase_failure(
                context.registry,
                context.policy,
                phase,
                context.stub_model_fixture_profile,
            )
        {
            builder.emit(Some(invocation), EventType::StepCompleted, step_payload)?;
            return Ok(Some(failure));
        }

        if phase_uses_stub_model(phase) {
            let compiled = compile_provider_turn_context(
                context.registry,
                flow_block,
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

pub fn step_payload(
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

pub fn phase_uses_stub_model(phase: &core_script::PhaseBlock) -> bool {
    !phase.instruction_refs.is_empty()
}

pub fn stub_message_content(
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

pub fn command_policy_for_phase<'a>(
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
