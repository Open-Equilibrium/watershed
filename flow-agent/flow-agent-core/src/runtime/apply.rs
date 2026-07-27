use crate::runtime::{
    event_construction::{FlowInvocation, RuntimeStreamSignatureBuilder},
    event_writer::RuntimeEventSink,
    failures::{runtime_failure_for_tool_error, runtime_failure_for_unhandled_error},
    fixture_effects::{apply_planned_fixture_effect, preflight_planned_fixture_effect},
    fs_guards::AnchoredWorkspace,
    planning::{
        CONTEXT_PLAN_DOMAIN, EVENT_PLAN_DOMAIN, FlowExecutionAction, FlowExecutionOptions,
        FlowExecutionPlan, PlannedFixtureAction, RuntimeExecution, ToolSideEffectMode,
    },
    types::{EventClock, RuntimeError, render_human_failure_status},
    validate::validate_event_size,
};
use proto::{EventEnvelope, EventType};
use std::path::Path;

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
    let mut sink = sink;
    let mut event_signature = RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN);
    let mut context_signature = RuntimeStreamSignatureBuilder::new(CONTEXT_PLAN_DOMAIN);
    for action in &application.plan.actions {
        match action {
            FlowExecutionAction::Event(action) => {
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
                    );
                }
            }
            FlowExecutionAction::Fixture(_) => {}
        }
    }
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
    for (invocation, event_type, payload) in failure_events {
        commit_planned_transition_event(
            application.session_id,
            application.options.clock,
            invocation,
            event_type,
            payload,
            sink,
            &mut event_signature,
        )?;
    }
    for (invocation, event_type, payload) in current_flow_failure_events {
        commit_planned_transition_event(
            application.session_id,
            application.options.clock,
            invocation,
            event_type,
            payload,
            sink,
            &mut event_signature,
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
            sink,
            &mut event_signature,
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
            sink,
            &mut event_signature,
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

fn commit_planned_transition_event(
    session_id: &str,
    clock: EventClock,
    invocation: Option<&FlowInvocation>,
    event_type: EventType,
    payload: serde_json::Value,
    sink: &mut Option<&mut dyn RuntimeEventSink>,
    event_signature: &mut RuntimeStreamSignatureBuilder,
) -> Result<(), RuntimeError> {
    let sequence = u64::try_from(event_signature.record_count)
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
    if let Some(sink) = sink.as_deref_mut() {
        let measurement_started_at = sink.measurement_started_at();
        sink.commit(&event, &canonical_jsonl, None, measurement_started_at)?;
    }
    event_signature.push(canonical_jsonl.as_bytes());
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
