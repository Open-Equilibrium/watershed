//! Feature-gated observational evidence for the M1.2 Executor startup boundary.

use std::{
    path::Path,
    time::{Duration, Instant},
};

use crate::runtime::{
    digest::{is_lowercase_sha256_hex, strip_sha256_prefix},
    executor::{ExecutorDispatchOutcome, PreparedExecutor},
    fs_guards::AnchoredWorkspace,
    productive::ensure_productive_tool_execution_platform,
    run_attempts::RunAttemptOutcome,
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
    }
}

/// One unadjusted observation through the selected and prepared Executor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct M12ExecutorStartupMeasurement {
    /// Preparation and readiness through validated Tool result and enforcement receipt.
    pub executor_elapsed: Duration,
    /// Mandatory own-file protection confirmed by the validated Executor receipt.
    pub self_protection_active: bool,
}

fn is_prefixed_lower_sha256(value: &str) -> bool {
    strip_sha256_prefix(value).is_some_and(is_lowercase_sha256_hex)
}

/// Rejects public M1.2 Executor checks outside supported native Tool hosts.
pub fn ensure_m12_executor_host() -> Result<(), String> {
    ensure_productive_tool_execution_platform()
        .map_err(|_| "M1.2 Executor startup requires a supported native Tool host".to_owned())
}

/// Measures one fixed no-op Tool through the real prepared Executor boundary.
pub fn run_m12_executor_startup(workspace: &Path) -> Result<M12ExecutorStartupMeasurement, String> {
    ensure_m12_executor_host()?;
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
    let executor = PreparedExecutor::prepare_selected()
        .map_err(|_| "selected Executor did not prepare for M1.2 evidence")?;
    let prepared = executor
        .prepare_tool(
            &workspace,
            &policy,
            command_policy,
            &invocation,
            "m12-startup-evidence",
        )
        .map_err(|_| "selected Executor did not complete M1.2 evidence")?;
    let expected_request_hash = prepared.request_hash().to_owned();
    let expected_policy_digest = prepared.policy_digest().to_owned();
    let dispatch = executor
        .execute_prepared(prepared)
        .map_err(|_| "selected Executor did not complete M1.2 evidence")?;
    let ExecutorDispatchOutcome::Completed(execution) = dispatch else {
        return Err("selected Executor rejected the M1.2 evidence Tool before launch".to_owned());
    };

    if execution.outcome.status != RunAttemptOutcome::Completed
        || execution.outcome.classification.is_some()
        || execution.outcome.exit_code != Some(0)
        || execution.outcome.stdout != b"\n"
        || !execution.outcome.stderr.is_empty()
    {
        return Err("Executor did not return the exact no-op Tool result".to_owned());
    }
    if proto::validate_enforcement_receipt_v0(&execution.enforcement, &expected_policy_digest)
        .is_err()
        || execution.request_hash != expected_request_hash
        || !is_prefixed_lower_sha256(&execution.request_hash)
    {
        return Err("Executor did not return the exact enforcement evidence".to_owned());
    }
    let executor_elapsed = started.elapsed();

    Ok(M12ExecutorStartupMeasurement {
        executor_elapsed,
        self_protection_active: execution.enforcement.self_protection_active,
    })
}

#[cfg(test)]
mod tests {
    use super::{fixed_executor_policy, is_prefixed_lower_sha256};

    #[test]
    fn fixed_evidence_policy_is_an_exact_empty_echo() {
        let policy = fixed_executor_policy();

        policy.validate().unwrap();
        let command = &policy.commands[0];
        assert_eq!(command.command_id, "agent-echo");
        assert!(command.argv.is_empty());
        assert!(command.allowed_parameters.is_empty());
        assert!(command.environment.allow.is_empty());
        assert_eq!(
            command.environment.default,
            core_policy::EnvironmentDefault::Clear
        );
        assert_eq!(command.tool_kind, core_policy::ToolKind::PredefinedCommand);
        assert_eq!(policy.phase_scope[0].tool_ids, ["m12-startup-noop"]);
        assert_eq!(policy.runtime_limits.timeout_ms, 5_000);
    }

    #[test]
    fn startup_evidence_accepts_only_the_run_log_request_hash_format() {
        assert!(is_prefixed_lower_sha256(&format!(
            "sha256:{}",
            "a".repeat(64)
        )));
        assert!(!is_prefixed_lower_sha256(&"a".repeat(64)));
        assert!(!is_prefixed_lower_sha256(&format!(
            "sha256:{}",
            "A".repeat(64)
        )));
    }
}
