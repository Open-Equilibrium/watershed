//! A stored session's record (ADR-0257): what Meta-Harness already records about a session, as
//! JSON for the person. It reads the session file and the project's checkpoints, records nothing
//! new and claims no completeness (ADR-0222).

use std::path::Path;

use serde_json::{Value, json};

use crate::MAX_RECORD_BYTES;
use crate::{
    ChangeStatus, Checkpoint, Checkpoints, DiffError, History, Phase, TurnCheckpoints, Whitespace,
};

/// The record of the session stored in `file`, as the JSON text of `record.json`: its turns with
/// their checkpoints and the files each changed. `project` is the folder the session works in,
/// `host` the machine that ran it, and `id` and `title` name the session as Liquid shows it.
pub async fn session_record(
    checkpoints: &Checkpoints,
    file: &Path,
    project: &Path,
    host: &str,
    id: &str,
    title: &str,
) -> Result<String, DiffError> {
    let history = History::from_file(file, MAX_RECORD_BYTES).map_err(DiffError::Io)?;
    // A stored file names no leaf; pi continues the session from its last entry.
    let mut turns = Vec::new();
    for turn in history.turns(history.last_id()) {
        turns.push(turn_record(checkpoints, &turn).await?);
    }
    let branch = match checkpoints.status().await {
        Ok(status) => status.branch,
        Err(DiffError::NotARepository) => None,
        Err(error) => return Err(error),
    };
    let record = json!({
        "schema": "watershed-session-record-v1",
        "host": host,
        "harness": "pi",
        "project": project.display().to_string(),
        "branch": branch,
        "session": { "id": id, "title": title },
        "turns": turns,
    });
    serde_json::to_string_pretty(&record)
        .map_err(|error| DiffError::Io(std::io::Error::other(error)))
}

/// One turn: its times, how it ended, its checkpoint commits and the files between them. Files
/// are `null` when either checkpoint is missing, so they are not claimed empty.
async fn turn_record(
    checkpoints: &Checkpoints,
    turn: &TurnCheckpoints,
) -> Result<Value, DiffError> {
    let start = Checkpoint {
        session: turn.session.clone(),
        turn: turn.turn.clone(),
        phase: Phase::Start,
    };
    let end = Checkpoint {
        phase: Phase::End,
        ..start.clone()
    };
    let start_commit = checkpoints.commit_of(&start).await?;
    let end_commit = checkpoints.commit_of(&end).await?;
    let files = match (&start_commit, &end_commit) {
        (Some(_), Some(_)) => Some(changed_files(checkpoints, &start, &end).await?),
        _ => None,
    };
    Ok(json!({
        "turn": turn.turn,
        "started_ms": turn.started_ms,
        "ended_ms": turn.ended_ms,
        "outcome": outcome(turn),
        "error": turn.failure.as_ref().or(turn.stop_error.as_ref()),
        "stop_reason": turn.stop_reason,
        "recorded": turn.changed.map(|count| json!({
            "files": count.files,
            "added": count.added,
            "removed": count.removed,
        })),
        "start_commit": start_commit,
        "end_commit": end_commit,
        "files": files,
    }))
}

/// How a turn ended as pi and Watershed stored it: aborted or failed by its last response or a
/// checkpoint, unfinished without an end checkpoint, otherwise completed.
fn outcome(turn: &TurnCheckpoints) -> &'static str {
    match turn.stop_reason.as_deref() {
        Some("aborted") => "aborted",
        Some("error") => "error",
        _ if turn.failure.is_some() => "error",
        _ if turn.end_entry.is_none() => "unfinished",
        _ => "completed",
    }
}

/// The files that changed between two checkpoints, with their added and removed lines.
async fn changed_files(
    checkpoints: &Checkpoints,
    from: &Checkpoint,
    to: &Checkpoint,
) -> Result<Value, DiffError> {
    let changes = checkpoints.changes(from, to, Whitespace::Shown).await?;
    Ok(changes
        .into_iter()
        .map(|change| {
            json!({
                "path": change.path,
                "status": match change.status {
                    ChangeStatus::Added => "added",
                    ChangeStatus::Modified => "modified",
                    ChangeStatus::Deleted => "deleted",
                },
                "added": change.added,
                "removed": change.removed,
            })
        })
        .collect())
}
