use super::legacy_migration::{
    legacy_flat_compatibility_is_available, legacy_session_is_terminal,
    migrate_legacy_session_if_present,
};
pub use super::session_event_stream::SessionEventReader;
pub(crate) use super::session_event_stream::ensure_in_memory_replay_output_limit;
use crate::runtime::types::RuntimeError;
use std::{io, path::Path};

impl SessionEventReader {
    /// Opens a session's validated log boundary without reading event payloads yet.
    pub fn open(workspace: impl AsRef<Path>, session_id: &str) -> Result<Self, RuntimeError> {
        let workspace_path = workspace.as_ref();
        legacy_flat_compatibility_is_available(workspace_path, session_id)?;
        match Self::open_flat(workspace_path, session_id) {
            Ok(reader) => Ok(reader),
            Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
                Self::open_conversation_run(workspace_path, session_id, session_id)
            }
            Err(error) => Err(error),
        }
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
        let workspace = workspace.as_ref();
        if conversation_id == run_session_id {
            if legacy_session_is_terminal(workspace, conversation_id)? == Some(false) {
                return Self::open_flat(workspace, run_session_id);
            }
            migrate_legacy_session_if_present(workspace, conversation_id)?;
        }
        Self::open_conversation_run_raw(workspace, conversation_id, run_session_id)
    }
}
