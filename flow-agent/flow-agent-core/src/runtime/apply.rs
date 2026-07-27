use crate::runtime::{
    event_construction::{FlowInvocation, RuntimeStreamSignatureBuilder},
    event_writer::RuntimeEventSink,
    failures::{runtime_failure_for_tool_error, runtime_failure_for_unhandled_error},
    fixture_effects::{apply_planned_fixture_effect, preflight_planned_fixture_effect},
    fs_guards::AnchoredWorkspace,
    planning::{
        CONTEXT_PLAN_DOMAIN, EVENT_PLAN_DOMAIN, FlowExecutionAction, FlowExecutionOptions,
        FlowExecutionPlan, PlannedFixtureAction, PlannedFlowFailureBoundary, RuntimeExecution,
        ToolSideEffectMode,
    },
    types::{EventClock, MAX_LIVE_FLOW_INVOCATIONS, RuntimeError, render_human_failure_status},
    validate::validate_event_size,
};
use proto::{EventEnvelope, EventType};
use std::{
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};

static LIVE_FLOW_INVOCATIONS: LiveInvocationCounter = LiveInvocationCounter::new();

struct LiveInvocationCounter {
    count: AtomicUsize,
}

impl LiveInvocationCounter {
    const fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
        }
    }

    fn acquire(&self) -> Result<LiveInvocationGuard<'_>, RuntimeError> {
        let mut observed = self.count.load(Ordering::Acquire);
        loop {
            if observed >= MAX_LIVE_FLOW_INVOCATIONS {
                return Err(RuntimeError::Protocol(format!(
                    "global live flow invocation limit reached: max {MAX_LIVE_FLOW_INVOCATIONS}"
                )));
            }
            match self.count.compare_exchange_weak(
                observed,
                observed + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(LiveInvocationGuard { counter: self }),
                Err(actual) => observed = actual,
            }
        }
    }
}

struct LiveInvocationGuard<'a> {
    counter: &'a LiveInvocationCounter,
}

impl Drop for LiveInvocationGuard<'_> {
    fn drop(&mut self) {
        self.counter.count.fetch_sub(1, Ordering::AcqRel);
    }
}

struct ActiveLiveInvocation {
    boundary: PlannedFlowFailureBoundary,
    _guard: Option<LiveInvocationGuard<'static>>,
}

struct LiveFlowInvocations {
    active: Vec<ActiveLiveInvocation>,
    enabled: bool,
    prefix_event_count: u64,
}

impl LiveFlowInvocations {
    fn for_application(
        plan: &FlowExecutionPlan,
        side_effect_mode: ToolSideEffectMode,
    ) -> Result<Self, RuntimeError> {
        let Some(prefix_event_count) = (match side_effect_mode {
            ToolSideEffectMode::Apply => Some(0),
            ToolSideEffectMode::Resume { prefix_event_count } => Some(prefix_event_count),
            ToolSideEffectMode::Plan | ToolSideEffectMode::PreflightResume { .. } => None,
        }) else {
            return Ok(Self {
                active: Vec::new(),
                enabled: false,
                prefix_event_count: 0,
            });
        };
        let mut tracker = Self {
            active: Vec::new(),
            enabled: true,
            prefix_event_count,
        };
        for action in &plan.actions {
            let FlowExecutionAction::Event(action) = action else {
                continue;
            };
            if action.event.sequence > prefix_event_count {
                break;
            }
            tracker.reconstruct_prefix_event(&action.event)?;
        }
        for active in &mut tracker.active {
            active._guard = Some(LIVE_FLOW_INVOCATIONS.acquire()?);
        }
        Ok(tracker)
    }

    fn reconstruct_prefix_event(&mut self, event: &EventEnvelope) -> Result<(), RuntimeError> {
        match event.event_type {
            EventType::FlowStarted => self.active.push(ActiveLiveInvocation {
                boundary: flow_boundary(event)?,
                _guard: None,
            }),
            EventType::FlowCompleted | EventType::FlowFailed => self.finish(event),
            _ => {}
        }
        Ok(())
    }

    fn before_event(&mut self, event: &EventEnvelope) -> Result<(), RuntimeError> {
        if !self.enabled || event.sequence <= self.prefix_event_count {
            return Ok(());
        }
        if event.event_type == EventType::FlowStarted {
            self.active.push(ActiveLiveInvocation {
                boundary: flow_boundary(event)?,
                _guard: Some(LIVE_FLOW_INVOCATIONS.acquire()?),
            });
        }
        Ok(())
    }

    fn after_event(&mut self, event: &EventEnvelope) {
        if self.enabled
            && event.sequence > self.prefix_event_count
            && matches!(
                event.event_type,
                EventType::FlowCompleted | EventType::FlowFailed
            )
        {
            self.finish(event);
        }
    }

