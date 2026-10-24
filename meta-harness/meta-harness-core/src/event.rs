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
        /// The complete text.
        text: String,
    },
}
