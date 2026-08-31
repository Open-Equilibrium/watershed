pub use super::session_event_stream::SessionEventReader;
pub(crate) use super::session_event_stream::ensure_in_memory_replay_output_limit;
use crate::runtime::types::RuntimeError;
use std::path::Path;

impl SessionEventReader {
    /// Opens a session's validated log boundary without reading event payloads yet.
    pub fn open(workspace: impl AsRef<Path>, session_id: &str) -> Result<Self, RuntimeError> {
        Self::open_flat(workspace.as_ref(), session_id)
    }

    /// Opens one validated run log owned by the addressed conversation.
    pub fn open_conversation_run(
        workspace: impl AsRef<Path>,
        conversation_id: &str,
        run_session_id: &str,
    ) -> Result<Self, RuntimeError> {
        if !proto::is_valid_session_id(conversation_id)
            || !proto::is_valid_session_id(run_session_id)
        {
            return Err(RuntimeError::Usage(
                "invalid conversation or run session id".to_owned(),
            ));
        }
        Self::open_conversation_run_raw(workspace.as_ref(), conversation_id, run_session_id)
    }
}
