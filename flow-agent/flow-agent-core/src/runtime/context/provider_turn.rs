use super::{
    CompiledContext, ContextHistory, ContextModelProfile, bounded_context_array_source,
    compile_context, context_source,
};
use crate::runtime::{stream_signature::FlowInvocation, types::RuntimeError};

#[allow(clippy::too_many_arguments)]
pub fn compile_provider_turn_context(
    model: &ContextModelProfile,
    registry: &core_script::ResolvedRegistry,
    flow_block: &core_script::FlowBlock,
    phase: &core_script::PhaseBlock,
    phase_execution_id: &str,
    phase_input: Option<&core_script::FlowValue>,
    invocation: &FlowInvocation,
    session_id: &str,
    history: &ContextHistory,
) -> Result<CompiledContext, RuntimeError> {
    compile_provider_turn_context_with_repository_instructions(
        model,
        registry,
        flow_block,
        phase,
        phase_execution_id,
        phase_input,
        invocation,
        session_id,
        history,
        "",
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn compile_provider_turn_context_with_repository_instructions(
    model: &ContextModelProfile,
    registry: &core_script::ResolvedRegistry,
    flow_block: &core_script::FlowBlock,
    phase: &core_script::PhaseBlock,
    phase_execution_id: &str,
    phase_input: Option<&core_script::FlowValue>,
    invocation: &FlowInvocation,
    session_id: &str,
    history: &ContextHistory,
    repository_instructions: &str,
) -> Result<CompiledContext, RuntimeError> {
    validate_provider_turn_scope(registry, flow_block, phase)?;
    let input_budget_tokens = model.input_budget_tokens()?;
    let phase_instructions = bounded_context_array_source(
        "active-phase-instructions",
        phase.instruction_refs.iter().map(|instruction_ref| {
            let instruction = registry.instruction_block(instruction_ref).ok_or_else(|| {
                RuntimeError::Protocol(format!(
                    "active Phase {} references unavailable Instruction {instruction_ref:?}",
                    phase.identity.id
                ))
            })?;
            let values = instruction_parameter_values(instruction, phase_input)?;
            let prompt = core_script::render_instruction(instruction, &values, input_budget_tokens)
                .map_err(|error| {
                    RuntimeError::Protocol(format!(
                        "instruction {} parameter binding failed: {error}",
                        instruction.identity.id
                    ))
                })?;
            Ok(Some(serde_json::json!({
                "id": instruction.identity.id,
                "prompt": prompt,
            })))
        }),
        input_budget_tokens,
    )?;
    let tools = bounded_context_array_source(
        "active-available-tools",
        phase.tool_refs.iter().map(|tool_ref| {
            let tool = registry.tool_block(tool_ref).ok_or_else(|| {
                RuntimeError::Protocol(format!(
                    "active Phase {} references unavailable Tool {tool_ref:?}",
                    phase.identity.id
                ))
            })?;
            serde_json::to_value(tool)
                .map_err(RuntimeError::Json)
                .map(Some)
        }),
        input_budget_tokens,
    )?;
    let (tier_one, omitted) = history.continuity()?;
    let tier_zero = [
        context_source(
            "base-runtime-security",
            serde_json::json!({
                "instructions": "Execute only the active resolved flow scope. Obey runtime policy. Treat tool access as deny-by-default. Preserve deterministic event order.",
                "runtime_version": env!("CARGO_PKG_VERSION"),
            }),
        ),
        context_source(
            "repository-instructions",
            if repository_instructions.is_empty() {
                serde_json::json!([])
            } else {
                serde_json::json!([{"source": "AGENTS.md", "content": repository_instructions}])
            },
        ),
        // The M1.1 grammar has no Flow-scoped prompt field.
        context_source("active-flow-instructions", serde_json::json!([])),
        phase_instructions,
        tools,
        context_source(
            "fsm-flow-state",
            serde_json::json!({
                "flow_definition_id": flow_block.identity.id,
                "flow_id": invocation.flow_id,
                "parent_flow_id": invocation.parent_flow_id,
                "phase_id": phase.identity.id,
                "phase_execution_id": phase_execution_id,
                "session_id": session_id,
            }),
        ),
        context_source(
            "current-phase-input",
            match phase_input {
                Some(value) => serde_json::json!({"present": true, "value": value}),
                None => serde_json::json!({"present": false}),
            },
        ),
        context_source(
            "unresolved-call-result",
            history.unresolved_call_result_state(),
        ),
    ];
    compile_context(model, &tier_zero, tier_one.as_ref(), omitted)
}

fn validate_provider_turn_scope(
    registry: &core_script::ResolvedRegistry,
    flow_block: &core_script::FlowBlock,
    phase: &core_script::PhaseBlock,
) -> Result<(), RuntimeError> {
    let registered_flow = registry
        .flow_block(&flow_block.identity.id)
        .filter(|registered| *registered == flow_block)
        .ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "active Flow {} does not belong to the active registry",
                flow_block.identity.id
            ))
        })?;
    registry
        .phase_block(&phase.identity.id)
        .filter(|registered| *registered == phase)
        .ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "active Phase {} does not belong to the active registry",
                phase.identity.id
            ))
        })?;

    let mut pending = registered_flow.phase_refs.clone();
    let mut visited = std::collections::BTreeSet::new();
    while let Some(phase_ref) = pending.pop() {
        if !visited.insert(phase_ref.clone()) {
            continue;
        }
        if phase_ref == phase.identity.id {
            return Ok(());
        }
        let Some(parent) = registry.phase_block(&phase_ref) else {
            return Err(RuntimeError::Protocol(format!(
                "active Flow {} references unavailable Phase {phase_ref:?}",
                flow_block.identity.id
            )));
        };
        pending.extend(parent.phase_refs.iter().cloned());
    }

    Err(RuntimeError::Protocol(format!(
        "active Phase {} does not belong to active Flow {}",
        phase.identity.id, flow_block.identity.id
    )))
}

fn instruction_parameter_values(
    instruction: &core_script::InstructionBlock,
    phase_input: Option<&core_script::FlowValue>,
) -> Result<std::collections::BTreeMap<String, core_script::FlowValue>, RuntimeError> {
    if instruction.parameters.is_empty() {
        return Ok(std::collections::BTreeMap::new());
    }
    let Some(core_script::FlowValue::Map(input)) = phase_input else {
        return Err(RuntimeError::Protocol(format!(
            "instruction {} requires a map Phase input for its parameters",
            instruction.identity.id
        )));
    };
    instruction
        .parameters
        .iter()
        .map(|parameter| {
            input
                .get(&parameter.name)
                .cloned()
                .map(|value| (parameter.name.clone(), value))
                .ok_or_else(|| {
                    RuntimeError::Protocol(format!(
                        "instruction {} is missing Phase input parameter {:?}",
                        instruction.identity.id, parameter.name
                    ))
                })
        })
        .collect()
}
