//! What an agent session streams to Liquid.

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeEventKind {
    /// A turn started.
    TurnStarted,
    /// A turn ended normally.
    TurnCompleted,
    /// A turn was stopped or failed.
    TurnAborted,
    /// A turn was queued, removed from the queue, or its changed files became known.
    TurnsChanged,
    /// Turns were reverted; Liquid reloads the Chat from a snapshot.
    RolledBack,
    /// An item started.
    ItemStarted,
    /// Text was appended to an item.
    ContentDelta,
    /// An item ended with its whole text.
    ItemCompleted,
    /// A model request finished with its usage.
    ModelRequestCompleted,
    /// The agent asks the person something.
    RequestOpened,
    /// A request was answered or withdrawn.
    RequestResolved,
    /// Liquid fell behind the stream; it reloads the Chat from a snapshot and ignores the events
    /// up to its sequence (ADR-0290).
    FellBehind,
}

/// One event, in the thread's stream order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeEvent {
    /// What happened.
    pub kind: BridgeEventKind,
    /// T3's sequence number; a snapshot at this sequence or later already contains the event.
    pub sequence: i64,
    /// The turn, for turn and item events.
    pub turn: Option<i64>,
    /// The item, for item events.
    pub item: Option<BridgeItem>,
    /// Appended or complete text.
    pub text: Option<String>,
    /// A finished model request's usage.
    pub usage: Option<BridgeUsage>,
    /// An opened request.
    pub request: Option<BridgeRequest>,
    /// A resolved request's id.
    pub request_id: Option<String>,
}

/// What an item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeItemKind {
    /// The person's prompt.
    UserMessage,
    /// The agent's answer.
    AssistantMessage,
    /// The model's reasoning.
    Reasoning,
    /// A shell command.
    CommandExecution,
    /// A file edit.
    FileChange,
    /// Another tool call.
    DynamicToolCall,
}

/// One Chat item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeItem {
    /// The item's id within the session.
    pub id: i64,
    /// What it is.
    pub kind: BridgeItemKind,
    /// The tool, for a tool call.
    pub tool: Option<String>,
    /// What the tool works on, on one line.
    pub target: Option<String>,
    /// Whether it ended, for a finished item.
    pub ended: bool,
    /// Whether it failed.
    pub failed: bool,
    /// How long a tool ran.
    pub duration_ms: Option<i64>,
}

/// Tokens of a model request, or of a model over a session. T3 reports no cost (ADR-0293).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BridgeUsage {
    /// Input tokens, without cache reads and writes.
    pub input: i64,
    /// Output tokens, including reasoning.
    pub output: i64,
    /// Input tokens read from the cache.
    pub cache_read: i64,
    /// Input tokens written to the cache.
    pub cache_write: i64,
    /// Reasoning tokens, already in `output`.
    pub reasoning: i64,
}

/// A request the agent waits on: an approval or questions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRequest {
    /// The request.
    pub id: String,
    /// The turn that asks.
    pub turn: i64,
    /// An approval, or `None` for questions.
    pub approval: Option<BridgeApproval>,
    /// The questions, empty for an approval.
    pub questions: Vec<BridgeQuestion>,
}

/// An action the agent asks to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeApproval {
    /// T3's kind, such as `command`, `file_change` or `file_read`.
    pub kind: String,
    /// The command, path or other detail.
    pub prompt: String,
    /// Why the agent asks.
    pub reason: Option<String>,
    /// The choices, in T3's order.
    pub options: Vec<BridgeApprovalOption>,
}

/// One choice of an approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeApprovalOption {
    /// The decision sent back.
    pub decision: String,
    /// The label.
    pub label: String,
    /// A warning shown with it.
    pub warning: Option<String>,
}

/// One question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeQuestion {
    /// The id its answer is sent under.
    pub id: String,
    /// A short header.
    pub header: String,
    /// The question, in Markdown.
    pub question: String,
    /// The option labels.
    pub options: Vec<String>,
    /// The option descriptions, matching `options`.
    pub descriptions: Vec<String>,
    /// Whether several options may be chosen.
    pub multi_select: bool,
    /// Whether a typed answer is allowed.
    pub custom: bool,
}

/// What a changed file's change is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeChangeStatus {
    /// Added.
    Added,
    /// Changed.
    Modified,
    /// Deleted.
    Deleted,
    /// Renamed.
    Renamed,
}

/// One changed file and its line counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeFileChange {
    /// The path, relative to the checkout.
    pub path: String,
    /// The path as Liquid shows it, with hiding characters written out.
    pub shown: String,
    /// What changed.
    pub status: BridgeChangeStatus,
    /// Added lines.
    pub added: i64,
    /// Removed lines.
    pub removed: i64,
}
