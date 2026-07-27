use crate::runtime::{
    session_reading::read_existing_session,
    types::{EmitMode, RunOutput, RuntimeError},
};
use std::path::Path;

/// Replays a persisted terminal or non-terminal session log without modifying it.
pub fn replay_session(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    read_existing_session(workspace.as_ref(), session_id, emit)
}
