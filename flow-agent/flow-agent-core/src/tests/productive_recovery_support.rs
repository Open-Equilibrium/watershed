use crate::runtime::{
    context::ContextHistory,
    run_attempts::{ProductiveRecovery, RunAttemptKind, RunAttemptResult},
    types::RuntimeError,
};

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum ProductiveInterruptionPoint {
    Phase,
    Terminal,
}

pub(super) struct InterruptingProductiveRecovery<'a, R> {
    inner: &'a mut R,
    point: ProductiveInterruptionPoint,
}

impl<'a, R> InterruptingProductiveRecovery<'a, R> {
    pub(super) fn new(inner: &'a mut R, point: ProductiveInterruptionPoint) -> Self {
        Self { inner, point }
    }
}

impl<R: ProductiveRecovery> ProductiveRecovery for InterruptingProductiveRecovery<'_, R> {
    fn recover_attempt(
        &mut self,
        kind: RunAttemptKind,
        attempt_id: &str,
        request_hash: &str,
        tool_id: Option<&str>,
    ) -> Result<Option<RunAttemptResult>, RuntimeError> {
        self.inner
            .recover_attempt(kind, attempt_id, request_hash, tool_id)
    }

    fn record_attempt(
        &mut self,
        tool_id: Option<&str>,
        request_hash: &str,
        result: &RunAttemptResult,
    ) -> Result<(), RuntimeError> {
        self.inner.record_attempt(tool_id, request_hash, result)
    }

    fn phase_boundary(
        &mut self,
        flow_execution_id: &str,
        phase_execution_id: &str,
        phase_id: &str,
        iteration: u8,
        result: &core_script::FlowValue,
        will_repeat: bool,
    ) -> Result<(), RuntimeError> {
        if self.point == ProductiveInterruptionPoint::Phase {
            return Err(RuntimeError::Protocol(
                "fixture interruption after committed provider result".to_owned(),
            ));
        }
        self.inner.phase_boundary(
            flow_execution_id,
            phase_execution_id,
            phase_id,
            iteration,
            result,
            will_repeat,
        )
    }

    fn transition_boundary(
        &mut self,
        flow_execution_id: &str,
        from_phase_id: &str,
        to_phase_id: Option<&str>,
    ) -> Result<(), RuntimeError> {
        self.inner
            .transition_boundary(flow_execution_id, from_phase_id, to_phase_id)
    }

    fn flow_boundary(
        &mut self,
        flow_execution_id: &str,
        result: Option<&core_script::FlowValue>,
    ) -> Result<(), RuntimeError> {
        self.inner.flow_boundary(flow_execution_id, result)
    }

    fn terminal_boundary(
        &mut self,
        _history: &ContextHistory,
        _failed: bool,
        _run_event_count: u64,
    ) -> Result<(), RuntimeError> {
        Err(RuntimeError::Protocol(
            "fixture interruption prevents a terminal snapshot".to_owned(),
        ))
    }

    fn read_object(&self, uri: &str) -> Result<Vec<u8>, RuntimeError> {
        self.inner.read_object(uri)
    }
}