    fn finish(&mut self, event: &EventEnvelope) {
        let Some(flow_id) = event.flow_id.as_deref() else {
            return;
        };
        if let Some(index) = self
            .active
            .iter()
            .rposition(|active| active.boundary.flow_id == flow_id)
        {
            self.active.remove(index);
        }
    }

    fn active_boundaries(&self) -> Vec<PlannedFlowFailureBoundary> {
        self.active
            .iter()
            .map(|active| active.boundary.clone())
            .collect()
    }
}

fn flow_boundary(event: &EventEnvelope) -> Result<PlannedFlowFailureBoundary, RuntimeError> {
    let flow_id = event.flow_id.clone().ok_or_else(|| {
        RuntimeError::Protocol("flow.started action is missing flow_id".to_owned())
    })?;
    let flow_definition_id = event
        .payload
        .get("flow_definition_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            RuntimeError::Protocol(
                "flow.started action is missing payload.flow_definition_id".to_owned(),
            )
        })?;
    Ok(PlannedFlowFailureBoundary {
        flow_definition_id: flow_definition_id.to_owned(),
        flow_id,
        parent_flow_id: event.parent_flow_id.clone(),
    })
}

pub struct FlowApplication<'a> {
    #[cfg(test)]
    pub(crate) workspace: &'a Path,
    pub(crate) session_id: &'a str,
    pub(crate) options: FlowExecutionOptions,
    pub(crate) plan: &'a FlowExecutionPlan,
}

#[cfg(test)]
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
    let execution_workspace = AnchoredWorkspace::open(application.workspace)?;
    apply_flow_with_workspace(application, &execution_workspace, sink)
}

pub(crate) fn apply_flow_with_anchored_workspace(
    application: FlowApplication<'_>,
    execution_workspace: &AnchoredWorkspace,
    sink: Option<&mut dyn RuntimeEventSink>,
) -> Result<RuntimeExecution, RuntimeError> {
    if application.options.side_effect_mode == ToolSideEffectMode::Plan {
        return Err(RuntimeError::Protocol(
            "flow apply cannot use ToolSideEffectMode::Plan".to_owned(),
        ));
    }
    application.plan.validate_integrity()?;
    apply_flow_with_workspace(application, execution_workspace, sink)
}

pub(crate) fn preflight_flow_execution_plan(
    plan: &FlowExecutionPlan,
    execution_workspace: &AnchoredWorkspace,
    side_effect_mode: ToolSideEffectMode,
) -> Result<(), RuntimeError> {
    plan.validate_integrity()?;
    execution_workspace.verify_identity(plan.workspace_identity())?;
    for action in &plan.actions {
        if let FlowExecutionAction::Fixture(action) = action
            && (side_effect_mode.should_execute_tool(action.completion_sequence)
                || side_effect_mode.should_preflight_tool(action.completion_sequence))
        {
            preflight_planned_fixture_effect(execution_workspace.root(), action)?;
        }
    }
    Ok(())
}

fn apply_flow_with_workspace(
    application: FlowApplication<'_>,
    execution_workspace: &AnchoredWorkspace,
    sink: Option<&mut dyn RuntimeEventSink>,
) -> Result<RuntimeExecution, RuntimeError> {
    preflight_flow_execution_plan(
        application.plan,
        execution_workspace,
        application.options.side_effect_mode,
    )?;
    let planned_session_id = application
        .plan
        .actions
        .iter()
        .find_map(|action| match action {
            FlowExecutionAction::Event(action) => Some(action.event.session_id.as_str()),
            FlowExecutionAction::Fixture(_) => None,
        })
        .ok_or_else(|| RuntimeError::Protocol("flow execution plan has no events".to_owned()))?;
    if planned_session_id != application.session_id {
        return Err(RuntimeError::Protocol(
            "flow apply did not match its execution plan".to_owned(),
        ));
    }
    let mut live_invocations = LiveFlowInvocations::for_application(
        application.plan,
        application.options.side_effect_mode,
    )?;
    let mut sink = sink;
    let mut event_signature = RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN);
    let mut context_signature = RuntimeStreamSignatureBuilder::new(CONTEXT_PLAN_DOMAIN);
    for action in &application.plan.actions {
        match action {
            FlowExecutionAction::Event(action) => {
                if let Err(error) = live_invocations.before_event(&action.event) {
                    return terminalize_live_invocation_error(
                        &application,
                        error,
                        &mut sink,
                        event_signature,
                        context_signature,
                        &mut live_invocations,
                    );
                }
                if let Some(sink) = sink.as_deref_mut() {
                    let measurement_started_at = sink.measurement_started_at();
                    sink.commit(
                        &action.event,
                        &action.canonical_jsonl,
                        action.context_checkpoint.clone(),
                        measurement_started_at,
                    )?;
                }
                event_signature.push(action.canonical_jsonl.as_bytes());
                if let Some(checkpoint) = &action.context_checkpoint {
                    context_signature.push(checkpoint.manifest.line.as_bytes());
                }
                live_invocations.after_event(&action.event);
            }
            FlowExecutionAction::Fixture(action)
                if application
                    .options
                    .side_effect_mode
                    .should_execute_tool(action.completion_sequence) =>
            {
                execution_workspace.verify_binding()?;
                if let Err(error) = apply_planned_fixture_effect(execution_workspace.root(), action)
                {
                    return terminalize_planned_fixture_error(
                        &application,
                        action,
                        error,
                        &mut sink,
                        event_signature,
                        context_signature,
                        &mut live_invocations,
                    );
                }
            }
            FlowExecutionAction::Fixture(_) => {}
        }
    }
    debug_assert!(live_invocations.active.is_empty());
    Ok(RuntimeExecution {
        actions: application.plan.actions.clone(),
        context_manifests: context_signature.signature(),
        #[cfg(test)]
        event_transition_nanos: Vec::new(),
        events: event_signature.signature(),
        failed: application.plan.execution.failed,
        failure_status: application.plan.execution.failure_status.clone(),
        terminal_error: clone_planned_terminal_error(
            application.plan.execution.terminal_error.as_ref(),
        ),
        tool_intents: application.plan.execution.tool_intents.clone(),
    })
}

