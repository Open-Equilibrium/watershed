use crate::runtime::{
    context::{ContextHistory, ContextObject},
    productive::ProductiveCompletionCommitPoint,
    run_attempts::{ProductiveRecovery, RunAttemptKind, RunAttemptOutcome, RunAttemptResult},
    types::RuntimeError,
};
use std::{cell::Cell, collections::BTreeMap};
pub(in super::super) struct RecoveryObjectTerminal;

impl ProductiveRecovery for RecoveryObjectTerminal {
    fn terminal_boundary(
        &mut self,
        history: &ContextHistory,
        _failed: bool,
        _run_event_count: u64,
    ) -> Result<(), RuntimeError> {
        let object = history.recovery_object()?;
        ContextHistory::from_recovery_bytes(&object.bytes).map(|_| ())
    }
}

#[derive(Default)]
pub(in super::super) struct ObjectRecovery(pub(in super::super) BTreeMap<String, Vec<u8>>);

impl ObjectRecovery {
    pub(in super::super) fn from_objects(objects: &[ContextObject]) -> Self {
        Self(
            objects
                .iter()
                .map(|object| {
                    (
                        format!("session-object:sha256:{}", object.digest),
                        object.bytes.clone(),
                    )
                })
                .collect(),
        )
    }
}

impl ProductiveRecovery for ObjectRecovery {
    fn read_object(&self, uri: &str) -> Result<Vec<u8>, RuntimeError> {
        self.0
            .get(uri)
            .cloned()
            .ok_or_else(|| RuntimeError::Protocol(format!("missing fixture object {uri}")))
    }
}

pub(in super::super) struct CountingObjectRecovery {
    pub(in super::super) objects: ObjectRecovery,
    pub(in super::super) reads: Cell<usize>,
}

impl ProductiveRecovery for CountingObjectRecovery {
    fn read_object(&self, uri: &str) -> Result<Vec<u8>, RuntimeError> {
        self.reads.set(self.reads.get() + 1);
        self.objects.read_object(uri)
    }
}

pub(in super::super) struct DefaultRecovery;

impl ProductiveRecovery for DefaultRecovery {}

#[derive(Default)]
pub(in super::super) struct CompletionBoundaryRecordingRecovery {
    pub(in super::super) commits: Vec<ProductiveCompletionCommitPoint>,
}

impl ProductiveRecovery for CompletionBoundaryRecordingRecovery {
    fn phase_boundary(
        &mut self,
        _flow_execution_id: &str,
        _phase_execution_id: &str,
        _phase_id: &str,
        _iteration: u8,
        _result: &core_script::FlowValue,
        _will_repeat: bool,
    ) -> Result<(), RuntimeError> {
        self.commits
            .push(ProductiveCompletionCommitPoint::PhaseRecovery);
        Ok(())
    }

    fn transition_boundary(
        &mut self,
        _flow_execution_id: &str,
        _from_phase_id: &str,
        _to_phase_id: Option<&str>,
    ) -> Result<(), RuntimeError> {
        self.commits
            .push(ProductiveCompletionCommitPoint::TransitionRecovery);
        Ok(())
    }

    fn flow_boundary(
        &mut self,
        _flow_execution_id: &str,
        _result: Option<&core_script::FlowValue>,
    ) -> Result<(), RuntimeError> {
        self.commits
            .push(ProductiveCompletionCommitPoint::FlowRecovery);
        Ok(())
    }
}

pub(in super::super) enum InjectedAttemptRecovery {
    ProviderError,
    ProviderResult(RunAttemptResult),
    ToolResult(RunAttemptResult),
    ToolWrongKind,
}

impl ProductiveRecovery for InjectedAttemptRecovery {
    fn recover_attempt(
        &mut self,
        kind: RunAttemptKind,
        _attempt_id: &str,
        _request_hash: &str,
        _tool_id: Option<&str>,
    ) -> Result<Option<RunAttemptResult>, RuntimeError> {
        match self {
            Self::ProviderError if kind == RunAttemptKind::Provider => Err(RuntimeError::Protocol(
                "fixture provider recovery failure".to_owned(),
            )),
            Self::ProviderResult(result) if kind == RunAttemptKind::Provider => {
                Ok(Some(result.clone()))
            }
            Self::ToolResult(result) if kind == RunAttemptKind::Tool => Ok(Some(result.clone())),
            Self::ToolWrongKind if kind == RunAttemptKind::Tool => Ok(Some(RunAttemptResult {
                attempt_id: "tool-000001".to_owned(),
                attempt_kind: RunAttemptKind::Provider,
                outcome: RunAttemptOutcome::Completed,
                classification: None,
                exit_code: None,
                timestamp: "2026-07-30T12:00:00Z".to_owned(),
                durable_output: Some(serde_json::json!({})),
            })),
            _ => Ok(None),
        }
    }
}

#[derive(Clone, Copy)]
pub(in super::super) enum FailingRecoveryBoundary {
    RecordAttempt,
    Phase,
    Transition,
    Flow,
    Terminal,
}

pub(in super::super) struct FailingBoundaryRecovery(pub(in super::super) FailingRecoveryBoundary);

impl FailingBoundaryRecovery {
    fn failure(&self, boundary: &str) -> RuntimeError {
        RuntimeError::Protocol(format!("fixture {boundary} recovery failure"))
    }
}

impl ProductiveRecovery for FailingBoundaryRecovery {
    fn record_attempt(
        &mut self,
        _tool_id: Option<&str>,
        _request_hash: &str,
        _result: &RunAttemptResult,
    ) -> Result<(), RuntimeError> {
        match self.0 {
            FailingRecoveryBoundary::RecordAttempt => Err(self.failure("attempt")),
            _ => Ok(()),
        }
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
        match self.0 {
            FailingRecoveryBoundary::Phase => Err(self.failure("Phase")),
            _ => Ok(()),
        }
    }

    fn transition_boundary(
        &mut self,
        _flow_execution_id: &str,
        _from_phase_id: &str,
        _to_phase_id: Option<&str>,
    ) -> Result<(), RuntimeError> {
        match self.0 {
            FailingRecoveryBoundary::Transition => Err(self.failure("Transition")),
            _ => Ok(()),
        }
    }

    fn flow_boundary(
        &mut self,
        _flow_execution_id: &str,
        _result: Option<&core_script::FlowValue>,
    ) -> Result<(), RuntimeError> {
        match self.0 {
            FailingRecoveryBoundary::Flow => Err(self.failure("Flow")),
            _ => Ok(()),
        }
    }

    fn terminal_boundary(
        &mut self,
        _history: &ContextHistory,
        _failed: bool,
        _run_event_count: u64,
    ) -> Result<(), RuntimeError> {
        match self.0 {
            FailingRecoveryBoundary::Terminal => Err(self.failure("terminal")),
            _ => Ok(()),
        }
    }
}
