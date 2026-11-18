#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeLocalServer {
    pub host: String,
    pub port: i64,
    pub url: String,
    pub process: Option<String>,
    pub pid: Option<i64>,
    pub terminal: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeRunStatus {
    Never,
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeScheduleKind {
    Interval,
    FixedTime,
    Once,
    Webhook,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeSchedule {
    pub kind: BridgeScheduleKind,
    pub every_ms: Option<i64>,
    pub time_of_day: Option<String>,
    pub weekdays: Vec<i64>,
    pub run_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeAutomation {
    pub id: String,
    pub title: String,
    pub schedule: BridgeSchedule,
    pub enabled: bool,
    pub next_run_ms: Option<i64>,
    pub last_run: BridgeRunStatus,
    pub blocked_by_policy: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeLineageRelation {
    ContextTransfer,
    Subagent,
    ParentAgent,
    Fork,
    ParentThread,
    SideChat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeLineageGroup {
    Related,
    Active,
    Previous,
    SideChats,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeLineageRow {
    pub thread_id: String,
    pub relation: BridgeLineageRelation,
    pub group: BridgeLineageGroup,
    pub title: String,
    pub status: Option<String>,
    pub status_label: String,
    pub failed: bool,
    pub started_ms: Option<i64>,
    pub ended_ms: Option<i64>,
    pub merge_target: bool,
    pub stop_subagent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeLineage {
    pub rows: Vec<BridgeLineageRow>,
    pub running: i64,
    pub side_chats_running: i64,
    pub can_disconnect: bool,
    pub merge_target_title: Option<String>,
    pub merge_ready: bool,
    pub can_merge: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgePace {
    Ahead,
    On,
    Under,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeUsageWindow {
    pub label: String,
    pub remaining_percent: i64,
    pub resets_at_ms: Option<i64>,
    pub pace: Option<BridgePace>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeUsageLimits {
    pub notice: Option<String>,
    pub windows: Vec<BridgeUsageWindow>,
}
