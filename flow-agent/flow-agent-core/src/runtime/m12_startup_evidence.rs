//! Feature-gated observational evidence for the M1.2 Executor startup boundary.

use std::{
    path::Path,
    time::{Duration, Instant},
};

use crate::runtime::{
    executor::PreparedExecutor, fs_guards::AnchoredWorkspace, run_attempts::RunAttemptOutcome,
    tool_runner::ToolInvocation,
};

fn fixed_executor_policy() -> core_policy::PolicyArtifact {
    core_policy::PolicyArtifact {
        commands: vec![core_policy::CommandPolicy {
            allowed_parameters: Vec::new(),
            argv: Vec::new(),
            command_id: "agent-echo".to_owned(),
            environment: core_policy::EnvironmentPolicy {
                allow: Vec::new(),
                default: core_policy::EnvironmentDefault::Clear,
            },
            executable: "registry:agent-echo".to_owned(),
            filesystem: core_policy::FilesystemPolicy {
                read_only_mounts: vec!["workspace".to_owned()],
                writable_mounts: Vec::new(),
            },
            network: core_policy::NetworkPolicy {
                allow: Vec::new(),
                default: core_policy::NetworkDefault::Deny,
            },
            runtime_profile: core_policy::ToolRuntimeProfile::Exact,
            script_runtime: None,
            tool_id: "m12-startup-noop".to_owned(),
            tool_kind: core_policy::ToolKind::PredefinedCommand,
        }],
        phase_scope: vec![core_policy::PhaseScope {
            phase_id: "evidence".to_owned(),
            tool_ids: vec!["m12-startup-noop".to_owned()],
        }],
        policy_version: core_policy::POLICY_VERSION_V0.to_owned(),
        runtime_limits: core_policy::RuntimeLimits {
            headless: true,
            timeout_ms: 5_000,
        },
        source_flow_definition_id: "m12-executor-startup".to_owned(),
        target: core_policy::PolicyTarget::LinuxBubblewrapSeccomp,
    }
}

/// One unadjusted observation through the selected and prepared Executor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct M12ExecutorStartupMeasurement {
    /// Preparation and readiness through validated Tool result and enforcement receipt.
    pub executor_elapsed: Duration,
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Measures one fixed no-op Tool through the real prepared Executor boundary.
pub fn run_m12_executor_startup(workspace: &Path) -> Result<M12ExecutorStartupMeasurement, String> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return Err(
            "M1.2 startup evidence requires the Ubuntu 24.04 x64 reference platform".to_owned(),
        );
    }
    let policy = fixed_executor_policy();
    policy
        .validate()
        .map_err(|_| "M1.2 evidence policy was invalid")?;
    let command_policy = policy
        .commands
        .first()
        .ok_or("M1.2 evidence command was missing")?;
    let workspace =
        AnchoredWorkspace::open(workspace).map_err(|_| "M1.2 evidence workspace did not open")?;
    let invocation = ToolInvocation {
        executable: "/bin/echo".to_owned(),
        argv: Vec::new(),
    };

    let started = Instant::now();
    let mut executor = PreparedExecutor::prepare_selected()
        .map_err(|_| "selected Executor did not prepare for M1.2 evidence")?;
    let execution = executor
        .execute(
            &workspace,
            &policy,
            command_policy,
            &invocation,
            "m12-startup-evidence",
        )
        .map_err(|_| "selected Executor did not complete M1.2 evidence")?;

    if execution.outcome.status != RunAttemptOutcome::Completed
        || execution.outcome.classification.is_some()
        || execution.outcome.exit_code != Some(0)
        || execution.outcome.stdout != b"\n"
        || !execution.outcome.stderr.is_empty()
    {
        return Err("Executor did not return the exact no-op Tool result".to_owned());
    }
    if !execution.enforcement.isolation_active
        || execution.enforcement.runtime_profile != proto::RuntimeReadProfileV0::Exact
        || !is_lower_sha256(&execution.enforcement.applied_policy_digest)
        || !is_lower_sha256(&execution.request_hash)
    {
        return Err("Executor did not return the exact enforcement evidence".to_owned());
    }
    let executor_elapsed = started.elapsed();

    Ok(M12ExecutorStartupMeasurement { executor_elapsed })
}

#[cfg(test)]
mod tests {
    use super::fixed_executor_policy;

    #[test]
    fn fixed_evidence_policy_is_an_exact_empty_echo() {
        let policy = fixed_executor_policy();

        policy.validate().unwrap();
        let command = &policy.commands[0];
        assert_eq!(command.command_id, "agent-echo");
        assert!(command.argv.is_empty());
        assert!(command.environment.allow.is_empty());
        assert_eq!(
            command.runtime_profile,
            core_policy::ToolRuntimeProfile::Exact
        );
        assert_eq!(command.filesystem.read_only_mounts, ["workspace"]);
    }
}
