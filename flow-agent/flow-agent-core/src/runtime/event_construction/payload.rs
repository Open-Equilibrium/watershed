pub(crate) fn flow_started_payload(flow: &core_script::FlowBlock) -> serde_json::Value {
    serde_json::json!({
        "flow_definition_id": flow.identity.id,
        "flow_name": flow.identity.name,
    })
}

pub(crate) fn flow_completed_payload(
    flow: &core_script::FlowBlock,
    result: &Option<core_script::FlowValue>,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "flow_definition_id": flow.identity.id,
        "flow_name": flow.identity.name,
    });
    if let Some(result) = result {
        payload
            .as_object_mut()
            .expect("Flow payload is an object")
            .insert("result".to_owned(), serde_json::json!(result));
    }
    payload
}

pub(crate) fn phase_entered_payload(
    registry: &core_script::ResolvedRegistry,
    phase: &core_script::PhaseBlock,
    phase_execution_id: &str,
    iteration: u8,
) -> serde_json::Value {
    let instruction_ids = phase
        .instruction_refs
        .iter()
        .map(|instruction_ref| {
            registry
                .instruction_block(instruction_ref)
                .expect("validated Instruction reference remains in the registry")
                .identity
                .id
                .clone()
        })
        .collect::<Vec<_>>();
    let tool_ids = phase
        .tool_refs
        .iter()
        .map(|tool_ref| {
            registry
                .tool_block(tool_ref)
                .expect("validated Tool reference remains in the registry")
                .identity
                .id
                .clone()
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "instruction_ids": instruction_ids,
        "iteration": iteration,
        "phase_execution_id": phase_execution_id,
        "phase_id": phase.identity.id,
        "phase_kind": phase_kind(phase),
        "phase_name": phase.identity.name,
        "tool_ids": tool_ids,
    })
}

pub(crate) fn phase_completed_payload(
    phase: &core_script::PhaseBlock,
    phase_execution_id: &str,
    iteration: u8,
    result: &core_script::FlowValue,
    will_repeat: bool,
) -> serde_json::Value {
    serde_json::json!({
        "iteration": iteration,
        "phase_execution_id": phase_execution_id,
        "phase_id": phase.identity.id,
        "phase_kind": phase_kind(phase),
        "result": result,
        "will_repeat": will_repeat,
    })
}

pub(crate) fn phase_kind(phase: &core_script::PhaseBlock) -> proto::PhaseKind {
    if phase.phase_refs.is_empty() {
        proto::PhaseKind::Leaf
    } else {
        proto::PhaseKind::Composite
    }
}

pub(crate) fn tool_started_payload(
    tool: &core_script::ToolBlock,
    command_policy: &core_policy::CommandPolicy,
    attempt_id: Option<&str>,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "allowed_parameters": command_policy
            .allowed_parameters
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<Vec<_>>(),
        "max_concurrent_processes_and_threads": command_policy.max_concurrent_processes_and_threads,
        "network_access": tool_network_access(&tool.network),
        "read_only_mounts": command_policy.filesystem.read_only_mounts,
        "runtime_profile": command_policy.runtime_profile.as_str(),
        "tool_id": tool.identity.id,
        "tool_kind": tool_kind(&tool.tool_kind),
        "tool_name": tool.identity.name,
        "writable_mounts": command_policy.filesystem.writable_mounts,
    });
    if let Some(attempt_id) = attempt_id {
        payload
            .as_object_mut()
            .expect("Tool payload is an object")
            .insert(
                "attempt_id".to_owned(),
                serde_json::Value::String(attempt_id.to_owned()),
            );
    }
    payload
}

fn tool_kind(tool_kind: &core_script::ToolKind) -> proto::ToolKind {
    match tool_kind {
        core_script::ToolKind::PredefinedCommand => proto::ToolKind::PredefinedCommand,
        core_script::ToolKind::OwnScript => proto::ToolKind::OwnScript,
    }
}

fn tool_network_access(network: &core_script::NetworkPolicy) -> proto::ToolNetworkAccess {
    match network {
        core_script::NetworkPolicy::Deny(_) => proto::ToolNetworkAccess::Deny,
        core_script::NetworkPolicy::Declared { .. } => proto::ToolNetworkAccess::Declared,
    }
}
