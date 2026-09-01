use crate::runtime::{
    context::ContextObject,
    run_attempts::{ProductiveAttemptLog, RunAttemptKind, RunAttemptResult},
    types::RuntimeError,
};
#[derive(Default)]
pub(in super::super) struct MemoryAttempts {
    pub(in super::super) durable_outputs: Vec<Option<serde_json::Value>>,
    pub(in super::super) intents: Vec<(RunAttemptKind, String, Option<String>)>,
    pub(in super::super) objects: Vec<ContextObject>,
    pub(in super::super) results: Vec<(RunAttemptKind, String, String, Option<String>)>,
    pub(in super::super) terminal_results: Vec<RunAttemptResult>,
    pub(in super::super) timestamps: Vec<String>,
}

impl ProductiveAttemptLog for MemoryAttempts {
    fn persist_objects(&mut self, objects: &[ContextObject]) -> Result<(), RuntimeError> {
        self.objects.extend_from_slice(objects);
        Ok(())
    }

    fn intent(
        &mut self,
        kind: RunAttemptKind,
        attempt_id: &str,
        _request_hash: &str,
        tool_id: Option<&str>,
        timestamp: &str,
    ) -> Result<(), RuntimeError> {
        self.intents
            .push((kind, attempt_id.to_owned(), tool_id.map(str::to_owned)));
        self.timestamps.push(timestamp.to_owned());
        Ok(())
    }

    fn terminal(&mut self, result: &RunAttemptResult) -> Result<(), RuntimeError> {
        self.durable_outputs.push(result.durable_output.clone());
        self.terminal_results.push(result.clone());
        self.results.push((
            result.attempt_kind,
            result.attempt_id.clone(),
            result.outcome.to_string(),
            result.classification.clone(),
        ));
        Ok(())
    }
}
