mod invocation;
#[cfg(all(unix, any(test, feature = "m11-budget-evidence")))]
mod unix_process;

use crate::runtime::run_attempts::RunAttemptOutcome;
pub(crate) use crate::runtime::run_attempts::ToolTerminalClassification;
pub(crate) use invocation::{build_tool_invocation, validate_parameter_value};
#[cfg(test)]
pub(crate) use invocation::{encoded_exec_vector_bytes, validate_tool_invocation};
#[cfg(all(test, unix))]
pub(crate) use unix_process::{
    PrimaryTrigger, READY_CANCELLATION_MARKER, force_reap_timeout_for_test, visible_exit_code,
};
#[cfg(all(unix, any(test, feature = "m11-budget-evidence")))]
pub(crate) use unix_process::{
    ToolRunControl, execute_tool_invocation, measure_ready_process_group_cleanup,
    measure_ready_tool_cancellation,
};

pub(crate) const MAX_TOOL_EXEC_ENTRIES: usize = 2_048;
pub(crate) const MAX_TOOL_EXEC_BYTES: usize = 128 * 1024;
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) const MAX_TOOL_STREAM_BYTES: usize = proto::MAX_EXECUTOR_TOOL_STREAM_BYTES_V0;
pub(crate) const OWN_SCRIPT_EXECUTABLE: &str = "/bin/sh";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToolInvocation {
    pub(crate) executable: String,
    pub(crate) argv: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ToolRunnerError {
    ExecByteBudget { actual: usize },
    ExecEntryBudget { actual: usize },
    InvalidParameter(String),
    NulByte,
    PatternMatcherUnavailable,
    UnsupportedCommand,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToolExecutionOutcome {
    pub(crate) status: RunAttemptOutcome,
    pub(crate) classification: Option<ToolTerminalClassification>,
    pub(crate) exit_code: Option<i32>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

impl ToolExecutionOutcome {
    #[cfg(any(unix, test))]
    pub(crate) fn cancelled() -> Self {
        Self {
            status: RunAttemptOutcome::Cancelled,
            classification: Some(ToolTerminalClassification::Cancelled),
            exit_code: None,
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    pub(crate) fn mark_cancelled(&mut self) {
        self.status = RunAttemptOutcome::Cancelled;
        self.classification = Some(ToolTerminalClassification::Cancelled);
        self.exit_code = None;
    }
}
