//! A session's stored history for the Trajectory tab: its branch tree and Trajectories.

use crate::api::events::{BridgeItemKind, BridgeUsage};

/// The session's branch tree and the requested Trajectories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeHistory {
    /// Every entry; parents precede their children.
    pub tree: Vec<BridgeTreeNode>,
    /// The entry the next prompt continues from.
    pub leaf: Option<String>,
    /// One Trajectory per requested branch, in request order.
    pub trajectories: Vec<BridgeTrajectory>,
    /// What every stored response used, on every branch.
    pub usage: BridgeUsage,
}

/// One entry of the branch tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTreeNode {
    /// The entry id.
    pub id: String,
    /// The parent entry, or `None` for a root.
    pub parent: Option<String>,
    /// What the entry holds.
    pub kind: BridgeEntryKind,
    /// A one-line preview of its text or tool.
    pub preview: String,
    /// The label set on this entry.
    pub label: Option<String>,
    /// Unix milliseconds when the entry was stored, if known.
    pub at_ms: Option<i64>,
    /// Whether a tool call or model response failed.
    pub failed: bool,
}

/// What a tree entry holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeEntryKind {
    /// A user prompt.
    User,
    /// A model response.
    Assistant,
    /// A tool result.
    ToolResult,
    /// A change of the system prompt or tools.
    System,
    /// An extension's entry.
    Custom,
    /// Anything else.
    Other,
}

/// One branch from its first entry to its leaf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTrajectory {
    /// The rows in order.
    pub records: Vec<BridgeRecord>,
    /// Milliseconds from the first to the last entry.
    pub duration_ms: i64,
}

/// One Trajectory row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRecord {
    /// The entry id, used to branch.
    pub entry: String,
    /// The turn on this branch, counted from 1.
    pub turn: i64,
    /// What the row shows.
    pub kind: BridgeRecordKind,
    /// The prompt, the response's first line or the tool name.
    pub label: String,
    /// Tool arguments as JSON text.
    pub detail: String,
    /// What a tool call works on, such as a command or a path, on one line.
    pub target: String,
    /// The file a successful file change changed.
    pub changed_file: Option<String>,
    /// Milliseconds from the branch's first entry to the start.
    pub start_ms: i64,
    /// How long it took, when measured.
    pub duration_ms: Option<i64>,
    /// Milliseconds until a response's first streamed content.
    pub first_token_ms: Option<i64>,
    /// The model of a response.
    pub model: String,
    /// Tokens and cost of a response.
    pub usage: Option<BridgeUsage>,
    /// Why a response ended.
    pub stop_reason: String,
    /// Whether a tool call or response failed.
    pub failed: bool,
    /// What the model saw for a response.
    pub context: Option<BridgeStepContext>,
    /// What a tool call did.
    pub outcome: BridgeToolOutcome,
    /// Characters the step adds to the model's context: its prompt, response or tool result.
    pub added_chars: i64,
}

/// What a tool call did, as far as its stored result shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BridgeToolOutcome {
    /// Lines an edit added or a write wrote.
    pub added: Option<i64>,
    /// Lines an edit removed.
    pub removed: Option<i64>,
    /// A shell command's exit code.
    pub exit_code: Option<i32>,
    /// Tests that passed, from a common test runner's output.
    pub tests_passed: Option<i64>,
    /// Tests that failed.
    pub tests_failed: Option<i64>,
}

/// What a Trajectory row shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeRecordKind {
    /// A user prompt.
    User,
    /// A model request and its response.
    Message,
    /// A tool call and its result.
    Tool,
}

/// What the agent sent to the model for one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeStepContext {
    /// Characters of the system prompt.
    pub prompt_chars: i64,
    /// The system prompt's named sections, such as `preamble` or `project_context`.
    pub sections: Vec<String>,
    /// The tools the model could call.
    pub tools: Vec<String>,
    /// Characters of the tools' declarations.
    pub tool_chars: i64,
    /// Messages in the conversation before the request.
    pub messages: i64,
    /// Characters of those messages.
    pub message_chars: i64,
    /// What changed since the previous request.
    pub new_since_previous: Vec<String>,
}

/// Everything the model saw for one request, in parts with their length in characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeContextDetail {
    /// Sections of the system prompt.
    pub sections: Vec<BridgeContextPart>,
    /// Declared tools.
    pub tools: Vec<BridgeContextPart>,
    /// Messages on the branch up to the request.
    pub messages: Vec<BridgeContextMessage>,
}

/// A named part of the model's context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeContextPart {
    /// Its name.
    pub name: String,
    /// Its length in characters.
    pub chars: i64,
}

