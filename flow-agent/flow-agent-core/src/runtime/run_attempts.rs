use crate::runtime::{
    context::{ContextHistory, ContextObject},
    digest::sha256_hex,
    error::PROVIDER_ERROR_REASON,
    types::{CANCELLED_REASON, RuntimeError},
};
use proto::EventType;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RunAttemptKind {
    Provider,
    Tool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RunAttemptOutcome {
    Completed,
    Failed,
    TimedOut,
    Cancelled,
}

impl Serialize for RunAttemptOutcome {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RunAttemptOutcome {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).ok_or_else(|| {
            serde::de::Error::custom(format!("unknown run attempt outcome: {value}"))
        })
    }
}

impl RunAttemptOutcome {
    const ALL: [Self; 4] = [
        Self::Completed,
        Self::Failed,
        Self::TimedOut,
        Self::Cancelled,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::TimedOut => "timed-out",
            Self::Cancelled => CANCELLED_REASON,
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|outcome| outcome.as_str() == value)
    }

    pub(crate) const fn tool_terminal_event(self) -> EventType {
        match self {
            Self::Completed => EventType::ToolCompleted,
            Self::TimedOut => EventType::ToolTimedOut,
            Self::Failed | Self::Cancelled => EventType::ToolFailed,
        }
    }
}

