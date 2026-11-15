//! Agent-neutral agent events, in T3 Code's vocabulary (ADR-0281).

/// One turn: a user prompt until the agent settles, including its retries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TurnId(pub u64);

/// One item within a session, such as a message, a reasoning block or a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemId(pub u64);

/// What an item represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    /// A prompt or steering message from the user.
    UserMessage,
    /// Text the model wrote for the user.
    AssistantMessage,
    /// The model's reasoning text.
    Reasoning,
    /// A shell command.
    CommandExecution,
    /// A change to project files.
    FileChange,
    /// Any other tool call.
    DynamicToolCall,
    /// An error T3 or the provider reported in the Chat.
    Error,
}

/// Kind-specific information about a started item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemDetail {
    /// No further information.
    None,
    /// A tool call.
    Tool {
        /// The agent's tool name.
        name: String,
        /// What the call works on, such as a command or a path, on one line.
        target: String,
    },
}

/// Tokens of one model request, as the provider reported them; T3 reports no cost (ADR-0293).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    /// Input tokens.
    pub input: u64,
    /// Output tokens, including reasoning tokens.
    pub output: u64,
    /// Input tokens read from the provider's cache.
    pub cache_read: u64,
    /// Input tokens written to the provider's cache.
    pub cache_write: u64,
    /// Reasoning tokens, already counted in `output`.
    pub reasoning: u64,
}

/// One open request from the agent that waits for the user.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequestId(pub String);

/// A request the agent waits on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRequest {
    /// The request.
    pub id: RequestId,
    /// The turn that asks.
    pub turn: TurnId,
    /// What it asks.
    pub request: Request,
}

/// What a request asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Approve an action.
    Approval {
        /// T3's request kind, such as `command`, `file_change` or `file_read`.
        kind: String,
        /// The command, path or other detail.
        prompt: String,
        /// Why the agent asks, if it said.
        reason: Option<String>,
        /// The choices, in T3's order.
        options: Vec<ApprovalOption>,
    },
    /// Answer one or more questions.
    Questions(Vec<Question>),
}

/// One choice of an approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalOption {
    /// T3's decision: `accept`, `acceptForSession`, `acceptAlways`, `decline` or `cancel`.
    pub decision: String,
    /// The label.
    pub label: String,
    /// A warning shown with it.
    pub warning: Option<String>,
}

/// One question the agent asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// The id its answer is sent under.
    pub id: String,
    /// A short header.
    pub header: String,
    /// The question, in Markdown.
    pub question: String,
    /// The options, as label and description.
    pub options: Vec<(String, String)>,
    /// Whether several options may be chosen.
    pub multi_select: bool,
    /// Whether a typed answer is allowed.
    pub custom: bool,
}

/// How an item ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    /// The item completed.
    Completed,
    /// The item failed, such as a tool call that reported an error.
    Failed,
}

/// An agent-neutral agent event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentEvent {
    /// A turn started.
    TurnStarted {
        /// The turn.
        turn: TurnId,
    },
    /// A turn ended without being aborted.
    TurnCompleted {
        /// The turn.
        turn: TurnId,
        /// The turn's checkpoint id, which names its end checkpoint; absent when it requested none.
        checkpoint: Option<String>,
    },
    /// A turn ended because it was aborted.
    TurnAborted {
        /// The turn.
        turn: TurnId,
        /// The turn's checkpoint id, which names its end checkpoint; absent when it requested none.
        checkpoint: Option<String>,
    },
    /// The agent asks the person to approve an action or answer questions.
    RequestOpened {
        /// The request.
        request: PendingRequest,
    },
    /// The person, or T3, resolved a request.
    RequestResolved {
        /// The request.
        request: RequestId,
    },
    /// Turns were reverted and left the thread; the Chat reloads from a snapshot.
    RolledBack,
    /// A turn's changed files became known, or changed.
    TurnsChanged,
    /// An item started.
    ItemStarted {
        /// The owning turn.
        turn: TurnId,
        /// The item.
        item: ItemId,
        /// What the item represents.
        kind: ItemKind,
        /// Kind-specific information.
        detail: ItemDetail,
    },
    /// Text was appended to a streaming item.
    ContentDelta {
        /// The item.
        item: ItemId,
        /// The appended text.
        delta: String,
    },
    /// An item completed with its authoritative text.
    ItemCompleted {
        /// The owning turn.
        turn: TurnId,
        /// The item.
        item: ItemId,
        /// What the item represents.
        kind: ItemKind,
        /// The complete text or tool output.
        text: String,
        /// How the item ended.
        status: ItemStatus,
        /// How long a tool ran, when the agent measured it.
        duration_ms: Option<u64>,
    },
    /// The model finished its response to a request.
    ModelRequestCompleted {
        /// The owning turn.
        turn: TurnId,
        /// Tokens and cost.
        usage: Usage,
        /// Why the response ended, such as `stop`, `toolUse`, `error` or `aborted`.
        stop_reason: String,
        /// The provider's error, if the request failed.
        error: Option<String>,
    },
}