/// A message on the branch up to a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeContextMessage {
    /// The entry id.
    pub entry: String,
    /// What it is, such as `result of bash · npm test`.
    pub label: String,
    /// Its length in characters.
    pub chars: i64,
    /// Whether the model saw it; a compaction or edit can take it out.
    pub in_context: bool,
    /// Whether it arrived since the previous request.
    pub fresh: bool,
}

/// A page of the chat of the session's current branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTranscript {
    /// The items in order.
    pub items: Vec<BridgeTranscriptItem>,
    /// Whether earlier entries remain.
    pub more: bool,
    /// The earliest entry the page covers, even when it shows none of them; the next page
    /// ends before it.
    pub start: Option<String>,
}

/// One chat item rebuilt from a stored entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTranscriptItem {
    /// The entry it comes from.
    pub entry: String,
    /// What it is.
    pub kind: BridgeItemKind,
    /// A prompt's or response's text.
    pub text: String,
    /// A tool call's tool.
    pub tool: Option<String>,
    /// What a tool call works on, such as a command or a path, on one line.
    pub target: Option<String>,
    /// Whether a tool call failed.
    pub failed: bool,
    /// How long a tool call ran.
    pub duration_ms: Option<i64>,
    /// The lines a file change added.
    pub added: Option<i64>,
    /// The lines a file change removed.
    pub removed: Option<i64>,
}

/// A turn of the current branch, as the Diff tab lists it (ADR-0234).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTurn {
    /// The turn's checkpoint id.
    pub id: String,
    /// The entry that recorded its start checkpoint; the chat places its changes after the
    /// turn's entries.
    pub start_entry: String,
    /// The entry that recorded its end checkpoint, once it ended.
    pub end_entry: Option<String>,
    /// The Trajectory's turn of its first prompt; 0 before a prompt was stored.
    pub number: i64,
    /// The entry of its first prompt, before which a revert continues the session.
    pub prompt_entry: Option<String>,
    /// The first line of its first prompt.
    pub prompt: String,
    /// Unix milliseconds when it started, if known.
    pub started_ms: Option<i64>,
    /// Unix milliseconds when it ended, if it did.
    pub ended_ms: Option<i64>,
    /// Whether its end checkpoint was taken, so its changes can be shown.
    pub ended: bool,
    /// Why one of its checkpoints could not be taken.
    pub failure: Option<String>,
    /// Files it changed, counted at its end.
    pub files: Option<i64>,
    /// Lines it added, counted at its end.
    pub added: Option<i64>,
    /// Lines it removed, counted at its end.
    pub removed: Option<i64>,
}

/// Whether the changes between two checkpoints could be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeChangesState {
    /// The changes are listed.
    Shown,
    /// The project is not in a Git repository, so it has no checkpoints.
    OutsideGit,
    /// A checkpoint was removed.
    Removed,
}

/// The files that changed between two checkpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTurnChanges {
    /// Whether they could be read.
    pub state: BridgeChangesState,
    /// The changed files; empty unless shown.
    pub files: Vec<crate::api::events::BridgeFileChange>,
}

/// What reverting the files to before a turn would change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRevertPlan {
    /// Whether the turn's start checkpoint could be read.
    pub state: BridgeChangesState,
    /// The files below the working folder that would change, sorted by path.
    pub files: Vec<BridgeRevertFile>,
    /// The agents running in the checkout, which refuse the revert.
    pub running: Vec<BridgeRunningAgent>,
    /// The branch checked out, as Liquid shows it, or none when HEAD names a commit.
    pub branch: Option<String>,
}

/// One file a revert would change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRevertFile {
    /// The path relative to the root of the Git repository.
    pub path: String,
    /// The path as Liquid shows it, with characters that could hide or reorder it written out.
    pub shown: String,
    /// How the revert changes it: `Added` writes back a file deleted since the turn started,
    /// `Deleted` deletes a file new since then.
    pub status: crate::api::events::BridgeChangeStatus,
    /// Whether it changed after the last turn ended, so the revert also undoes that edit.
    pub edited: bool,
    /// Unix milliseconds when it was last written, if it exists.
    pub modified_ms: Option<i64>,
    /// Whether the revert deletes or overwrites something Git ignores, only because a restored
    /// file needs its place; such a folder's path ends in `/`.
    pub ignored: bool,
    /// Whether the revert leaves it alone because it is inside, at or above a submodule or
    /// nested repository.
    pub kept: bool,
}

/// An agent that runs a turn in the checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRunningAgent {
    /// Whether it is this session's agent.
    pub own: bool,
    /// Its session's title, when its session file is found.
    pub title: Option<String>,
}
