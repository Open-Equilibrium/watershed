use super::{executor_error, invalid_request};
use crate::runtime::{
    run_attempts::{RunAttemptOutcome, ToolTerminalClassification},
    tool_runner::ToolExecutionOutcome,
    types::RuntimeError,
};

pub(super) fn validate_receipt_identity(
    receipt: &proto::EnforcementReceiptV0,
    probe: &proto::ExecutorProbeV0,
) -> Result<(), RuntimeError> {
    if receipt.executor != probe.executor
        || receipt.executor_version != probe.executor_version
        || receipt.backend != probe.backend
        || receipt.backend_version != probe.backend_version
        || receipt.platform != probe.platform
    {
        return Err(executor_error(
            proto::ExecutorErrorCodeV0::InvalidResponse,
            "Executor enforcement receipt identity does not match readiness",
        ));
    }
    Ok(())
}

pub(super) fn decode_tool_outcome(
    result: proto::ExecutorToolResultV0,
) -> Result<ToolExecutionOutcome, RuntimeError> {
    let status = match result.status {
        proto::ExecutorToolStatusV0::Completed => RunAttemptOutcome::Completed,
        proto::ExecutorToolStatusV0::Failed => RunAttemptOutcome::Failed,
        proto::ExecutorToolStatusV0::TimedOut => RunAttemptOutcome::TimedOut,
        proto::ExecutorToolStatusV0::Cancelled => RunAttemptOutcome::Cancelled,
    };
    let classification = result
        .classification
        .map(|classification| match classification {
            proto::ExecutorToolClassificationV0::NonzeroExit => {
                ToolTerminalClassification::NonzeroExit
            }
            proto::ExecutorToolClassificationV0::ProcessCapacityExceeded => {
                ToolTerminalClassification::ProcessCapacityExceeded
            }
            proto::ExecutorToolClassificationV0::SignalTermination => {
                ToolTerminalClassification::SignalTermination
            }
            proto::ExecutorToolClassificationV0::StderrCapExceeded => {
                ToolTerminalClassification::StderrCapExceeded
            }
            proto::ExecutorToolClassificationV0::StdoutCapExceeded => {
                ToolTerminalClassification::StdoutCapExceeded
            }
            proto::ExecutorToolClassificationV0::StdoutStderrCapExceeded => {
                ToolTerminalClassification::StdoutStderrCapExceeded
            }
            proto::ExecutorToolClassificationV0::ToolTimedOut => {
                ToolTerminalClassification::ToolTimedOut
            }
            proto::ExecutorToolClassificationV0::OutputCollectorFailed => {
                ToolTerminalClassification::OutputCollectorFailed
            }
            proto::ExecutorToolClassificationV0::OutputDrainTimeout => {
                ToolTerminalClassification::OutputDrainTimeout
            }
            proto::ExecutorToolClassificationV0::Cancelled => ToolTerminalClassification::Cancelled,
        });
    Ok(ToolExecutionOutcome {
        status,
        classification,
        exit_code: result.exit_code,
        stderr: proto::decode_executor_stream_v0(&result.stderr_base64).map_err(invalid_request)?,
        stdout: proto::decode_executor_stream_v0(&result.stdout_base64).map_err(invalid_request)?,
    })
}
