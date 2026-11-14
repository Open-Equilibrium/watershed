//! A session's stored history: its branch tree, its chat and its turns.

use crate::api::events::{BridgeItemKind, BridgeUsage};

/// The session's branch tree and what its stored responses used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeHistory {
    /// Every entry; parents precede their children.
    pub tree: Vec<BridgeTreeNode>,
    /// The entry the next prompt continues from.
    pub leaf: Option<String>,
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

/// A turn of the current branch, as the Diff panel lists it (ADR-0234).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTurn {
    /// The turn's checkpoint id.
    pub id: String,
    /// The entry that recorded its start checkpoint; the chat places its changes after the
    /// turn's entries.
    pub start_entry: String,
    /// The entry that recorded its end checkpoint, once it ended.
    pub end_entry: Option<String>,
    /// The turn of its first prompt on the branch, counted from 1; 0 before a prompt was stored.
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
    /// Opaque identities of the state the plan was read in; return unchanged to revert, never
    /// display these strings (ADR-0256).
    pub confirmation: BridgeRevertConfirmation,
}

/// Exact identities returned unchanged by the revert dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRevertConfirmation {
    pub target: String,
    pub checkout: String,
    pub branch: Option<String>,
    pub kept: Vec<String>,
    pub ignored: Vec<String>,
    pub ignored_state: String,
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

/// What a session's stored entries count to, on every branch (ADR-0279).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeStatistics {
    /// What each model used over the whole session, sorted by cost and then tokens.
    pub models: Vec<BridgeModelUsage>,
    /// The calls of each tool over the whole session, largest total first.
    pub tools: Vec<BridgeToolCalls>,
    /// The turns, oldest first.
    pub turns: Vec<BridgeTurnStatistics>,
    /// When the session's first entry was stored, in Unix milliseconds.
    pub started_ms: Option<i64>,
    /// When its last entry was stored.
    pub ended_ms: Option<i64>,
}

/// What each kind of a model's tokens cost, in millionths of a US dollar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BridgeCostByKind {
    /// Input tokens.
    pub input: i64,
    /// Output tokens.
    pub output: i64,
    /// Input tokens read from the provider's cache.
    pub cache_read: i64,
    /// Input tokens written to the provider's cache.
    pub cache_write: i64,
}

/// What one model used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeModelUsage {
    /// The model as `provider/id`; empty for usage no model produced, such as a compaction summary.
    pub model: String,
    /// Tokens and cost.
    pub usage: BridgeUsage,
    /// What each kind of its tokens cost.
    pub cost_by_kind: BridgeCostByKind,
    /// Responses that reported usage.
    pub responses: i64,
    /// Responses that reported tokens but no cost.
    pub unpriced: i64,
}

/// The calls of one tool, by outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeToolCalls {
    /// The tool's name as Watershed shows it.
    pub name: String,
    /// Calls whose result succeeded.
    pub succeeded: i64,
    /// Calls whose result failed.
    pub failed: i64,
    /// Calls without a result, whose turn ended or was followed by a later turn.
    pub cancelled: i64,
    /// Calls without a result in the session's last turn, which may still be running.
    pub open: i64,
    /// Time its calls took; `None` when none of its calls has a timing entry.
    pub duration_ms: Option<i64>,
}

/// One turn and what it counted to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTurnStatistics {
    /// The entry of its first prompt, or of its start checkpoint when no prompt was stored yet.
    pub entry: String,
    /// The first line of its first prompt.
    pub prompt: String,
    /// When its first entry was stored, in Unix milliseconds.
    pub started_ms: Option<i64>,
    /// When its last entry was stored.
    pub ended_ms: Option<i64>,
    /// What each model used, sorted as the session's models are.
    pub models: Vec<BridgeModelUsage>,
    /// Time its requests took; `None` when the turn has no timing entries.
    pub model_ms: Option<i64>,
    /// Time its tools took; `None` when the turn has no timing entries.
    pub tool_ms: Option<i64>,
    /// Tool calls its responses made.
    pub tool_calls: i64,
    /// Tool calls whose result failed.
    pub failed: i64,
}

/// The context window the next request fills, and when pi compacts it (ADR-0279).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeContext {
    /// Tokens the window holds now, when pi knows them; `None` right after a compaction.
    pub tokens: Option<i64>,
    /// Tokens the model's context window holds.
    pub window: i64,
    /// Tokens at which pi starts compacting; `None` when auto compaction is off or pi's settings
    /// cannot be read.
    pub compacts_at: Option<i64>,
}
