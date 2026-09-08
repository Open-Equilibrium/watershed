use crate::runtime::{
    executor::{ExecutorDispatchOutcome, ExecutorToolExecution},
    fs_guards::AnchoredWorkspace,
    productive::{ProductiveToolExecutor, ProductiveToolPreflight, test_enforcement_receipt},
    run_attempts::RunAttemptOutcome,
    tool_runner::{ToolExecutionOutcome, ToolInvocation},
    types::RuntimeError,
};

pub(in super::super) struct FakePreparedTool {
    invocation: ToolInvocation,
    policy_digest: String,
    request_hash: String,
}
pub(in super::super) struct FakeToolExecutor {
    pub(in super::super) cancel_before_outcome: bool,
    pub(in super::super) error_after_interrupt: bool,
    pub(in super::super) fault: FakeToolExecutionFault,
    pub(in super::super) invocations: Vec<ToolInvocation>,
    pub(in super::super) outcome: ToolExecutionOutcome,
    pub(in super::super) preflights: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub(in super::super) enum FakeToolExecutionFault {
    #[default]
    None,
    PrepareError,
    PolicyRejected,
    StartedExecutorError,
    ExecutorError,
    InvalidTerminal,
    RequestHashMismatch,
    ReceiptMismatch,
    InactiveSelfProtection,
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
            preflights: 0,
        }
    }
}

impl ProductiveToolExecutor for FakeToolExecutor {
    type Prepared = FakePreparedTool;
    type Waiting = FakePreparedTool;

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
        if matches!(self.fault, FakeToolExecutionFault::PrepareError) {
            return Err(RuntimeError::Protocol(
                "fixture Executor preparation failure".to_owned(),
            ));
        }
        let policy_digest = "0".repeat(64);
        let request_hash = super::fake_tool_request_hash();
        Ok(FakePreparedTool {
            invocation: invocation.clone(),
            policy_digest,
            request_hash,
        })
    }

    fn request_hash<'a>(&self, prepared: &'a Self::Prepared) -> &'a str {
        &prepared.request_hash
    }

    fn policy_digest<'a>(&self, prepared: &'a Self::Prepared) -> &'a str {
        &prepared.policy_digest
    }

    fn preflight(
        &mut self,
        prepared: Self::Prepared,
    ) -> Result<ProductiveToolPreflight<Self::Waiting>, RuntimeError> {
        self.preflights += 1;
        if matches!(self.fault, FakeToolExecutionFault::PolicyRejected) {
            return Ok(ProductiveToolPreflight::Rejected(
                proto::ExecutorErrorCodeV0::PolicyUnsupported,
            ));
        }
        Ok(ProductiveToolPreflight::Ready(prepared))
    }

    fn start(&mut self, prepared: Self::Waiting) -> Result<ExecutorDispatchOutcome, RuntimeError> {
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
        if matches!(self.fault, FakeToolExecutionFault::StartedExecutorError) {
            return Ok(ExecutorDispatchOutcome::Error(
                proto::ExecutorErrorCodeV0::SandboxSetupFailed,
            ));
        }
        let policy_digest = if matches!(self.fault, FakeToolExecutionFault::ReceiptMismatch) {
            "2".repeat(64)
        } else {
            prepared.policy_digest
        };
        let request_hash = if matches!(self.fault, FakeToolExecutionFault::RequestHashMismatch) {
            crate::runtime::session_definition::sha256_hash_text(b"mismatched fake Tool request")
        } else {
            prepared.request_hash
        };
        let outcome = if matches!(self.fault, FakeToolExecutionFault::InvalidTerminal) {
            ToolExecutionOutcome {
                status: RunAttemptOutcome::Completed,
                classification: None,
                exit_code: Some(7),
                stdout: Vec::new(),
                stderr: Vec::new(),
            }
        } else {
            self.outcome.clone()
        };
        let mut enforcement = test_enforcement_receipt(policy_digest);
        if matches!(self.fault, FakeToolExecutionFault::InactiveSelfProtection) {
            enforcement.self_protection_active = false;
        }
        Ok(ExecutorDispatchOutcome::Completed(Box::new(
            ExecutorToolExecution {
                enforcement,
                outcome,
                request_hash,
            },
        )))
    }
}

pub(in super::super) struct UnsupportedToolExecutor;

impl ProductiveToolExecutor for UnsupportedToolExecutor {
    type Prepared = ();
    type Waiting = ();

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

    fn preflight(
        &mut self,
        _prepared: Self::Prepared,
    ) -> Result<ProductiveToolPreflight<Self::Waiting>, RuntimeError> {
        unreachable!()
    }

    fn start(&mut self, _waiting: Self::Waiting) -> Result<ExecutorDispatchOutcome, RuntimeError> {
        unreachable!()
    }
}
