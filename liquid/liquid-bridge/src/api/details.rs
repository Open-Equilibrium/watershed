//! What the details card beside a session shows: its local servers, automations, lineage and
//! usage limits (T3 Code 0.1.9's thread details card). Times are Unix milliseconds.

/// A local server a terminal or the agent of the session started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeLocalServer {
    /// The host, such as `localhost`.
    pub host: String,
    /// The port it listens on.
    pub port: i64,
    /// Its address, which opens in the browser.
    pub url: String,
    /// The process's name, with hiding characters written out.
    pub process: Option<String>,
    /// The process id, which stopping the server needs.
    pub pid: Option<i64>,
    /// Whether a terminal started it rather than the agent.
    pub terminal: bool,
}

/// How a scheduled task's last run ended, or that it never ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeRunStatus {
    /// It never ran.
    Never,
    /// It runs now.
    Running,
    /// Its last run succeeded.
    Succeeded,
    /// Its last run failed.
    Failed,
}

/// When a scheduled task runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeScheduleKind {
    /// Every `every_ms`.
    Interval,
    /// At `time_of_day` on `weekdays`.
    FixedTime,
    /// Once, at `run_at`.
    Once,
    /// On each webhook request.
    Webhook,
}

/// A schedule with the fields of its kind; the card words it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeSchedule {
    /// The kind.
    pub kind: BridgeScheduleKind,
    /// The interval, for an interval schedule.
    pub every_ms: Option<i64>,
    /// The time of day, `HH:MM`, for a fixed-time schedule.
    pub time_of_day: Option<String>,
    /// The weekdays, 0 for Sunday; empty for every day.
    pub weekdays: Vec<i64>,
    /// The ISO 8601 time with its offset, for a one-time schedule.
    pub run_at: Option<String>,
}

/// A scheduled task bound to the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeAutomation {
    /// T3's task id.
    pub id: String,
    /// Its title, with hiding characters written out.
    pub title: String,
    /// When it runs.
    pub schedule: BridgeSchedule,
    /// Whether it runs.
    pub enabled: bool,
    /// When it runs next.
    pub next_run_ms: Option<i64>,
    /// How its last run ended.
    pub last_run: BridgeRunStatus,
    /// Why policy keeps it from running, with hiding characters written out.
    pub blocked_by_policy: Option<String>,
}

/// How a lineage row's thread relates to the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeLineageRelation {
    /// Context transferred between the two.
    ContextTransfer,
    /// A subagent the session started.
    Subagent,
    /// The thread that started this session as a subagent.
    ParentAgent,
    /// A fork the session started.
    Fork,
    /// The session's fork source.
    ParentThread,
    /// A side chat of the session.
    SideChat,
}

/// The section of the card a lineage row sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeLineageGroup {
    /// Listed at once.
    Related,
    /// Subagents that still run or wait.
    Active,
    /// Subagents that ended, under "Previous agents".
    Previous,
    /// Side chats.
    SideChats,
}

/// One thread related to the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeLineageRow {
    /// The thread's id; opening the row opens it.
    pub thread_id: String,
    /// How it relates.
    pub relation: BridgeLineageRelation,
    /// Where it sits.
    pub group: BridgeLineageGroup,
    /// Its title, with hiding characters written out.
    pub title: String,
    /// Its status, as T3 reports it.
    pub status: Option<String>,
    /// The status in T3's words.
    pub status_label: String,
    /// Whether it failed.
    pub failed: bool,
    /// When a subagent started; set only while its elapsed time is known.
    pub started_ms: Option<i64>,
    /// When a finished subagent ended; `None` while it runs.
    pub ended_ms: Option<i64>,
    /// Whether it is the thread this session merges back into.
    pub merge_target: bool,
    /// The subagent a stop would end.
    pub stop_subagent: Option<String>,
}

/// The lineage the card shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeLineage {
    /// The related threads, in T3's order.
    pub rows: Vec<BridgeLineageRow>,
    /// Subagents that run.
    pub running: i64,
    /// Side chats that run.
    pub side_chats_running: i64,
    /// Whether the provider session can be disconnected.
    pub can_disconnect: bool,
    /// The merge target's title, with hiding characters written out.
    pub merge_target_title: Option<String>,
    /// Whether the session has a finished run to merge back.
    pub merge_ready: bool,
    /// Whether "Merge back" can run now.
    pub can_merge: bool,
}

/// Where a usage window stands against the time it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgePace {
    /// Used faster than time passed.
    Ahead,
    /// Used about as fast as time passed.
    On,
    /// Used slower than time passed.
    Under,
}

/// One window of the provider's usage limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeUsageWindow {
    /// Its label, with hiding characters written out.
    pub label: String,
    /// The percentage left, from 0 to 100.
    pub remaining_percent: i64,
    /// When it resets, when that is still ahead.
    pub resets_at_ms: Option<i64>,
    /// Its pace, when known.
    pub pace: Option<BridgePace>,
}

/// The provider's usage limits: its windows, or why there are none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeUsageLimits {
    /// Why the windows are missing, with hiding characters written out.
    pub notice: Option<String>,
    /// The windows, in T3's order.
    pub windows: Vec<BridgeUsageWindow>,
}