fn terminalize_planned_fixture_error(
    application: &FlowApplication<'_>,
    action: &PlannedFixtureAction,
    error: RuntimeError,
    sink: &mut Option<&mut dyn RuntimeEventSink>,
    mut event_signature: RuntimeStreamSignatureBuilder,
    context_signature: RuntimeStreamSignatureBuilder,
    live_invocations: &mut LiveFlowInvocations,
) -> Result<RuntimeExecution, RuntimeError> {
    let mapped_failure = runtime_failure_for_tool_error(&error, &action.failure_transition.tool_id);
    let known_tool_failure = mapped_failure.is_some();
    let mut failure = mapped_failure.unwrap_or_else(|| runtime_failure_for_unhandled_error(&error));
    failure.tool_id = Some(action.failure_transition.tool_id.clone());
    let invocation = FlowInvocation {
        flow_id: action.failure_transition.flow_id.clone(),
        parent_flow_id: action.failure_transition.parent_flow_id.clone(),
    };
    let failure_events = vec![
        (
            Some(&invocation),
            EventType::ToolFailed,
            serde_json::json!({
                "error": failure.reason,
                "tool_id": action.failure_transition.tool_id,
            }),
        ),
        (
            Some(&invocation),
            EventType::StepCompleted,
            action.failure_transition.step_payload.clone(),
        ),
    ];
    let mut error_payload = serde_json::json!({
        "code": failure.reason,
        "message": failure.message,
    });
    if !failure.data.is_empty() {
        error_payload
            .as_object_mut()
            .expect("planned error payload is an object")
            .insert(
                "data".to_owned(),
                serde_json::Value::Object(failure.data.clone()),
            );
    }
    let current_flow_failure_events = [
        (Some(&invocation), EventType::Error, error_payload),
        (
            Some(&invocation),
            EventType::FlowFailed,
            serde_json::json!({
                "error": failure.reason,
                "flow_definition_id": action.failure_transition.flow_definition_id,
            }),
        ),
    ];
    let mut transition_state = PlannedTransitionState {
        sink,
        event_signature: &mut event_signature,
        live_invocations,
    };
    for (invocation, event_type, payload) in failure_events {
        commit_planned_transition_event(
            application.session_id,
            application.options.clock,
            invocation,
            event_type,
            payload,
            &mut transition_state,
        )?;
    }
    for (invocation, event_type, payload) in current_flow_failure_events {
        commit_planned_transition_event(
            application.session_id,
            application.options.clock,
            invocation,
            event_type,
            payload,
            &mut transition_state,
        )?;
    }
    for boundary in action.failure_transition.ancestor_flows.iter().rev() {
        let invocation = FlowInvocation {
            flow_id: boundary.flow_id.clone(),
            parent_flow_id: boundary.parent_flow_id.clone(),
        };
        commit_planned_transition_event(
            application.session_id,
            application.options.clock,
            Some(&invocation),
            EventType::FlowFailed,
            serde_json::json!({
                "error": failure.reason,
                "flow_definition_id": boundary.flow_definition_id,
            }),
            &mut transition_state,
        )?;
    }
    let failure_events = [(
        None,
        EventType::SessionFailed,
        serde_json::json!({"reason": failure.reason}),
    )];
    for (invocation, event_type, payload) in failure_events {
        commit_planned_transition_event(
            application.session_id,
            application.options.clock,
            invocation,
            event_type,
            payload,
            &mut transition_state,
        )?;
    }
    let failure_status = Some(render_human_failure_status(
        &failure.reason,
        Some(failure.message),
    ));
    Ok(RuntimeExecution {
        actions: application.plan.actions.clone(),
        context_manifests: context_signature.signature(),
        #[cfg(test)]
        event_transition_nanos: Vec::new(),
        events: event_signature.signature(),
        failed: true,
        failure_status,
        terminal_error: if matches!(
            application.options.side_effect_mode,
            ToolSideEffectMode::Resume { .. }
        ) || !known_tool_failure
        {
            Some(error)
        } else {
            None
        },
        tool_intents: application.plan.execution.tool_intents.clone(),
    })
}

