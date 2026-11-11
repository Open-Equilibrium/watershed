//! A branch's turns with the checkpoints Watershed's extension recorded at their start and end
//! (ADR-0234).

use super::entry::{Body, Entry, first_line};
use super::trajectory::path;
use crate::Phase;

/// Longest prompt shown for a turn, in characters.
const PROMPT_CHARS: usize = 80;

/// A turn and its checkpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnCheckpoints {
    /// The session id its checkpoints are stored under.
    pub session: String,
    /// The turn id its checkpoints are stored under.
    pub turn: String,
    /// The entry that recorded its start checkpoint.
    pub start_entry: String,
    /// The entry that recorded its end checkpoint, once it ended.
    pub end_entry: Option<String>,
    /// The Trajectory's turn of its first prompt, counted from 1 on the branch; 0 before a
    /// prompt was stored.
    pub number: u64,
    /// The entry of its first prompt; continuing the session from it hands the prompt back.
    pub prompt_entry: Option<String>,
    /// The first line of its first prompt.
    pub prompt: String,
    /// When its start checkpoint was recorded, in Unix milliseconds.
    pub started_ms: Option<u64>,
    /// When its end checkpoint was recorded, in Unix milliseconds.
    pub ended_ms: Option<u64>,
    /// Whether its end checkpoint was taken, so its changes can be shown.
    pub ended: bool,
    /// Why one of its checkpoints could not be taken.
    pub failure: Option<String>,
    /// What it changed, counted when its end checkpoint was taken.
    pub changed: Option<ChangeCount>,
    /// Why the last response before its end checkpoint stopped, as pi stored it, such as `stop`
    /// or `aborted`.
    pub stop_reason: Option<String>,
    /// The error pi stored for that response, when it failed.
    pub stop_error: Option<String>,
}

/// Files and lines a turn changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeCount {
    /// Changed files.
    pub files: u64,
    /// Added lines.
    pub added: u64,
    /// Removed lines.
    pub removed: u64,
}

/// The turns on the branch ending at `leaf`, oldest first.
pub(super) fn turns(entries: &[Entry], leaf: Option<&str>) -> Vec<TurnCheckpoints> {
    let mut turns: Vec<TurnCheckpoints> = Vec::new();
    let mut prompts = 0;
    for entry in path(entries, leaf) {
        match &entry.body {
            Body::User { text, .. } => {
                prompts += 1;
                if let Some(turn) = turns.last_mut()
                    && turn.number == 0
                    && turn.end_entry.is_none()
                {
                    turn.number = prompts;
                    turn.prompt_entry = Some(entry.id.clone());
                    turn.prompt = first_line(text, PROMPT_CHARS);
                }
            }
            Body::Checkpoint {
                session,
                turn,
                phase: Phase::Start,
                error,
                ..
            } => turns.push(TurnCheckpoints {
                session: session.clone(),
                turn: turn.clone(),
                start_entry: entry.id.clone(),
                end_entry: None,
                number: 0,
                prompt_entry: None,
                prompt: String::new(),
                started_ms: entry.at_ms,
                ended_ms: None,
                ended: false,
                failure: error.clone(),
                changed: None,
                stop_reason: None,
                stop_error: None,
            }),
            Body::Assistant {
                stop_reason, error, ..
            } => {
                if let Some(turn) = turns.last_mut().filter(|turn| turn.end_entry.is_none()) {
                    turn.stop_reason = (!stop_reason.is_empty()).then(|| stop_reason.clone());
                    turn.stop_error = error.clone();
                }
            }
            Body::Checkpoint {
                session,
                turn,
                phase: Phase::End,
                error,
                changed,
            } => {
                if let Some(started) = turns
                    .iter_mut()
                    .rev()
                    .find(|started| started.session == *session && started.turn == *turn)
                {
                    started.ended_ms = entry.at_ms;
                    started.end_entry = Some(entry.id.clone());
                    started.changed = *changed;
                    started.ended = error.is_none() && started.failure.is_none();
                    if started.failure.is_none() {
                        started.failure = error.clone();
                    }
                }
            }
            _ => {}
        }
    }
    turns
}
