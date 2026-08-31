use crate::runtime::{
    event_construction::{
        ConstructedRuntimeEvent, PlannedRuntimeEvent, RuntimeEventAlternative,
        construct_runtime_transition, fixture_failure_transition_events,
        live_invocation_failure_transition_events, validate_runtime_transition_capacity,
    },
    event_writer::RuntimeEventSink,
    execution_plan::{
        FlowExecutionAction, FlowExecutionOptions, FlowExecutionPlan, PlannedFixtureAction,
        RuntimeExecution, ToolSideEffectMode,
    },
    failures::{
        fixture_failure_capacity_candidates, runtime_failure_for_tool_error,
        runtime_failure_for_unhandled_error,
    },
    fixture_effects::{apply_planned_fixture_effect, preflight_planned_fixture_effect},
    fs_guards::AnchoredWorkspace,
    live_flow_invocations::LiveFlowInvocations,
    stream_signature::{CONTEXT_PLAN_DOMAIN, EVENT_PLAN_DOMAIN, RuntimeStreamSignatureBuilder},
    types::{RuntimeError, render_human_failure_status},
};
use proto::EventType;
#[cfg(test)]
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
    validate_flow_application(&application)?;
    let execution_workspace = AnchoredWorkspace::open(application.workspace)?;
    apply_flow_with_workspace(application, &execution_workspace, sink)
}

pub(crate) fn apply_flow_with_anchored_workspace(
    application: FlowApplication<'_>,
    execution_workspace: &AnchoredWorkspace,
    sink: Option<&mut dyn RuntimeEventSink>,
) -> Result<RuntimeExecution, RuntimeError> {
    validate_flow_application(&application)?;
    apply_flow_with_workspace(application, execution_workspace, sink)
}

fn validate_flow_application(application: &FlowApplication<'_>) -> Result<(), RuntimeError> {
    if application.options.side_effect_mode == ToolSideEffectMode::Plan {
        return Err(RuntimeError::Protocol(
            "flow apply cannot use ToolSideEffectMode::Plan".to_owned(),
        ));
    }
    application.plan.validate_integrity()
}

