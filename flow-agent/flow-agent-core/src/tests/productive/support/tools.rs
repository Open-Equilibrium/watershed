use crate::runtime::{
    fs_guards::AnchoredDir,
    productive::ProductiveToolExecutor,
    run_attempts::RunAttemptOutcome,
    tool_runner::{ToolExecutionOutcome, ToolInvocation},
    types::RuntimeError,
};
use std::time::Duration;
pub(in super::super) struct FakeToolExecutor {
    pub(in super::super) cancel_before_outcome: bool,
    pub(in super::super) error_after_interrupt: bool,
    pub(in super::super) invocations: Vec<ToolInvocation>,
    pub(in super::super) outcome: ToolExecutionOutcome,
}

impl Default for FakeToolExecutor {
    fn default() -> Self {
        Self {
            cancel_before_outcome: false,
            error_after_interrupt: false,
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
    fn supports_productive_tools(&self) -> bool {
        true
    }

    fn execute(
        &mut self,
        invocation: &ToolInvocation,
        _workspace: &AnchoredDir,
        _timeout: Duration,
    ) -> Result<ToolExecutionOutcome, RuntimeError> {
        self.invocations.push(invocation.clone());
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
        Ok(self.outcome.clone())
    }
}

pub(in super::super) struct UnsupportedToolExecutor;

impl ProductiveToolExecutor for UnsupportedToolExecutor {
    fn supports_productive_tools(&self) -> bool {
        false
    }

    fn execute(
        &mut self,
        _invocation: &ToolInvocation,
        _workspace: &AnchoredDir,
        _timeout: Duration,
    ) -> Result<ToolExecutionOutcome, RuntimeError> {
        panic!("unsupported productive Tools must fail before dispatch")
    }
}
