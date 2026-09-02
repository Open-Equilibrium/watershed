use super::super::attempt_codec::canonical_request_hash;
use crate::runtime::{
    executor::{ExecutorDispatchOutcome, ExecutorToolExecution},
    fs_guards::{AnchoredDir, AnchoredWorkspace},
    productive::ProductiveToolExecutor,
    tool_runner::{ToolExecutionOutcome, ToolInvocation},
    types::RuntimeError,
};
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

pub(crate) struct SystemProductiveToolExecutor;

pub(crate) struct SystemPreparedTool {
    enforcement: proto::EnforcementReceiptV0,
    outcome: ToolExecutionOutcome,
    request_hash: String,
}

impl ProductiveToolExecutor for SystemProductiveToolExecutor {
    type Prepared = SystemPreparedTool;

    fn supports_productive_tools(&self) -> bool {
        cfg!(unix)
    }

    fn prepare(
        &mut self,
        invocation: &ToolInvocation,
        workspace: &AnchoredWorkspace,
        policy: &core_policy::PolicyArtifact,
        command_policy: &core_policy::CommandPolicy,
        request_id: &str,
    ) -> Result<Self::Prepared, RuntimeError> {
        let request_hash = canonical_request_hash(&serde_json::json!({
            "command": command_policy,
            "invocation": {
                "argv": &invocation.argv,
                "executable": &invocation.executable,
            },
            "policy": policy,
            "request_id": request_id,
        }))?;
        let policy_digest =
            canonical_request_hash(&serde_json::to_value(policy).map_err(RuntimeError::Json)?)?;
        let outcome = self.execute(
            invocation,
            workspace.root(),
            Duration::from_millis(policy.runtime_limits.timeout_ms),
        )?;
        Ok(SystemPreparedTool {
            enforcement: test_enforcement_receipt(
                policy_digest,
                command_policy.max_concurrent_processes_and_threads,
                command_policy.runtime_profile,
            ),
            outcome,
            request_hash,
        })
    }

    fn request_hash<'a>(&self, prepared: &'a Self::Prepared) -> &'a str {
        &prepared.request_hash
    }

    fn policy_digest<'a>(&self, prepared: &'a Self::Prepared) -> &'a str {
        &prepared.enforcement.applied_policy_digest
    }

    fn max_concurrent_processes_and_threads(&self, prepared: &Self::Prepared) -> u32 {
        prepared.enforcement.max_concurrent_processes_and_threads
    }

    fn runtime_profile(&self, prepared: &Self::Prepared) -> proto::RuntimeReadProfileV0 {
        prepared.enforcement.runtime_profile
    }

    fn execute_prepared(
        &mut self,
        prepared: Self::Prepared,
    ) -> Result<ExecutorDispatchOutcome, RuntimeError> {
        Ok(ExecutorDispatchOutcome::Completed(Box::new(
            ExecutorToolExecution {
                enforcement: prepared.enforcement,
                outcome: prepared.outcome,
                request_hash: prepared.request_hash,
            },
        )))
    }
}

impl SystemProductiveToolExecutor {
    pub(crate) fn execute(
        &mut self,
        invocation: &ToolInvocation,
        workspace: &AnchoredDir,
        timeout: Duration,
    ) -> Result<ToolExecutionOutcome, RuntimeError> {
        #[cfg(unix)]
        {
            let deadline = Instant::now()
                .checked_add(timeout)
                .ok_or_else(|| RuntimeError::Protocol("Tool deadline overflowed".to_owned()))?;
            Ok(crate::runtime::tool_runner::execute_tool_invocation(
                invocation,
                workspace,
                crate::runtime::tool_runner::ToolRunControl {
                    cancelled: crate::runtime::cancellation::productive_cancellation(),
                    deadline,
                },
            ))
        }
        #[cfg(not(unix))]
        {
            let _ = (invocation, workspace, timeout);
            Err(RuntimeError::Usage(
                "productive Tools are unavailable on this platform".to_owned(),
            ))
        }
    }
}

pub(crate) fn test_enforcement_receipt(
    applied_policy_digest: String,
    max_concurrent_processes_and_threads: u32,
    profile: core_script::ToolRuntimeProfile,
) -> proto::EnforcementReceiptV0 {
    proto::EnforcementReceiptV0 {
        applied_policy_digest,
        backend: proto::EXECUTOR_BACKEND_V0.to_owned(),
        backend_version: "test".to_owned(),
        executor: proto::EXECUTOR_NAME_V0.to_owned(),
        executor_version: "test".to_owned(),
        isolation_active: true,
        max_concurrent_processes_and_threads,
        platform: proto::EXECUTOR_PLATFORM_V0.to_owned(),
        runtime_profile: match profile {
            core_script::ToolRuntimeProfile::Exact => proto::RuntimeReadProfileV0::Exact,
            core_script::ToolRuntimeProfile::HostSystemRead => {
                proto::RuntimeReadProfileV0::HostSystemRead
            }
        },
    }
}
