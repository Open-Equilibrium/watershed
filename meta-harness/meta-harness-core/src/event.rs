#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TurnId(pub u64);

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemId(pub u64);

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    UserMessage,
    AssistantMessage,
    Reasoning,
    CommandExecution,
    FileChange,
    DynamicToolCall,
    Error,
    TaskList,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum ItemDetail {
    None,
    Tool { name: String, target: String },
    Tasks { steps: Vec<TaskStep> },
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct TaskStep {
    pub text: String,
    pub status: StepStatus,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Pending,
    Running,
    Completed,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub reasoning: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequestId(pub String);

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PendingRequest {
    pub id: RequestId,
    pub turn: TurnId,
    pub request: Request,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Approval {
        kind: String,
        prompt: String,
        reason: Option<String>,
        options: Vec<ApprovalOption>,
    },
    Questions(Vec<Question>),
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ApprovalOption {
    pub decision: String,
    pub label: String,
    pub warning: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub id: String,
    pub header: String,
    pub question: String,
    pub options: Vec<(String, String)>,
    pub multi_select: bool,
    pub custom: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    Completed,
    Failed,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum AgentEvent {
    TurnStarted {
        turn: TurnId,
    },
    TurnCompleted {
        turn: TurnId,
        checkpoint: Option<String>,
    },
    TurnAborted {
        turn: TurnId,
        checkpoint: Option<String>,
    },
    RequestOpened {
        request: PendingRequest,
    },
    RequestResolved {
        request: RequestId,
    },
    RolledBack,
    TurnsChanged,
    ItemStarted {
        turn: TurnId,
        item: ItemId,
        kind: ItemKind,
        detail: ItemDetail,
    },
    ContentDelta {
        item: ItemId,
        delta: String,
    },
    ItemUpdated {
        item: ItemId,
        detail: ItemDetail,
    },
    ItemCompleted {
        turn: TurnId,
        item: ItemId,
        kind: ItemKind,
        text: String,
        status: ItemStatus,
        duration_ms: Option<u64>,
    },
    ModelRequestCompleted {
        turn: TurnId,
        usage: Usage,
    },
}
