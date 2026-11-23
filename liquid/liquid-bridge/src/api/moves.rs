use meta_harness_core::remote::{Host, Thread, connection_lost};
use meta_harness_core::{
    Arrival, Change, Difference, Direction, Harness, Landing, MoveCheck, MoveFinish, MovePackage,
    MovePhase, Moving, diff_blocks,
};

use super::session::{AgentHost, AgentProject, AgentSession, run};
use crate::api::thread::BridgeTranscriptItem;
use crate::convert;

const UNKNOWN_CALL: &str = "Meta-Harness does not understand this request";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeMoveCheck {
    pub title: String,
    pub movable: bool,
    pub harness: Option<String>,
    pub model: Option<String>,
    pub running: bool,
    pub uncommitted_files: i64,
    pub branch: Option<String>,
    pub remote_url: Option<String>,
    pub blocked: Option<String>,
    pub moved: bool,
    pub moving: Option<BridgeMoving>,
    pub agent: Option<BridgeAgentMove>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeAgentMove {
    pub name: String,
    pub memory_bytes: i64,
    pub routines: i64,
    pub earlier: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeMovePhase {
    Preparing,
    Handed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeMoving {
    pub transfer: String,
    pub phase: BridgeMovePhase,
    pub target_address: String,
    pub target_name: String,
    pub target_environment: String,
    pub since_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeArrivalKind {
    Landed,
    Starting,
    Refused,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeArrival {
    pub kind: BridgeArrivalKind,
    pub thread: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeHostProject {
    pub root: String,
    pub title: String,
    pub remote_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeMoveRecord {
    pub from_host: String,
    pub to_host: String,
    pub at_ms: i64,
    pub transcript: Vec<BridgeTranscriptItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeConfigKind {
    Instruction,
    Skill,
    McpServer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeDirection {
    Add,
    Replace,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeConfigChange {
    pub kind: BridgeConfigKind,
    pub name: String,
    pub direction: BridgeDirection,
}

#[flutter_rust_bridge::frb(opaque)]
pub struct BridgeConfigChanges {
    differences: Vec<Difference>,
}

impl BridgeConfigChanges {
    pub fn changes(&self) -> Vec<BridgeConfigChange> {
        self.differences.iter().map(config_change).collect()
    }
}

#[flutter_rust_bridge::frb(opaque)]
pub struct BridgeMovePackage {
    pub(crate) package: MovePackage,
}

impl BridgeMovePackage {
    pub fn transfer(&self) -> String {
        self.package.transfer.clone()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeLandingKind {
    NewWorktree,
    Continue,
    Attach,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeLanding {
    pub kind: BridgeLandingKind,
    pub path: Option<String>,
    pub reason: Option<String>,
}

pub(crate) fn landing_of(landing: Landing) -> BridgeLanding {
    let (kind, path, reason) = match landing {
        Landing::NewWorktree => (BridgeLandingKind::NewWorktree, None, None),
        Landing::Continue { path } => (BridgeLandingKind::Continue, Some(path), None),
        Landing::Attach => (BridgeLandingKind::Attach, None, None),
        Landing::Blocked { reason } => (BridgeLandingKind::Blocked, None, Some(reason)),
    };
    BridgeLanding { kind, path, reason }
}

pub struct BridgeMoveIn {
    pub thread: Option<String>,
    pub lost: bool,
    pub message: String,
}

pub(crate) fn move_in_failure(message: String) -> BridgeMoveIn {
    BridgeMoveIn {
        thread: None,
        lost: connection_lost(&message),
        message,
    }
}

impl AgentSession {
    pub fn move_check(&self) -> Result<BridgeMoveCheck, String> {
        Ok(move_check_of(run(self.thread.move_check())?))
    }

    pub fn move_out(
        &self,
        transfer: String,
        target_address: String,
        target_name: String,
        target_environment: String,
        commit: bool,
    ) -> Result<BridgeMovePackage, String> {
        Ok(BridgeMovePackage {
            package: run(self.thread.move_out(
                &transfer,
                &target_address,
                &target_name,
                &target_environment,
                commit,
                None,
            ))?,
        })
    }

    pub fn move_finish(&self, transfer: String, to_host: Option<String>) -> Result<(), String> {
        let finish = match to_host {
            Some(to_host) => MoveFinish::Seal { to_host },
            None => MoveFinish::Release,
        };
        run(self.thread.move_finish(&transfer, finish))
    }

    pub fn move_record(&self) -> Result<Option<BridgeMoveRecord>, String> {
        Ok(
            run(self.thread.move_record())?.map(|record| BridgeMoveRecord {
                from_host: record.from_host,
                to_host: record.to_host,
                at_ms: i64::try_from(record.at_ms).unwrap_or(i64::MAX),
                transcript: record
                    .transcript
                    .into_iter()
                    .map(convert::transcript_item)
                    .collect(),
            }),
        )
    }
}

impl AgentProject {
    pub(crate) fn move_in(&self, package: BridgeMovePackage) -> Result<AgentSession, String> {
        let thread: Thread = run(self.project.move_in(package.package))?;
        Ok(AgentSession::follow(thread, self.root.clone()))
    }
}

impl AgentHost {
    pub fn move_in(&self, root: String, package: BridgeMovePackage) -> BridgeMoveIn {
        match self
            .project(root)
            .and_then(|project| project.move_in(package))
        {
            Ok(session) => BridgeMoveIn {
                thread: Some(session.id()),
                lost: false,
                message: String::new(),
            },
            Err(message) => move_in_failure(message),
        }
    }

    pub fn preflight(&self, root: String, branch: String) -> Result<BridgeLanding, String> {
        Ok(landing_of(run(self.host.preflight(&root, &branch))?))
    }

    pub fn move_status(&self, transfer: String) -> Result<BridgeArrival, String> {
        Ok(arrival_of(run(self.host.move_status(&transfer))?))
    }

    pub fn move_abort(&self, transfer: String) -> Result<BridgeArrival, String> {
        Ok(arrival_of(run(self.host.move_abort(&transfer))?))
    }

    pub fn supports_moves(&self) -> Result<bool, String> {
        match run(self.host.projects()) {
            Ok(_) => Ok(true),
            Err(error) if error.starts_with(UNKNOWN_CALL) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub fn host_projects(&self) -> Result<Vec<BridgeHostProject>, String> {
        Ok(run(self.host.projects())?
            .into_iter()
            .map(host_project)
            .collect())
    }

    pub fn add_host_project(&self, root: String) -> Result<BridgeHostProject, String> {
        Ok(host_project(run(self.host.add_project(&root))?))
    }

    pub fn clone_host_project(
        &self,
        remote_url: String,
        path: String,
    ) -> Result<BridgeHostProject, String> {
        Ok(host_project(run(self
            .host
            .clone_project(&remote_url, &path))?))
    }

    pub fn configuration_changes(&self, from: &AgentHost) -> Result<BridgeConfigChanges, String> {
        Ok(BridgeConfigChanges {
            differences: run(differences(&from.host, &self.host))?,
        })
    }

    pub fn bring_configuration(
        &self,
        from: &AgentHost,
        shown: &BridgeConfigChanges,
    ) -> Result<(), String> {
        let differences = run(differences(&from.host, &self.host))?;
        unchanged(&shown.differences, &differences)?;
        if differences.is_empty() {
            return Ok(());
        }
        let changes: Vec<Change> = differences
            .into_iter()
            .map(|difference| difference.change)
            .collect();
        let number = run(self.host.propose(
            "Configuration brought along with a session move (ADR-0308)",
            changes,
        ))?;
        run(self.host.approve(number)).map(drop)
    }
}

async fn differences(from: &Host, to: &Host) -> Result<Vec<Difference>, String> {
    let from = from.config().await?.blocks;
    let to = to.config().await?.blocks;
    Ok(diff_blocks(&from, &to))
}

pub(crate) fn unchanged(shown: &[Difference], now: &[Difference]) -> Result<(), String> {
    if shown == now {
        Ok(())
    } else {
        Err("The configuration changed after its changes were shown, so nothing was brought. Check the changes again.".to_owned())
    }
}

pub(crate) fn config_change(difference: &Difference) -> BridgeConfigChange {
    let (kind, name) = match &difference.change {
        Change::Instruction { name, .. } => (BridgeConfigKind::Instruction, name),
        Change::Skill { name, .. } => (BridgeConfigKind::Skill, name),
        Change::McpServer { name, .. } => (BridgeConfigKind::McpServer, name),
    };
    BridgeConfigChange {
        kind,
        name: name.clone(),
        direction: match difference.direction {
            Direction::Add => BridgeDirection::Add,
            Direction::Replace => BridgeDirection::Replace,
            Direction::Remove => BridgeDirection::Remove,
        },
    }
}

pub(crate) fn move_check_of(check: MoveCheck) -> BridgeMoveCheck {
    BridgeMoveCheck {
        title: check.title,
        movable: check.harness.is_some(),
        harness: check.harness.map(|harness| match harness {
            Harness::ClaudeCode => "Claude Code".to_owned(),
            Harness::Codex => "Codex".to_owned(),
        }),
        model: check.model.map(|model| model.model),
        running: check.running,
        uncommitted_files: i64::from(check.uncommitted_files),
        branch: check.branch,
        remote_url: check.remote_url,
        blocked: check.blocked,
        moved: check.moved,
        moving: check.moving.map(moving_of),
        agent: check.agent.map(|agent| BridgeAgentMove {
            name: meta_harness_core::visible_name(&agent.name),
            memory_bytes: i64::try_from(agent.memory_bytes).unwrap_or(i64::MAX),
            routines: i64::from(agent.routines),
            earlier: i64::from(agent.earlier),
        }),
    }
}

pub(crate) fn moving_of(moving: Moving) -> BridgeMoving {
    BridgeMoving {
        transfer: moving.transfer,
        phase: match moving.phase {
            MovePhase::Preparing => BridgeMovePhase::Preparing,
            MovePhase::Handed => BridgeMovePhase::Handed,
        },
        target_address: moving.target_address,
        target_name: moving.target_name,
        target_environment: moving.target_environment,
        since_ms: i64::try_from(moving.since_ms).unwrap_or(i64::MAX),
    }
}

pub(crate) fn arrival_of(arrival: Arrival) -> BridgeArrival {
    let (kind, thread) = match arrival {
        Arrival::Landed { thread } => (BridgeArrivalKind::Landed, Some(thread)),
        Arrival::Starting => (BridgeArrivalKind::Starting, None),
        Arrival::Refused => (BridgeArrivalKind::Refused, None),
        Arrival::Unknown => (BridgeArrivalKind::Unknown, None),
    };
    BridgeArrival { kind, thread }
}

fn host_project(project: meta_harness_core::HostProject) -> BridgeHostProject {
    BridgeHostProject {
        root: project.root,
        title: project.title,
        remote_url: project.remote_url,
    }
}
