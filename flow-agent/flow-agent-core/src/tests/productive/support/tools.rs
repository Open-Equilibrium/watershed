use crate::runtime::{
    executor::ExecutorToolExecution,
    fs_guards::AnchoredWorkspace,
    productive::{ProductiveToolExecutor, test_enforcement_receipt},
    run_attempts::RunAttemptOutcome,
    tool_runner::{ToolExecutionOutcome, ToolInvocation},
    types::RuntimeError,
};

pub(in super::super) struct FakePreparedTool {
    invocation: ToolInvocation,
    policy_digest: String,
    request_hash: String,
    runtime_profile: core_script::ToolRuntimeProfile,
}
pub(in super::super) struct FakeToolExecutor {
    pub(in super::super) cancel_before_outcome: bool,
    pub(in super::super) error_after_interrupt: bool,
    pub(in super::super) fault: FakeToolExecutionFault,
    pub(in super::super) invocations: Vec<ToolInvocation>,
    pub(in super::super) outcome: ToolExecutionOutcome,
}

#[derive(Clone, Copy, Debug, Default)]
pub(in super::super) enum FakeToolExecutionFault {
    #[default]
    None,
    ExecutorError,
    RequestHashMismatch,
    ReceiptMismatch,
}

impl Default for FakeToolExecutor {
    fn default() -> Self {
        Self {
            cancel_before_outcome: false,
            error_after_interrupt: false,
            fault: FakeToolExecutionFault::None,
            invocations: Vec::new(),
            outcome: ToolExecutionOutcome {
                status: RunAttemptOutcome::Completed,
                classification: None,
                exit_code: Some(0),
                stdout: b"tool-output\n".to_vec(),
                stderr: Vec::new(),
            },
        }
    }
}

impl ProductiveToolExecutor for FakeToolExecutor {
    type Prepared = FakePreparedTool;

    fn supports_productive_tools(&self) -> bool {
        true
    }

    fn prepare(
        &mut self,
        invocation: &ToolInvocation,
        _workspace: &AnchoredWorkspace,
        policy: &core_policy::PolicyArtifact,
        command_policy: &core_policy::CommandPolicy,
        request_id: &str,
    ) -> Result<Self::Prepared, RuntimeError> {
        let _ = (policy, command_policy, request_id);
        let policy_digest = "0".repeat(64);
        let request_hash = "1".repeat(64);
        Ok(FakePreparedTool {
            invocation: invocation.clone(),
            policy_digest,
            request_hash,
            runtime_profile: command_policy.runtime_profile,
        })
    }

    fn request_hash<'a>(&self, prepared: &'a Self::Prepared) -> &'a str {
        &prepared.request_hash
    }

    fn policy_digest<'a>(&self, prepared: &'a Self::Prepared) -> &'a str {
        &prepared.policy_digest
    }

    fn runtime_profile(&self, prepared: &Self::Prepared) -> proto::RuntimeReadProfileV0 {
        match prepared.runtime_profile {
            core_script::ToolRuntimeProfile::Exact => proto::RuntimeReadProfileV0::Exact,
            core_script::ToolRuntimeProfile::HostSystemRead => {
                proto::RuntimeReadProfileV0::HostSystemRead
            }
        }
    }

    fn execute_prepared(
        &mut self,
        prepared: Self::Prepared,
    ) -> Result<ExecutorToolExecution, RuntimeError> {
        self.invocations.push(prepared.invocation);
        if self.cancel_before_outcome {
            assert_eq!(
                crate::request_productive_interrupt(),
                crate::ProductiveInterruptAction::Cancel
            );
        }
        if self.error_after_interrupt {
            assert_eq!(
                crate::request_productive_interrupt(),
                crate::ProductiveInterruptAction::Cancel
            );
            return Err(RuntimeError::Protocol(
                "Tool failed after cancellation won".to_owned(),
            ));
        }
        if matches!(self.fault, FakeToolExecutionFault::ExecutorError) {
            return Err(RuntimeError::Protocol(
                "fixture Executor failure".to_owned(),
            ));
        }
        let policy_digest = if matches!(self.fault, FakeToolExecutionFault::ReceiptMismatch) {
            "2".repeat(64)
        } else {
            prepared.policy_digest
        };
        let request_hash = if matches!(self.fault, FakeToolExecutionFault::RequestHashMismatch) {
            "2".repeat(64)
        } else {
            prepared.request_hash
        };
        Ok(ExecutorToolExecution {
            enforcement: test_enforcement_receipt(policy_digest, prepared.runtime_profile),
            outcome: self.outcome.clone(),
            request_hash,
        })
    }
}

pub(in super::super) struct UnsupportedToolExecutor;

impl ProductiveToolExecutor for UnsupportedToolExecutor {
    type Prepared = ();

    fn supports_productive_tools(&self) -> bool {
        false
    }

    fn prepare(
        &mut self,
        _invocation: &ToolInvocation,
        _workspace: &AnchoredWorkspace,
        _policy: &core_policy::PolicyArtifact,
        _command_policy: &core_policy::CommandPolicy,
        _request_id: &str,
    ) -> Result<Self::Prepared, RuntimeError> {
        panic!("unsupported productive Tools must fail before dispatch")
    }

    fn request_hash<'a>(&self, _prepared: &'a Self::Prepared) -> &'a str {
        unreachable!()
    }

    fn policy_digest<'a>(&self, _prepared: &'a Self::Prepared) -> &'a str {
        unreachable!()
    }

    fn runtime_profile(&self, _prepared: &Self::Prepared) -> proto::RuntimeReadProfileV0 {
        unreachable!()
    }

    fn execute_prepared(
        &mut self,
        _prepared: Self::Prepared,
    ) -> Result<ExecutorToolExecution, RuntimeError> {
        unreachable!()
    }
}
