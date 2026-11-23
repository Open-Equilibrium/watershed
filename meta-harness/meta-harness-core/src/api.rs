use serde::{Deserialize, Serialize};

use crate::config::Change;
use crate::host_move::{MoveFinish, MovePackage};
use crate::hosts::{Access, ClientKind};
use crate::{GitActionRequest, ModelChoice, Permissions, StartThread, ThreadSummary};

pub const CLIENT_HEADER: &str = "meta-harness-client";

pub const API_HEADER: &str = "meta-harness-api";

pub const PAIRING_HEADER: &str = "meta-harness-pairing";

pub const TUNNEL_HEADER: &str = "meta-harness-tunnel";

pub const API_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Pairing {
    pub name: String,
    pub token: String,
    #[serde(default)]
    pub access: Access,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Call {
    Host(HostCall),
    Remote(RemoteCall),
    Invite {
        kind: ClientKind,
        access: Access,
    },
    Hosts(HostsCall),
    Project {
        root: String,
        call: ProjectCall,
    },
    Thread {
        root: String,
        id: String,
        call: ThreadCall,
    },
    Git {
        checkout: Checkout,
        call: GitCall,
    },
    Config(ConfigCall),
    Agents(AgentsCall),
}

pub(crate) const MAX_MESSAGE_BYTES: usize = 176 * 1024 * 1024;

pub(crate) fn socket_config() -> tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
    tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES))
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentsCall {
    List,
    Create(crate::agents::NewAgent),
    Remove(String),
    Routines(String),
    AddRoutine {
        name: String,
        routine: crate::agents::NewRoutine,
    },
    RemoveRoutine {
        name: String,
        id: String,
    },
    Update {
        name: String,
        rename: Option<String>,
        model: Option<crate::ModelChoice>,
        permissions: Option<crate::Permissions>,
    },
    Instructions(String),
    SetInstructions {
        name: String,
        text: String,
    },
    Changes(String),
    Undo {
        name: String,
        change: i64,
    },
    Apply {
        name: String,
        change: i64,
    },
    Mention {
        name: String,
        root: String,
        thread: String,
        text: String,
    },
    Mentions(String),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HostCall {
    Status,
    Providers,
    EnvironmentName,
    EnvironmentMachine,
    EnvironmentId,
    WriterModel,
    SetWriterModel(ModelChoice),
    Projects,
    CloneProject { remote_url: String, path: String },
    AddProject { root: String },
    MoveStatus { transfer: String },
    MoveAbort { transfer: String },
    Preflight { root: String, branch: String },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RemoteCall {
    On { port: u16 },
    Off,
    Status,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HostsCall {
    T3Pairing,
    Revoke(String),
    Add {
        address: String,
        code: String,
        access: Option<Access>,
    },
    List,
    Remove(String),
    Agents(bool),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProjectCall {
    Threads,
    Start(StartThread),
    MoveIn(MovePackage),
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
    Visit,
    UsageLimits,
    Lineage,
    MergeBack,
    Disconnect,
    StopSubagent(String),
    MoveCheck,
    MoveOut {
        transfer: String,
        target_address: String,
        target_name: String,
        target_environment: String,
        commit: bool,
        message: Option<String>,
    },
    MoveFinish {
        transfer: String,
        finish: MoveFinish,
    },
    MoveRecord,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Checkout {
    Project(String),
    Thread { root: String, id: String },
}

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

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConfigCall {
    Current,
    Propose {
        summary: String,
        changes: Vec<Change>,
    },
    Revert(u64),
    Pending,
    Approve(u64),
    Reject(u64),
    Log,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Stream {
    Subscribe {
        root: String,
        id: String,
    },
    Servers {
        root: String,
        id: String,
    },
    Automations {
        root: String,
        id: String,
    },
    LineageChanges {
        root: String,
        id: String,
    },
    GitWatch(Checkout),
    GitRun {
        checkout: Checkout,
        request: GitActionRequest,
    },
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub(crate) enum Request {
    Call { id: u64, call: Call },
    Stream { id: u64, stream: Stream },
    Cancel { cancel: u64 },
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub(crate) enum Reply {
    Ok { id: u64, ok: serde_json::Value },
    Error { id: u64, error: String },
    Item { id: u64, item: serde_json::Value },
    End { id: u64, end: () },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub t3_version: String,
    pub pid: u32,
    pub revision: u64,
    #[serde(default = "reconciled_when_absent")]
    pub reconciled: bool,
}

fn reconciled_when_absent() -> bool {
    true
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RemoteStatus {
    pub port: u16,
    pub addresses: Vec<String>,
    pub waiting: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::Status;

    #[test]
    fn a_status_of_an_older_service_reads_as_reconciled() {
        let status: Status =
            serde_json::from_str(r#"{"t3_version":"0.1.9","pid":7,"revision":3}"#).unwrap();
        assert!(status.reconciled);
    }
}
