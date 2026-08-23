use crate::runtime::{
    event_construction::RuntimeEventBuilder,
    execution_plan::{
        FlowExecutionOptions, FlowExecutionPlan, RuntimeExecution, ToolSideEffectMode,
    },
    failures::runtime_failure_for_unhandled_error,
    fs_guards::AnchoredWorkspace,
    types::RuntimeError,
};
use proto::EventType;
#[cfg(test)]
use std::path::Path;

mod execution;

#[cfg(test)]
pub(crate) use execution::emit_planned_tool;
use execution::{ExecutionOutcome, FlowEmitContext, emit_flow_block};

#[cfg(test)]
pub fn plan_flow(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    root_flow: &core_script::FlowBlock,
    session_id: &str,
    options: FlowExecutionOptions,
) -> Result<FlowExecutionPlan, RuntimeError> {
    let workspace = AnchoredWorkspace::open(workspace)?;
    plan_flow_with_workspace(&workspace, registry, policy, root_flow, session_id, options)
}

pub(crate) fn plan_flow_with_workspace(
    workspace: &AnchoredWorkspace,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    root_flow: &core_script::FlowBlock,
    session_id: &str,
    options: FlowExecutionOptions,
) -> Result<FlowExecutionPlan, RuntimeError> {
    if options.side_effect_mode != ToolSideEffectMode::Plan {
        return Err(RuntimeError::Protocol(
            "flow planning requires ToolSideEffectMode::Plan".to_owned(),
        ));
    }
    compile_flow_plan(registry, policy, root_flow, session_id, options)
        .map(|execution| FlowExecutionPlan::from_execution(execution, workspace.identity()))
}

fn compile_flow_plan(
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    root_flow: &core_script::FlowBlock,
    session_id: &str,
    options: FlowExecutionOptions,
) -> Result<RuntimeExecution, RuntimeError> {
    let mut builder = RuntimeEventBuilder::with_clock(session_id.to_owned(), options.clock, true);
    let start_payload = if options.stub_model_fixture_profile {
        serde_json::json!({"reason":"fixture-start"})
    } else {
        serde_json::json!({})
    };
    builder.emit(None, EventType::SessionStarted, start_payload)?;

    let context = FlowEmitContext {
        registry,
        policy,
        side_effect_mode: options.side_effect_mode,
        stub_model_fixture_profile: options.stub_model_fixture_profile,
    };
    let failed = match emit_flow_block(
        &context,
        root_flow,
        None,
        options.root_input.clone(),
        &mut builder,
        &[],
    ) {
        Ok(ExecutionOutcome::Failed(failure)) => Some(failure),
        Ok(ExecutionOutcome::Completed(_)) => None,
        Err(err) if execution::should_terminalize_error(options.side_effect_mode, &err) => {
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
