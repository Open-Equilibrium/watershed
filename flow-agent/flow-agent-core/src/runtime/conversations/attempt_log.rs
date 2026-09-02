use super::{run_log::RunAttemptLedger, run_objects::RunObjectStore};
use crate::runtime::{
    context::ContextObject,
    run_attempts::{ProductiveAttemptLog, RunAttemptIntent, RunAttemptResult},
    types::RuntimeError,
};

pub(crate) struct ConversationAttemptLog {
    ledger: RunAttemptLedger,
    run_objects: RunObjectStore,
}

impl ConversationAttemptLog {
    #[cfg(test)]
    pub(crate) fn open(
        workspace: &std::path::Path,
        conversation_id: &str,
        run_session_id: &str,
    ) -> Result<Self, RuntimeError> {
        let run_objects = RunObjectStore::open(workspace, conversation_id, run_session_id)?;
        Self::open_with_run_objects(workspace, conversation_id, run_session_id, run_objects)
    }

    pub(crate) fn open_with_run_objects(
        workspace: &std::path::Path,
        conversation_id: &str,
        run_session_id: &str,
        run_objects: RunObjectStore,
    ) -> Result<Self, RuntimeError> {
        Ok(Self {
            ledger: RunAttemptLedger::open(workspace, conversation_id, run_session_id)?,
            run_objects,
        })
    }
}

impl ProductiveAttemptLog for ConversationAttemptLog {
    fn persist_objects(&mut self, objects: &[ContextObject]) -> Result<(), RuntimeError> {
        self.run_objects.persist(objects)
    }

    fn intent(&mut self, intent: &RunAttemptIntent) -> Result<(), RuntimeError> {
        self.ledger.append_intent(intent)
    }

    fn terminal(&mut self, result: &RunAttemptResult) -> Result<(), RuntimeError> {
        self.ledger.append_result(result)
    }
}
