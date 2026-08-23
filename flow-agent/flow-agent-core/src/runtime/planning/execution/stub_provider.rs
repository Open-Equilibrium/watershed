use super::FlowEmitContext;
use crate::runtime::{
    context::{ContextModelProfile, compile_provider_turn_context},
    event_construction::RuntimeEventBuilder,
    stream_signature::FlowInvocation,
    types::RuntimeError,
};
use proto::EventType;

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_stub_provider_turn(
    context: &FlowEmitContext<'_>,
    flow_block: &core_script::FlowBlock,
    phase: &core_script::PhaseBlock,
    phase_execution_id: &str,
    invocation: &FlowInvocation,
    builder: &mut RuntimeEventBuilder,
    input: Option<&core_script::FlowValue>,
    content_override: Option<&str>,
) -> Result<(), RuntimeError> {
    let compiled = compile_provider_turn_context(
        &ContextModelProfile::stub_v0(),
        context.registry,
        flow_block,
        phase,
        phase_execution_id,
        input,
        invocation,
        &builder.session_id,
        &builder.history,
    )?;
    let content = match content_override {
        Some(content) => content,
        None => stub_message_content(context.registry, phase, &compiled.provider_bytes)?,
    };
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
    )
}

pub(super) fn stub_provider_requests_tools(
    registry: &core_script::ResolvedRegistry,
    phase: &core_script::PhaseBlock,
) -> bool {
    if phase.tool_refs.is_empty() {
        return false;
    }
    !phase.instruction_refs.iter().any(|instruction_ref| {
        registry
            .instruction_block(instruction_ref)
            .is_some_and(|instruction| instruction.prompt.contains("fixture-tool-request: none"))
    })
}

pub(super) fn stub_phase_result(
    registry: &core_script::ResolvedRegistry,
    phase: &core_script::PhaseBlock,
    iteration: u8,
    input: Option<&core_script::FlowValue>,
) -> Result<core_script::FlowValue, RuntimeError> {
    for instruction_ref in &phase.instruction_refs {
        let instruction = registry
            .instruction_block(instruction_ref)
            .expect("validated Instruction reference remains in the registry");
        if let Some(json) = instruction.prompt.split("fixture-results:").nth(1) {
            let values: Vec<core_script::FlowValue> =
                serde_json::from_str(json.trim()).map_err(|error| {
                    RuntimeError::Protocol(format!(
                        "stub fixture-results for {} are invalid: {error}",
                        instruction.identity.id
                    ))
                })?;
            let index =
                usize::from(iteration.saturating_sub(1)).min(values.len().saturating_sub(1));
            return values.get(index).cloned().ok_or_else(|| {
                RuntimeError::Protocol(format!(
                    "stub fixture-results for {} must not be empty",
                    instruction.identity.id
                ))
            });
        }
        if let Some(json) = instruction.prompt.split("fixture-result:").nth(1) {
            return serde_json::from_str(json.trim()).map_err(|error| {
                RuntimeError::Protocol(format!(
                    "stub fixture-result for {} is invalid: {error}",
                    instruction.identity.id
                ))
            });
        }
    }
    if let Some(input) = input
        && core_script::validate_flow_value_against_contract(input, &phase.output).is_ok()
    {
        return Ok(input.clone());
    }
    stub_value_for_contract(
        &phase.output,
        stub_message_content(registry, phase, b"stub")?,
    )
}

fn stub_value_for_contract(
    contract: &core_script::ValueContract,
    content: &str,
) -> Result<core_script::FlowValue, RuntimeError> {
    Ok(match contract {
        core_script::ValueContract::Boolean => core_script::FlowValue::Boolean(true),
        core_script::ValueContract::Integer { min, max } => {
            let value = min.unwrap_or(0).min(max.unwrap_or(i64::MAX));
            core_script::FlowValue::Integer(value.to_string())
        }
        core_script::ValueContract::String { max_length } => {
            let value = match max_length {
                Some(limit) => content.chars().take(usize::from(*limit)).collect(),
                None => content.to_owned(),
            };
            core_script::FlowValue::String(value)
        }
        core_script::ValueContract::List { .. } => core_script::FlowValue::List(Vec::new()),
        core_script::ValueContract::Map { fields } => core_script::FlowValue::Map(
            fields
                .iter()
                .filter(|field| field.required)
                .map(|field| {
                    stub_value_for_contract(&field.value_contract, content)
                        .map(|value| (field.name.clone(), value))
                })
                .collect::<Result<_, RuntimeError>>()?,
        ),
        core_script::ValueContract::SessionObject => core_script::FlowValue::SessionObject(
            core_script::build_session_object_uri(&"0".repeat(64)).map_err(|error| {
                RuntimeError::Protocol(format!("stub session object URI is invalid: {error}"))
            })?,
        ),
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
        let instruction = registry
            .instruction_block(instruction_ref)
            .expect("validated Instruction reference remains in the registry");
        if instruction.prompt.to_ascii_lowercase().contains("smoke") {
            return Ok("smoke");
        }
    }

    Ok("hello")
}