impl std::fmt::Display for RunAttemptOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolEnforcementExpectation {
    pub(crate) applied_policy_digest: String,
    pub(crate) max_concurrent_processes_and_threads: u32,
    pub(crate) runtime_profile: proto::RuntimeReadProfileV0,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RunAttemptIntent {
    pub(crate) attempt_id: String,
    pub(crate) attempt_kind: RunAttemptKind,
    pub(crate) expected_enforcement: Option<ToolEnforcementExpectation>,
    pub(crate) request_hash: String,
    pub(crate) tool_id: Option<String>,
    pub(crate) timestamp: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RunAttemptResult {
    pub(crate) attempt_id: String,
    pub(crate) attempt_kind: RunAttemptKind,
    pub(crate) outcome: RunAttemptOutcome,
    pub(crate) classification: Option<String>,
    pub(crate) exit_code: Option<i32>,
    pub(crate) timestamp: String,
    pub(crate) durable_output: Option<serde_json::Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderTerminalClassification {
    ProviderError,
    Cancelled,
}

impl ProviderTerminalClassification {
    const ALL: [Self; 2] = [Self::ProviderError, Self::Cancelled];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::ProviderError => PROVIDER_ERROR_REASON,
            Self::Cancelled => CANCELLED_REASON,
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|classification| classification.as_str() == value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolTerminalClassification {
    Cancelled,
    NonzeroExit,
    OutputCollectorFailed,
    OutputDrainTimeout,
    ProcessCapacityExceeded,
    ProcessReapFailed,
    ProcessSetupFailed,
    ProcessSignalFailed,
    ReconciledFailure,
    SignalTermination,
    StderrCapExceeded,
    StdoutCapExceeded,
    StdoutStderrCapExceeded,
    ToolTimedOut,
}

impl ToolTerminalClassification {
    const ALL: [Self; 14] = [
        Self::Cancelled,
        Self::NonzeroExit,
        Self::OutputCollectorFailed,
        Self::OutputDrainTimeout,
        Self::ProcessCapacityExceeded,
        Self::ProcessReapFailed,
        Self::ProcessSetupFailed,
        Self::ProcessSignalFailed,
        Self::ReconciledFailure,
        Self::SignalTermination,
        Self::StderrCapExceeded,
        Self::StdoutCapExceeded,
        Self::StdoutStderrCapExceeded,
        Self::ToolTimedOut,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Cancelled => CANCELLED_REASON,
            Self::NonzeroExit => "nonzero_exit",
            Self::OutputCollectorFailed => "output_collector_failed",
            Self::OutputDrainTimeout => "output_drain_timeout",
            Self::ProcessCapacityExceeded => "process_capacity_exceeded",
            Self::ProcessReapFailed => "process_reap_failed",
            Self::ProcessSetupFailed => "process_setup_failed",
            Self::ProcessSignalFailed => "process_signal_failed",
            Self::ReconciledFailure => "reconciled_failure",
            Self::SignalTermination => "signal_termination",
            Self::StderrCapExceeded => "stderr_cap_exceeded",
            Self::StdoutCapExceeded => "stdout_cap_exceeded",
            Self::StdoutStderrCapExceeded => "stdout_stderr_cap_exceeded",
            Self::ToolTimedOut => "tool_timed_out",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|classification| classification.as_str() == value)
    }

    pub(crate) fn matches_terminal(
        self,
        outcome: RunAttemptOutcome,
        exit_code: Option<i32>,
    ) -> bool {
        match (outcome, self) {
            (RunAttemptOutcome::TimedOut, Self::ToolTimedOut)
            | (RunAttemptOutcome::Cancelled, Self::Cancelled) => exit_code.is_none(),
            (RunAttemptOutcome::Failed, Self::NonzeroExit) => {
                exit_code.is_some_and(|code| code != 0)
            }
            (RunAttemptOutcome::Failed, Self::SignalTermination | Self::ProcessSetupFailed) => {
                exit_code.is_none()
            }
            (
                RunAttemptOutcome::Failed,
                Self::OutputCollectorFailed
                | Self::OutputDrainTimeout
                | Self::ProcessCapacityExceeded
                | Self::ProcessReapFailed
                | Self::ProcessSignalFailed
                | Self::ReconciledFailure
                | Self::StderrCapExceeded
                | Self::StdoutCapExceeded
                | Self::StdoutStderrCapExceeded,
            ) => true,
            _ => false,
        }
    }
}

pub(crate) fn resolve_tool_terminal(
    outcome: RunAttemptOutcome,
    classification: Option<ToolTerminalClassification>,
    exit_code: Option<i32>,
) -> Option<(EventType, Option<ToolTerminalClassification>)> {
    let valid = match outcome {
        RunAttemptOutcome::Completed => classification.is_none() && exit_code == Some(0),
        _ => classification
            .is_some_and(|classification| classification.matches_terminal(outcome, exit_code)),
    };
    valid.then_some((outcome.tool_terminal_event(), classification))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RunAttemptLifecycle {
    Completed,
    Uncertain,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RunAttemptState {
    pub(crate) attempt_id: String,
    pub(crate) attempt_kind: RunAttemptKind,
    pub(crate) lifecycle: RunAttemptLifecycle,
    pub(crate) outcome: Option<RunAttemptOutcome>,
    pub(crate) expected_enforcement: Option<ToolEnforcementExpectation>,
    pub(crate) request_hash: String,
    pub(crate) timestamp: String,
    pub(crate) tool_id: Option<String>,
}

pub(crate) trait ProductiveAttemptLog {
    fn persist_objects(&mut self, objects: &[ContextObject]) -> Result<(), RuntimeError>;

    fn intent(&mut self, intent: &RunAttemptIntent) -> Result<(), RuntimeError>;

    fn terminal(&mut self, result: &RunAttemptResult) -> Result<(), RuntimeError>;
}

pub(crate) trait ProductiveRecovery {
    fn recover_attempt(
        &mut self,
        _kind: RunAttemptKind,
        _attempt_id: &str,
        _request_hash: &str,
        _tool_id: Option<&str>,
    ) -> Result<Option<RunAttemptResult>, RuntimeError> {
        Ok(None)
    }

    fn record_attempt(
        &mut self,
        _tool_id: Option<&str>,
        _request_hash: &str,
        _result: &RunAttemptResult,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }

    fn phase_boundary(
        &mut self,
        _flow_execution_id: &str,
        _phase_execution_id: &str,
        _phase_id: &str,
        _iteration: u8,
        _result: &core_script::FlowValue,
        _will_repeat: bool,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }

    fn transition_boundary(
        &mut self,
        _flow_execution_id: &str,
        _from_phase_id: &str,
        _to_phase_id: Option<&str>,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }

    fn flow_boundary(
        &mut self,
        _flow_execution_id: &str,
        _result: Option<&core_script::FlowValue>,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }

    fn terminal_boundary(
        &mut self,
        _history: &ContextHistory,
        _failed: bool,
        _run_event_count: u64,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }

    fn read_object(&self, _uri: &str) -> Result<Vec<u8>, RuntimeError> {
        Err(RuntimeError::Protocol(
            "productive recovery object access is unavailable".to_owned(),
        ))
    }

    fn terminal_snapshot_hash(&self) -> Option<&str> {
        None
    }
}

pub(crate) fn read_verified_session_object(
    recovery: &dyn ProductiveRecovery,
    uri: &str,
    description: &str,
) -> Result<Vec<u8>, RuntimeError> {
    let bytes = recovery.read_object(uri)?;
    let expected_uri =
        core_script::build_session_object_uri(&sha256_hex(&bytes)).map_err(|error| {
            RuntimeError::Protocol(format!("{description} URI is invalid: {error}"))
        })?;
    if expected_uri != uri {
        return Err(RuntimeError::Protocol(format!(
            "{description} does not match its URI digest"
        )));
    }
    Ok(bytes)
}