pub(crate) fn preflight_flow_execution_plan(
    plan: &FlowExecutionPlan,
    execution_workspace: &AnchoredWorkspace,
    side_effect_mode: ToolSideEffectMode,
) -> Result<(), RuntimeError> {
    plan.validate_integrity()?;
    execution_workspace.verify_identity(plan.workspace_identity())?;
    for action in plan.actions.iter() {
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
    for action in application.plan.actions.iter() {
        match action {
            FlowExecutionAction::Event(action) => {
                if action.event.event_type == EventType::FlowStarted
                    && live_invocations.should_process(&action.event)
                {
                    preflight_live_invocation_failure_transition(
                        &application,
                        &mut sink,
                        &event_signature,
                        &live_invocations,
                    )?;
                }
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
                if live_invocations.should_process(&action.event)
                    && let Some(sink) = sink.as_deref_mut()
                {
                    sink.commit(
                        &action.event,
                        &action.canonical_jsonl,
                        action.context_checkpoint.clone(),
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
                    .should_execute_tool(action.completion_sequence)
                    || application
                        .options
                        .side_effect_mode
                        .should_preflight_tool(action.completion_sequence) =>
            {
                preflight_fixture_failure_transitions(
                    &application,
                    action,
                    &mut sink,
                    &event_signature,
                )?;
                if application
                    .options
                    .side_effect_mode
                    .should_execute_tool(action.completion_sequence)
                {
                    let apply_result = execution_workspace.verify_binding().and_then(|()| {
                        apply_planned_fixture_effect(execution_workspace.root(), action)
                    });
                    if let Err(error) = apply_result {
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
            }
            FlowExecutionAction::Fixture(_) => {}
        }
    }
    debug_assert!(live_invocations.is_empty());
    Ok(RuntimeExecution {
        actions: application.plan.actions.clone(),
        context_manifests: context_signature.signature(),
        events: event_signature.signature(),
        failed: application.plan.execution.failed,
        failure_status: application.plan.execution.failure_status.clone(),
        terminal_error: clone_planned_terminal_error(
            application.plan.execution.terminal_error.as_ref(),
        ),
        tool_intents: application.plan.execution.tool_intents.clone(),
    })
}

fn preflight_fixture_failure_transitions(
    application: &FlowApplication<'_>,
    action: &PlannedFixtureAction,
    sink: &mut Option<&mut dyn RuntimeEventSink>,
    event_signature: &RuntimeStreamSignatureBuilder,
) -> Result<(), RuntimeError> {
    let Some(sink) = sink.as_deref_mut() else {
        return Ok(());
    };
    if !sink.needs_alternative_preflight() {
        return Ok(());
    }
    let mut alternatives = Vec::new();
    for failure in fixture_failure_capacity_candidates() {
        alternatives.push(construct_runtime_alternative(
            application,
            event_signature,
            fixture_failure_transition_events(&action.failure_transition, &failure),
            "runtime failure transition",
        )?);
    }
    sink.preflight_alternatives(&alternatives)
}

fn preflight_live_invocation_failure_transition(
    application: &FlowApplication<'_>,
    sink: &mut Option<&mut dyn RuntimeEventSink>,
    event_signature: &RuntimeStreamSignatureBuilder,
    live_invocations: &LiveFlowInvocations,
) -> Result<(), RuntimeError> {
    let Some(sink) = sink.as_deref_mut() else {
        return Ok(());
    };
    if !sink.needs_alternative_preflight() {
        return Ok(());
    }
    let failure = runtime_failure_for_unhandled_error(&RuntimeError::Protocol(
        "global live flow invocation limit reached".to_owned(),
    ));
    let alternative = construct_runtime_alternative(
        application,
        event_signature,
        live_invocation_failure_transition_events(&live_invocations.active_boundaries(), &failure),
        "live invocation failure transition",
    )?;
    sink.preflight_alternatives(&[alternative])
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
    let alternative = construct_runtime_alternative(
        application,
        &event_signature,
        fixture_failure_transition_events(&action.failure_transition, &failure),
        "runtime failure transition",
    )?;
    let mut transition_state = PlannedTransitionState {
        sink,
        event_signature: &mut event_signature,
        live_invocations,
    };
    commit_constructed_transition(&alternative.events, &mut transition_state)?;
    let failure_status = Some(render_human_failure_status(
        &failure.reason,
        Some(failure.message),
    ));
    Ok(RuntimeExecution {
        actions: application.plan.actions.clone(),
        context_manifests: context_signature.signature(),
        events: event_signature.signature(),
        failed: true,
        failure_status,
        terminal_error: if !known_tool_failure {
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
    let alternative = construct_runtime_alternative(
        application,
        &event_signature,
        live_invocation_failure_transition_events(&active_boundaries, &failure),
        "live invocation failure transition",
    )?;
    let mut transition_state = PlannedTransitionState {
        sink,
        event_signature: &mut event_signature,
        live_invocations,
    };
    commit_constructed_transition(&alternative.events, &mut transition_state)?;
    Ok(RuntimeExecution {
        actions: application.plan.actions.clone(),
        context_manifests: context_signature.signature(),
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

fn construct_runtime_alternative(
    application: &FlowApplication<'_>,
    event_signature: &RuntimeStreamSignatureBuilder,
    planned: Vec<PlannedRuntimeEvent>,
    label: &'static str,
) -> Result<RuntimeEventAlternative, RuntimeError> {
    let alternative = RuntimeEventAlternative {
        events: construct_runtime_transition(
            application.session_id,
            application.options.clock,
            u64::try_from(event_signature.record_count).unwrap_or(u64::MAX),
            planned,
        )?,
        label,
    };
    validate_runtime_transition_capacity(
        event_signature.record_count,
        event_signature.byte_count,
        &alternative,
    )?;
    Ok(alternative)
}

struct PlannedTransitionState<'state, 'sink> {
    sink: &'state mut Option<&'sink mut dyn RuntimeEventSink>,
    event_signature: &'state mut RuntimeStreamSignatureBuilder,
    live_invocations: &'state mut LiveFlowInvocations,
}

fn commit_constructed_transition(
    events: &[ConstructedRuntimeEvent],
    state: &mut PlannedTransitionState<'_, '_>,
) -> Result<(), RuntimeError> {
    for constructed in events {
        state.live_invocations.before_event(&constructed.event)?;
        if let Some(sink) = state.sink.as_deref_mut() {
            sink.commit(&constructed.event, &constructed.canonical_jsonl, None)?;
        }
        state
            .event_signature
            .push(constructed.canonical_jsonl.as_bytes());
        state.live_invocations.after_event(&constructed.event);
    }
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
