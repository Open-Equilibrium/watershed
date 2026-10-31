//! Agent-neutral agent events, using T3 Code's provider runtime vocabulary (ADR-0195).

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

/// Tokens and cost of one model request, as the provider reported them.
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
    /// Cost in millionths of a US dollar.
    pub cost_micro_usd: u64,
}

/// The lines a file change added and removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangedLines {
    /// Added lines.
    pub added: u32,
    /// Removed lines.
    pub removed: u32,
}

/// What a session entry holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    /// A user message.
    User,
    /// A model response.
    Assistant,
    /// A tool result.
    ToolResult,
    /// A system message that changed prompt sections or tools.
    System,
    /// The user's decision on a tool call, recorded by Watershed's pi extension.
    Approval,
    /// An extension's custom entry.
    Custom,
    /// Any other entry.
    Other,
}

/// One entry of a session's branch tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    /// The agent's entry id.
    pub id: String,
    /// The parent entry, or `None` for a root.
    pub parent: Option<String>,
    /// What the entry holds.
    pub kind: EntryKind,
    /// A one-line preview of its text or tool.
    pub preview: String,
    /// The branch label set on this entry.
    pub label: Option<String>,
    /// Unix milliseconds when the entry was stored, if known.
    pub at_ms: Option<u64>,
    /// Whether a tool call or model response failed.
    pub failed: bool,
}

/// A session's entries as a tree, in append order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionTree {
    /// The entries; every parent precedes its children.
    pub nodes: Vec<TreeNode>,
    /// The entry the next message continues from.
    pub leaf: Option<String>,
}

/// One open request from the agent that waits for the user.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequestId(pub String);

/// What an approval request asks to allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestKind {
    /// Running a shell command.
    CommandExecutionApproval,
    /// Changing project files.
    FileChangeApproval,
    /// Calling any other tool.
    ToolApproval,
}

/// What a file change would do, as its approval shows it: the file and each replacement, with
/// the characters that could hide or reorder them written out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditPreview {
    /// The file, on one line, as the agent gave it.
    pub path: String,
    /// The file pi changes, on one line, when the path does not say so plainly: Watershed's
    /// pi extension resolves it as pi does.
    pub writes_to: Option<String>,
    /// The replacements in order; a write is one change that adds its whole content.
    pub changes: Vec<EditChange>,
    /// Whether the change replaces whatever the file holds, as a write does.
    pub whole_file: bool,
}

/// One replacement in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditChange {
    /// The text it removes.
    pub removed: String,
    /// The text it adds.
    pub added: String,
}

/// The user's answer to an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Allow this call only.
    AllowOnce,
    /// Allow this shell command, or this tool, for the rest of the session.
    AllowSession,
    /// Block this call.
    Reject,
}

/// A question an agent extension asks the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserInput {
    /// Choose one of several options.
    Select {
        /// The question.
        title: String,
        /// The options in display order.
        options: Vec<String>,
    },
    /// Confirm or decline.
    Confirm {
        /// The question.
        title: String,
        /// Further explanation.
        message: String,
    },
    /// Enter one line of text.
    Input {
        /// The question.
        title: String,
        /// Placeholder text.
        placeholder: String,
    },
    /// Edit multi-line text.
    Editor {
        /// The question.
        title: String,
        /// The initial text.
        prefill: String,
    },
}

/// The user's reply to a [`UserInput`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The chosen option or entered text.
    Value(String),
    /// The reply to a confirmation.
    Confirmed(bool),
    /// The user dismissed the question.
    Cancelled,
}

/// How urgent a notification is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationLevel {
    /// Information.
    Info,
    /// A warning.
    Warning,
    /// An error.
    Error,
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
    },
    /// A turn ended because it was aborted.
    TurnAborted {
        /// The turn.
        turn: TurnId,
    },
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
    /// A running item reported its latest partial output.
    ItemUpdated {
        /// The item.
        item: ItemId,
        /// The latest partial output text.
        output: String,
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
        /// The lines a file change added and removed.
        lines: Option<ChangedLines>,
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
    /// A tool call waits for the user's approval.
    RequestOpened {
        /// The request.
        request: RequestId,
        /// What the call would do.
        kind: RequestKind,
        /// The agent's tool name, with hiding characters written out as in `text`.
        tool: String,
        /// The call's arguments as JSON text.
        arguments: String,
        /// The whole call the user decides on, such as a shell command.
        text: String,
        /// Whether the tool is one of the agent's own reading tools (ADR-0208).
        read_only: bool,
        /// The session the call runs in, when the agent says.
        session: Option<String>,
        /// The call's item, when the agent says which call it asks about.
        item: Option<ItemId>,
        /// What a file change does, when pi's own `edit` or `write` asks.
        edit: Option<EditPreview>,
    },
    /// An approval request ended, answered or withdrawn.
    RequestResolved {
        /// The request.
        request: RequestId,
    },
    /// An agent extension asks the user a question.
    UserInputRequested {
        /// The request.
        request: RequestId,
        /// The question.
        input: UserInput,
    },
    /// The user answered a question.
    UserInputResolved {
        /// The request.
        request: RequestId,
    },
    /// An agent extension shows a notification.
    Notification {
        /// How urgent it is.
        level: NotificationLevel,
        /// The text.
        message: String,
    },
    /// An agent extension set or cleared a status entry.
    StatusChanged {
        /// The extension's status key.
        key: String,
        /// The text, or `None` when cleared.
        text: Option<String>,
    },
    /// An agent extension set or cleared a widget of text lines.
    WidgetChanged {
        /// The extension's widget key.
        key: String,
        /// The lines, or `None` when cleared.
        lines: Option<Vec<String>>,
    },
    /// An agent extension replaced the text in the input area.
    InputTextSet {
        /// The text.
        text: String,
    },
    /// The messages waiting for a running agent changed.
    QueueChanged(Queue),
}

/// Messages waiting for a running agent, in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Queue {
    /// Steering messages, delivered before the agent's next model request.
    pub steering: Vec<String>,
    /// Follow-up messages, delivered once the agent is done.
    pub follow_up: Vec<String>,
}

/// How a message reaches an agent that is already running; an idle agent starts with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Before the agent's next model request.
    Steer,
    /// Once the agent is done.
    FollowUp,
}
