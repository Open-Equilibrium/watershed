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
}