fn terminalize_live_invocation_error(
    application: &FlowApplication<'_>,
    error: RuntimeError,
    sink: &mut Option<&mut dyn RuntimeEventSink>,
    mut event_signature: RuntimeStreamSignatureBuilder,
    context_signature: RuntimeStreamSignatureBuilder,
    live_invocations: &mut LiveFlowInvocations,
) -> Result<RuntimeExecution, RuntimeError> {
    let failure = runtime_failure_for_unhandled_error(&error);
    let active_boundaries = live_invocations.active_boundaries();
    let mut transition_state = PlannedTransitionState {
        sink,
        event_signature: &mut event_signature,
        live_invocations,
    };
    for boundary in active_boundaries.iter().rev() {
        let invocation = FlowInvocation {
            flow_id: boundary.flow_id.clone(),
            parent_flow_id: boundary.parent_flow_id.clone(),
        };
        commit_planned_transition_event(
            application.session_id,
            application.options.clock,
            Some(&invocation),
            EventType::FlowFailed,
            serde_json::json!({
                "error": failure.reason,
                "flow_definition_id": boundary.flow_definition_id,
            }),
            &mut transition_state,
        )?;
    }
    commit_planned_transition_event(
        application.session_id,
        application.options.clock,
        None,
        EventType::SessionFailed,
        serde_json::json!({"reason":failure.reason}),
        &mut transition_state,
    )?;
    Ok(RuntimeExecution {
        actions: application.plan.actions.clone(),
        context_manifests: context_signature.signature(),
        #[cfg(test)]
        event_transition_nanos: Vec::new(),
        events: event_signature.signature(),
        failed: true,
        failure_status: Some(render_human_failure_status(
            &failure.reason,
            Some(failure.message),
        )),
        terminal_error: Some(error),
        tool_intents: application.plan.execution.tool_intents.clone(),
    })
}

struct PlannedTransitionState<'state, 'sink> {
    sink: &'state mut Option<&'sink mut dyn RuntimeEventSink>,
    event_signature: &'state mut RuntimeStreamSignatureBuilder,
    live_invocations: &'state mut LiveFlowInvocations,
}

fn commit_planned_transition_event(
    session_id: &str,
    clock: EventClock,
    invocation: Option<&FlowInvocation>,
    event_type: EventType,
    payload: serde_json::Value,
    state: &mut PlannedTransitionState<'_, '_>,
) -> Result<(), RuntimeError> {
    let sequence = u64::try_from(state.event_signature.record_count)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut event = EventEnvelope::new(
        format!("evt-{sequence:03}"),
        event_type,
        session_id,
        sequence,
        clock.timestamp(sequence),
        "flow-agent-cli",
        payload,
    );
    if let Some(invocation) = invocation {
        event.flow_id = Some(invocation.flow_id.clone());
        event.parent_flow_id = invocation.parent_flow_id.clone();
    }
    event.validate_v0().map_err(|error| {
        RuntimeError::Protocol(format!("constructed runtime event is invalid: {error}"))
    })?;
    let canonical_jsonl = event.canonical_jsonl().map_err(|error| {
        RuntimeError::Protocol(format!("failed to serialize runtime event: {error}"))
    })?;
    validate_event_size(
        Path::new("runtime.jsonl"),
        sequence as usize,
        canonical_jsonl.len(),
    )?;
    state.live_invocations.before_event(&event)?;
    if let Some(sink) = state.sink.as_deref_mut() {
        let measurement_started_at = sink.measurement_started_at();
        sink.commit(&event, &canonical_jsonl, None, measurement_started_at)?;
    }
    state.event_signature.push(canonical_jsonl.as_bytes());
    state.live_invocations.after_event(&event);
    Ok(())
}

fn clone_planned_terminal_error(error: Option<&RuntimeError>) -> Option<RuntimeError> {
    error.map(|error| match error {
        RuntimeError::ContextBudgetExceeded {
            input_budget_tokens,
            required_bytes,
        } => RuntimeError::ContextBudgetExceeded {
            input_budget_tokens: *input_budget_tokens,
            required_bytes: *required_bytes,
        },
        _ => RuntimeError::Protocol(error.to_string()),
    })
}
