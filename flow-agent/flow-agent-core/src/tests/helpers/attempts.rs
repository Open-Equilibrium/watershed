use crate::runtime::{
    context::ContextObject,
    run_attempts::{ProductiveAttemptLog, RunAttemptIntent, RunAttemptResult},
    types::RuntimeError,
};

#[derive(Default)]
pub(in crate::tests) struct DiscardAttempts;

impl ProductiveAttemptLog for DiscardAttempts {
    fn persist_objects(&mut self, _objects: &[ContextObject]) -> Result<(), RuntimeError> {
        Ok(())
    }

    fn intent(&mut self, _intent: &RunAttemptIntent) -> Result<(), RuntimeError> {
        Ok(())
    }

    fn terminal(&mut self, _result: &RunAttemptResult) -> Result<(), RuntimeError> {
        Ok(())
    }
}
