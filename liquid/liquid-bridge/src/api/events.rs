#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeEventKind {
    TurnStarted,
    TurnCompleted,
    TurnAborted,
    TurnsChanged,
    RolledBack,
    ItemStarted,
    ContentDelta,
    ItemUpdated,
    ItemCompleted,
    ModelRequestCompleted,
    RequestOpened,
    RequestResolved,
    FellBehind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeEvent {
    pub kind: BridgeEventKind,
    pub sequence: i64,
    pub turn: Option<i64>,
    pub item: Option<BridgeItem>,
    pub text: Option<String>,
    pub usage: Option<BridgeUsage>,
    pub request: Option<BridgeRequest>,
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeItemKind {
    UserMessage,
    AssistantMessage,
    Reasoning,
    CommandExecution,
    FileChange,
    DynamicToolCall,
    Error,
    TaskList,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeItem {
    pub id: i64,
    pub kind: BridgeItemKind,
    pub tool: Option<String>,
    pub target: Option<String>,
    pub ended: bool,
    pub failed: bool,
    pub duration_ms: Option<i64>,
    pub steps: Vec<BridgeTaskStep>,
    pub routine: Option<String>,
    pub opening: BridgeOpening,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeOpening {
    None,
    Start,
    Handoff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTaskStep {
    pub text: String,
    pub status: BridgeStepStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeStepStatus {
    Pending,
    Running,
    Completed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BridgeUsage {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub reasoning: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRequest {
    pub id: String,
    pub turn: i64,
    pub approval: Option<BridgeApproval>,
    pub questions: Vec<BridgeQuestion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeApproval {
    pub kind: String,
    pub prompt: String,
    pub reason: Option<String>,
    pub options: Vec<BridgeApprovalOption>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeApprovalOption {
    pub decision: String,
    pub label: String,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    pub options: Vec<String>,
    pub descriptions: Vec<String>,
    pub multi_select: bool,
    pub custom: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeFileChange {
    pub path: String,
    pub shown: String,
    pub status: BridgeChangeStatus,
    pub added: i64,
    pub removed: i64,
}
