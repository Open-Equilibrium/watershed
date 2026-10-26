//! Events as Dart receives them. A flat event with typed parts avoids Dart code generation for
//! enums with data; each [`BridgeEventKind`] documents which parts it fills.

/// What happened; the variant names the parts of [`BridgeEvent`] it fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeEventKind {
    /// A turn started; no parts.
    TurnStarted,
    /// A turn ended; no parts.
    TurnCompleted,
    /// A turn was stopped; no parts.
    TurnAborted,
    /// `item` with kind, tool and target.
    ItemStarted,
    /// `item` id and `text` with the latest partial output.
    ItemUpdated,
    /// `item` id and `text` with the appended text.
    ContentDelta,
    /// `item` with kind, status and duration, and `text` with the final text.
    ItemCompleted,
    /// `step` with usage, stop reason and error.
    ModelRequestCompleted,
    /// `request` with its approval.
    RequestOpened,
    /// `request` id.
    RequestResolved,
    /// `request` with its question.
    UserInputRequested,
    /// `request` id.
    UserInputResolved,
    /// `display` with level and `text`.
    Notification,
    /// `display` with key and optional `text`.
    StatusChanged,
    /// `display` with key and optional lines.
    WidgetChanged,
    /// `text`.
    TitleChanged,
    /// `text`.
    InputTextSet,
    /// `text` with the reason the session stopped the agent.
    Failed,
    /// `success`, and `text` with the end of the agent's error output; always the last event.
    Exited,
    /// The messages waiting for the running agent changed.
    QueueChanged,
}

/// One session event for Dart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeEvent {
    /// What happened.
    pub kind: BridgeEventKind,
    /// The item.
    pub item: Option<BridgeItem>,
    /// The model request.
    pub step: Option<BridgeStep>,
    /// The approval request or question.
    pub request: Option<BridgeRequest>,
    /// Text whose meaning the kind defines.
    pub text: Option<String>,
    /// A notification, status or widget.
    pub display: Option<BridgeDisplay>,
    /// Whether the agent process exited successfully.
    pub success: Option<bool>,
    /// The messages waiting for the running agent.
    pub queue: Option<BridgeQueue>,
}

/// Messages waiting for a running agent, in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BridgeQueue {
    /// Steering messages, delivered before the agent's next model request.
    pub steering: Vec<String>,
    /// Follow-up messages, delivered once the agent is done.
    pub follow_up: Vec<String>,
}

/// What an item represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeItemKind {
    /// A user prompt or steering message.
    UserMessage,
    /// Text the model wrote for the user.
    AssistantMessage,
    /// The model's reasoning.
    Reasoning,
    /// A shell command.
    CommandExecution,
    /// A change to project files.
    FileChange,
    /// Any other tool call.
    DynamicToolCall,
}

/// An item's identity and, depending on the event, its tool call or outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeItem {
    /// The item id.
    pub id: i64,
    /// What it represents, when the event says.
    pub kind: Option<BridgeItemKind>,
    /// The tool name of a tool call.
    pub tool: Option<String>,
    /// What a tool call works on, such as a command or a path, on one line.
    pub target: Option<String>,
    /// Whether the item failed.
    pub failed: bool,
    /// How long a tool ran.
    pub duration_ms: Option<i64>,
    /// The lines a file change added.
    pub added: Option<i64>,
    /// The lines a file change removed.
    pub removed: Option<i64>,
}

/// A completed model request's outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeStep {
    /// Tokens and cost.
    pub usage: BridgeUsage,
    /// Why the response ended.
    pub stop_reason: String,
    /// The provider's error.
    pub error: Option<String>,
}

/// Tokens and cost of one model request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BridgeUsage {
    /// Input tokens.
    pub input: i64,
    /// Output tokens, including reasoning.
    pub output: i64,
    /// Input tokens read from cache.
    pub cache_read: i64,
    /// Input tokens written to cache.
    pub cache_write: i64,
    /// Reasoning tokens, already counted in `output`.
    pub reasoning: i64,
    /// Cost in millionths of a US dollar.
    pub cost_micro_usd: i64,
}

/// An open request and, depending on the event, what it asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRequest {
    /// The request id.
    pub id: String,
    /// The tool call awaiting approval.
    pub approval: Option<BridgeApproval>,
    /// The question awaiting an answer.
    pub question: Option<BridgeQuestion>,
}

/// What an approval request asks to allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeRequestKind {
    /// Running a shell command.
    CommandExecution,
    /// Changing project files.
    FileChange,
    /// Calling any other tool.
    Tool,
}

/// A tool call awaiting approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeApproval {
    /// What the call would do.
    pub kind: BridgeRequestKind,
    /// The tool name.
    pub tool: String,
    /// The whole call the user decides on, such as a shell command.
    pub text: String,
    /// The id of the call's item in the chat, when known.
    pub item: Option<i64>,
}

/// The user's answer to an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeDecision {
    /// Allow this call only.
    AllowOnce,
    /// Allow this tool for the rest of the session.
    AllowSession,
    /// Block this call.
    Reject,
}

/// What kind of question an agent extension asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeQuestionKind {
    /// Choose one of `options`.
    Select,
    /// Confirm or decline; `detail` explains.
    Confirm,
    /// Enter one line; `detail` is the placeholder.
    Input,
    /// Edit multi-line text; `detail` is the initial text.
    Editor,
}

/// A question from an agent extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeQuestion {
    /// What kind of answer it expects.
    pub kind: BridgeQuestionKind,
    /// The question.
    pub title: String,
    /// The explanation, placeholder or initial text, depending on the kind.
    pub detail: String,
    /// The options of a selection.
    pub options: Vec<String>,
}

/// How urgent a notification is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeLevel {
    /// Information.
    Info,
    /// A warning.
    Warning,
    /// An error.
    Error,
}

/// A notification, status entry or widget from an agent extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeDisplay {
    /// The extension's status or widget key.
    pub key: String,
    /// The notification level.
    pub level: Option<BridgeLevel>,
    /// Widget lines, or `None` when cleared.
    pub lines: Option<Vec<String>>,
}

/// How a file changed in a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeChangeStatus {
    /// The file is new.
    Added,
    /// The file changed.
    Modified,
    /// The file was removed.
    Deleted,
}

/// One file a run changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeFileChange {
    /// The path relative to the root of the Git repository the project is in.
    pub path: String,
    /// How it changed.
    pub status: BridgeChangeStatus,
    /// Added lines.
    pub added: i64,
    /// Removed lines.
    pub removed: i64,
}
