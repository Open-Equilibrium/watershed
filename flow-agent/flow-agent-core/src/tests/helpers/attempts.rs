use crate::runtime::{
    context::ContextObject,
    run_attempts::{ProductiveAttemptLog, RunAttemptKind, RunAttemptResult},
    types::RuntimeError,
};

#[derive(Default)]
pub(in crate::tests) struct DiscardAttempts;

impl ProductiveAttemptLog for DiscardAttempts {
    fn persist_objects(&mut self, _objects: &[ContextObject]) -> Result<(), RuntimeError> {
        Ok(())
    }

    fn intent(
        &mut self,
        _kind: RunAttemptKind,
        _attempt_id: &str,
        _request_hash: &str,
        _tool_id: Option<&str>,
        _timestamp: &str,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }

    fn terminal(&mut self, _result: &RunAttemptResult) -> Result<(), RuntimeError> {
        Ok(())
    }
}
