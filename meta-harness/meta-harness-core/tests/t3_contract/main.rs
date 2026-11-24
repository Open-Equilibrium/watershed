#![cfg(unix)]
#![allow(clippy::unwrap_used)]

use futures_util::FutureExt;
use meta_harness_core::{
    Access, AcpAgent, AgentChange, AgentChangeKind, AgentEvent, Arrival, CONNECTION_FILE, Change,
    ChatItem, ClientKind, GitAction, GitActionRequest, GitProgress, Harness, Host, HostConfig,
    ItemDetail, Landing, LineCount, ModelChoice, MoveFinish, MovePackage, MovePhase, NewAgent,
    NewRoutine, Permissions, Request, Schedule, ScheduleKind, Service, ServiceConfig, Snapshot,
    StartThread, TurnStatus, Workspace, remote, t3_version,
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
