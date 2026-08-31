use crate::{
    OWN_SCRIPT_RUNNER_POSIX_SH, POLICY_VERSION_V0, TrustedPredefinedCommand,
    artifact::{
        AllowedParameterPolicy, CommandPolicy, EnvironmentDefault, EnvironmentPolicy,
        FilesystemPolicy, NetworkPolicy, PhaseScope, PolicyArtifact, PolicyArtifactValidationError,
        PolicyTarget, RuntimeLimits, policy_artifact_error,
    },
};
use core_script::NetworkDefault;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

/// Error returned while compiling policy artifacts from a script registry.
#[derive(Debug)]
pub enum PolicyCompileError {
    /// Requested flow reference was missing.
    MissingFlow(String),
    /// Supported policy-artifact target was asked to encode network allow entries.
    NonEmptyNetworkAllowlist {
        /// Tool id with non-empty network allow entries.
        tool_id: String,
    },
    /// Compiled artifact failed validation.
    InvalidArtifact(PolicyArtifactValidationError),
}

impl fmt::Display for PolicyCompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingFlow(reference) => {
                write!(f, "policy compile references missing flow {reference}")
            }
            Self::NonEmptyNetworkAllowlist { tool_id } => write!(
                f,
                "supported policy-artifact target for tool {tool_id} must use a deny-all network allowlist"
            ),
            Self::InvalidArtifact(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for PolicyCompileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidArtifact(err) => Some(err),
            Self::MissingFlow(_) | Self::NonEmptyNetworkAllowlist { .. } => None,
        }
    }
}

/// Compiles a policy artifact for one sandbox target.
pub fn compile_policy_artifact(
    registry: &core_script::ResolvedRegistry,
    flow_ref: &str,
    target: PolicyTarget,
) -> Result<PolicyArtifact, PolicyCompileError> {
    let flow_block = registry
        .flow_block(flow_ref)
        .ok_or_else(|| PolicyCompileError::MissingFlow(flow_ref.to_owned()))?;
    let mut phase_tools = BTreeMap::<String, BTreeSet<String>>::new();
    let mut tool_ids = BTreeSet::<String>::new();
    let mut visited_flows = BTreeSet::<String>::new();
    let mut visited_phases = BTreeSet::<String>::new();
    collect_flow_policy_scope(
        registry,
        flow_block,
        &mut phase_tools,
        &mut tool_ids,
        &mut visited_flows,
        &mut visited_phases,
    );

    let mut commands = Vec::new();
    for tool_id in tool_ids {
        let tool = registry
            .tool_block(&tool_id)
            .expect("resolved registry preserves collected tools");
        commands.push(command_policy_from_tool(tool, &target)?);
    }

    let artifact = PolicyArtifact {
        commands,
        phase_scope: phase_tools
            .into_iter()
            .map(|(phase_id, tool_ids)| PhaseScope {
                phase_id,
                tool_ids: tool_ids.into_iter().collect(),
            })
            .collect(),
        policy_version: POLICY_VERSION_V0.to_owned(),
        runtime_limits: RuntimeLimits {
            headless: true,
            timeout_ms: if flow_block.phase_refs.len() > 1 || !flow_block.subflow_refs.is_empty() {
                60_000
            } else {
                30_000
            },
        },
        source_flow_definition_id: flow_block.identity.id.clone(),
        target,
    };
    artifact
        .validate()
        .map_err(PolicyCompileError::InvalidArtifact)?;
    Ok(artifact)
}

fn collect_flow_policy_scope(
    registry: &core_script::ResolvedRegistry,
    flow_block: &core_script::FlowBlock,
    phase_tools: &mut BTreeMap<String, BTreeSet<String>>,
    tool_ids: &mut BTreeSet<String>,
    visited_flows: &mut BTreeSet<String>,
    visited_phases: &mut BTreeSet<String>,
) {
    if !visited_flows.insert(flow_block.identity.id.clone()) {
        return;
    }

    for phase_ref in &flow_block.phase_refs {
        let phase = registry
            .phase_block(phase_ref)
            .expect("resolved registry validates flow phase references");
        collect_phase_policy_scope(registry, phase, phase_tools, tool_ids, visited_phases);
    }

    for subflow_ref in &flow_block.subflow_refs {
        let subflow = registry
            .flow_block(subflow_ref)
            .expect("resolved registry validates subflow references");
        collect_flow_policy_scope(
            registry,
            subflow,
            phase_tools,
            tool_ids,
            visited_flows,
            visited_phases,
        );
    }
}

