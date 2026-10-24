//! Agent-neutral runtime events, using T3 Code's provider runtime vocabulary (ADR-0195).

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
        /// The call's arguments as JSON text.
        arguments: String,
    },
}

/// One model request within a turn: the agent sends its context and receives one response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StepId(pub u64);

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

/// One named part of the agent's system prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextSection {
    /// The section name.
    pub name: String,
    /// Its length in characters, or `None` when the section was removed.
    pub chars: Option<usize>,
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
    /// An extension's custom entry, with its type.
    Custom(String),
    /// Any other entry, with the agent's entry type.
    Other(String),
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    /// Running a shell command.
    CommandExecutionApproval,
    /// Changing project files.
    FileChangeApproval,
    /// Calling any other tool.
    ToolApproval,
}

/// The user's answer to an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Allow this call only.
    AllowOnce,
    /// Allow this tool for the rest of the session.
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

/// An agent-neutral runtime event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEvent {
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
    },
    /// The agent changed what it sends to the model: prompt sections or tools.
    ContextChanged {
        /// The owning turn.
        turn: TurnId,
        /// Changed prompt sections, by name.
        sections: Vec<ContextSection>,
        /// Characters of instructions appended to the prompt.
        appended_chars: usize,
        /// Tools added, by name.
        tools_added: Vec<String>,
        /// Tools removed, by name.
        tools_removed: Vec<String>,
    },
    /// The agent sent a request to the model.
    ModelRequestStarted {
        /// The owning turn.
        turn: TurnId,
        /// The request.
        step: StepId,
    },
    /// The model finished its response to a request.
    ModelRequestCompleted {
        /// The owning turn.
        turn: TurnId,
        /// The request.
        step: StepId,
        /// The model that answered.
        model: String,
        /// Tokens and cost.
        usage: Usage,
        /// Why the response ended, such as `stop`, `toolUse`, `error` or `aborted`.
        stop_reason: String,
        /// The provider's error, if the request failed.
        error: Option<String>,
        /// Milliseconds until the first streamed content, if any arrived.
        first_token_ms: Option<u64>,
        /// Milliseconds from request to completed response.
        duration_ms: u64,
    },
    /// A tool call waits for the user's approval.
    RequestOpened {
        /// The request.
        request: RequestId,
        /// What the call would do.
        kind: RequestKind,
        /// The agent's tool name.
        tool: String,
        /// The call's arguments as JSON text.
        arguments: String,
        /// Whether the tool declares that it only reads.
        read_only: bool,
    },
    /// The user answered an approval request.
    RequestResolved {
        /// The request.
        request: RequestId,
        /// The answer.
        decision: Decision,
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
    /// An agent extension set the session title.
    TitleChanged {
        /// The title.
        title: String,
    },
    /// An agent extension replaced the text in the input area.
    InputTextSet {
        /// The text.
        text: String,
    },
}
