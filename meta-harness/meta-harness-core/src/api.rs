//! Meta-Harness's API: the messages its service and clients exchange as JSON text frames over one
//! WebSocket (ADR-0185, ADR-0297). A client sends `{"id", "call"}` for one answer, `{"id",
//! "stream"}` for an answer followed by items, and `{"cancel": id}` to end a stream. The service
//! answers `{"id", "ok"}` or `{"id", "error"}`, then sends a stream's `{"id", "item"}` frames and
//! finally `{"id", "end": null}`.
//!
//! Every thread call names the project and the thread, so a client that reconnects goes on without
//! opening its threads again.

use serde::{Deserialize, Serialize};

use crate::config::Change;
use crate::{GitActionRequest, ModelChoice, Permissions, StartThread, ThreadSummary};

/// The header a client names itself with; the configuration log records it (ADR-0298).
pub const CLIENT_HEADER: &str = "meta-harness-client";

/// The header a client names its API version with. A service of another version, or a client that
/// names none, is refused at the handshake, since it would misread the frames.
pub const API_HEADER: &str = "meta-harness-api";

/// The API version this build speaks: the calls, streams and frames below.
pub const API_VERSION: u32 = 1;

/// A call with one answer.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Call {
    /// The host's T3 Code server.
    Host(HostCall),
    /// A project, by its folder.
    Project {
        /// The project folder.
        root: String,
        /// What to do.
        call: ProjectCall,
    },
    /// A thread of a project.
    Thread {
        /// The project folder.
        root: String,
        /// T3's thread id.
        id: String,
        /// What to do.
        call: ThreadCall,
    },
    /// Git for a checkout.
    Git {
        /// Whose checkout.
        checkout: Checkout,
        /// What to do.
        call: GitCall,
    },
    /// The central configuration (ADR-0298).
    Config(ConfigCall),
}

/// A call to the host.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HostCall {
    Status,
    Providers,
    EnvironmentName,
    WriterModel,
    SetWriterModel(ModelChoice),
}

/// A call to a project.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProjectCall {
    Threads,
    /// Starts a thread and answers its id.
    Start(StartThread),
    /// Checks the thread exists and answers nothing.
    Open(String),
    OnlyThreadInWorktree(ThreadSummary),
    Delete {
        thread: ThreadSummary,
        worktree: bool,
    },
    Archive {
        thread: String,
        archived: bool,
    },
}

/// A call to a thread.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ThreadCall {
    Title,
    Worktree,
    Ended,
    Snapshot,
    Turns,
    Statistics,
    Context,
    Model,
    Record,
    Send {
        text: String,
        steer: bool,
    },
    Stop,
    Diff {
        from: u64,
        to: u64,
        ignore_whitespace: bool,
    },
    Revert {
        turn: u64,
        restore_files: bool,
    },
    Fork(u64),
    SetPermissions(Permissions),
    SetModel(ModelChoice),
    SendQueuedNow(u64),
    EditQueued {
        turn: u64,
        text: String,
    },
    CancelQueued(u64),
    Respond {
        request: String,
        decision: String,
    },
    Answer {
        request: String,
        answers: Vec<(String, Vec<String>)>,
    },
    Dismiss(String),
    SetBranch {
        branch: String,
        worktree: Option<String>,
    },
    StopServer {
        pid: u32,
        port: u16,
    },
    SetAutomationEnabled {
        id: String,
        enabled: bool,
    },
    RunAutomationNow(String),
    UsageLimits,
    Lineage,
    MergeBack,
    Disconnect,
    StopSubagent(String),
}

/// Whose checkout a Git call works in.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Checkout {
    /// The project folder.
    Project(String),
    /// The thread's worktree, else its project folder, as it is when the call arrives.
    Thread { root: String, id: String },
}

/// A Git call.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GitCall {
    Status,
    Refs {
        query: String,
        cursor: u64,
        limit: u64,
    },
    Switch(String),
    CreateBranch(String),
    Init,
    Pull,
    BranchDiff(bool),
    UncommittedDiff(bool),
    ResolvePullRequest(String),
    CheckOutPullRequest {
        reference: String,
        worktree: bool,
    },
    Publish {
        provider: String,
        repository: String,
        public: bool,
    },
    PullRequest(u64),
    RunPullRequestAction {
        number: u64,
        action: String,
        merge_method: Option<String>,
    },
    PullRequestWorktree(String),
}

/// A configuration call (ADR-0298).
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConfigCall {
    /// The current revision and its configuration blocks.
    Current,
    /// Proposes changes to the current revision and answers the proposal's number.
    Propose {
        summary: String,
        changes: Vec<Change>,
    },
    /// Proposes going back to the blocks of an earlier revision.
    Revert(u64),
    /// The proposals that wait for a decision.
    Pending,
    Approve(u64),
    Reject(u64),
    /// The whole log, oldest first.
    Log,
}

/// A call answered by a first value and then items.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Stream {
    /// The thread's snapshot, then its events (ADR-0290).
    Subscribe { root: String, id: String },
    /// The thread's local servers, then each change.
    Servers { root: String, id: String },
    /// The thread's automations, then each change.
    Automations { root: String, id: String },
    /// Nothing, then an item whenever the thread's lineage may have changed.
    LineageChanges { root: String, id: String },
    /// Nothing, then each Git status.
    GitWatch(Checkout),
    /// Nothing, then the stacked action's progress.
    GitRun {
        checkout: Checkout,
        request: GitActionRequest,
    },
}

/// A frame a client sends.
#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub(crate) enum Request {
    Call { id: u64, call: Call },
    Stream { id: u64, stream: Stream },
    Cancel { cancel: u64 },
}

/// A frame the service sends.
#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub(crate) enum Reply {
    Ok { id: u64, ok: serde_json::Value },
    Error { id: u64, error: String },
    Item { id: u64, item: serde_json::Value },
    End { id: u64, end: () },
}

/// What `meta status` shows of a running service.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// The T3 Code release the service runs.
    pub t3_version: String,
    /// The service's process id.
    pub pid: u32,
    /// The current configuration revision.
    pub revision: u64,
}