fn collect_phase_policy_scope(
    registry: &core_script::ResolvedRegistry,
    phase: &core_script::PhaseBlock,
    phase_tools: &mut BTreeMap<String, BTreeSet<String>>,
    tool_ids: &mut BTreeSet<String>,
    visited_phases: &mut BTreeSet<String>,
) {
    if !visited_phases.insert(phase.identity.id.clone()) {
        return;
    }

    let scoped_tools = phase_tools.entry(phase.identity.id.clone()).or_default();
    for tool_ref in &phase.tool_refs {
        let tool = registry
            .tool_block(tool_ref)
            .expect("resolved registry validates phase tool references");
        scoped_tools.insert(tool.identity.id.clone());
        tool_ids.insert(tool.identity.id.clone());
    }

    for child_ref in &phase.phase_refs {
        let child = registry
            .phase_block(child_ref)
            .expect("resolved registry validates child phase references");
        collect_phase_policy_scope(registry, child, phase_tools, tool_ids, visited_phases);
    }
}

pub(crate) fn command_policy_from_tool(
    tool: &core_script::ToolBlock,
    target: &PolicyTarget,
) -> Result<CommandPolicy, PolicyCompileError> {
    let (command_id, argv, executable, script_runtime) = match (&tool.tool_kind, &tool.command) {
        (
            core_script::ToolKind::PredefinedCommand,
            core_script::ToolCommand::Predefined { command_id, argv },
        ) => {
            let command = TrustedPredefinedCommand::parse(command_id).ok_or_else(|| {
                PolicyCompileError::InvalidArtifact(policy_artifact_error(format!(
                    "predefined-command tool {} references unknown trusted command {command_id:?}",
                    tool.identity.id
                )))
            })?;
            let executable = command.executable();
            (command_id.clone(), argv.clone(), executable, None)
        }
        (core_script::ToolKind::OwnScript, core_script::ToolCommand::OwnScript(command_id)) => (
            command_id.clone(),
            Vec::new(),
            OWN_SCRIPT_RUNNER_POSIX_SH.to_owned(),
            Some(core_script::ScriptRuntime::PosixSh),
        ),
        _ => {
            return Err(PolicyCompileError::InvalidArtifact(policy_artifact_error(
                format!(
                    "tool {} command shape does not match tool_kind",
                    tool.identity.id
                ),
            )));
        }
    };

    let network = match &tool.network {
        core_script::NetworkPolicy::Deny(_) => NetworkPolicy {
            allow: Vec::new(),
            default: NetworkDefault::Deny,
        },
        core_script::NetworkPolicy::Declared { allow, .. } => {
            if matches!(target, PolicyTarget::LinuxLandlockSeccomp) && !allow.is_empty() {
                return Err(PolicyCompileError::NonEmptyNetworkAllowlist {
                    tool_id: tool.identity.id.clone(),
                });
            }
            NetworkPolicy {
                allow: allow.clone(),
                default: NetworkDefault::Deny,
            }
        }
    };

    Ok(CommandPolicy {
        allowed_parameters: tool
            .allowed_parameters
            .iter()
            .map(allowed_parameter_policy)
            .collect(),
        argv,
        command_id,
        environment: EnvironmentPolicy {
            allow: Vec::new(),
            default: EnvironmentDefault::Clear,
        },
        executable,
        filesystem: FilesystemPolicy {
            read_only_mounts: tool.read_only_mounts.clone(),
            writable_mounts: tool.writable_mounts.clone(),
        },
        network,
        runtime_profile: tool.runtime_profile,
        script_runtime,
        tool_id: tool.identity.id.clone(),
        tool_kind: tool.tool_kind.clone(),
    })
}

fn allowed_parameter_policy(parameter: &core_script::AllowedParameter) -> AllowedParameterPolicy {
    AllowedParameterPolicy {
        name: parameter.name.clone(),
        required: parameter.required,
        max: parameter.max,
        max_length: parameter.max_length,
        min: parameter.min,
        value_pattern: parameter.value_pattern.clone(),
        value_type: parameter.value_type.clone(),
        allowed_values: parameter.allowed_values.clone(),
    }
}
