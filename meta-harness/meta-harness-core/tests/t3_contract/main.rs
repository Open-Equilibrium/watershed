#![cfg(unix)]
#![allow(clippy::unwrap_used)]

use futures_util::FutureExt;
use meta_harness_core::{
    Access, Arrival, CONNECTION_FILE, Change, ClientKind, Landing, MoveFinish, MovePackage,
    MovePhase, Service, ServiceConfig, remote, t3_version,
};
use meta_harness_core::{
    AcpAgent, AgentChange, AgentChangeKind, AgentEvent, GitAction, GitActionRequest, GitProgress,
    Group, Harness, Host, HostConfig, ItemDetail, LineCount, ModelChoice, NewAgent, NewRoutine,
    Permissions, Relation, Request, Schedule, ScheduleKind, Snapshot, StartThread, TurnStatus,
    Workspace,
};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

mod agents;
mod git;
mod links;
mod moves_landing;
mod moves_source;
mod moves_target;
mod remote_access;
mod service;
mod support;
mod threads;
mod tunnel;

use support::*;
