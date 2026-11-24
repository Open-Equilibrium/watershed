use crate::api::{
    self, AgentsCall, Call, ConfigCall, HostCall, HostsCall, RemoteCall, RemoteStatus,
};
use crate::config::{Change, Current, LogEntry, Proposal};
use crate::hosts::{Access, ClientKind, HostList};
use crate::{Arrival, HostProject, Landing, ModelChoice, Provider};

use super::Host;

impl Host {
    pub async fn status(&self) -> Result<api::Status, String> {
        self.call(Call::Host(HostCall::Status)).await
    }

    pub async fn providers(&self) -> Result<Vec<Provider>, String> {
        self.call(Call::Host(HostCall::Providers)).await
    }

    pub async fn environment_name(&self) -> Result<String, String> {
        self.call(Call::Host(HostCall::EnvironmentName)).await
    }

    pub async fn environment_machine(&self) -> Result<String, String> {
        self.call(Call::Host(HostCall::EnvironmentMachine)).await
    }

    pub async fn environment_id(&self) -> Result<String, String> {
        self.call(Call::Host(HostCall::EnvironmentId)).await
    }

    pub async fn projects(&self) -> Result<Vec<HostProject>, String> {
        self.call(Call::Host(HostCall::Projects)).await
    }

    pub async fn preflight(&self, root: &str, branch: &str) -> Result<Landing, String> {
        self.call(Call::Host(HostCall::Preflight {
            root: root.to_owned(),
            branch: branch.to_owned(),
        }))
        .await
    }

    pub async fn move_status(&self, transfer: &str) -> Result<Arrival, String> {
        self.call(Call::Host(HostCall::MoveStatus {
            transfer: transfer.to_owned(),
        }))
        .await
    }

    pub async fn move_abort(&self, transfer: &str) -> Result<Arrival, String> {
        self.call(Call::Host(HostCall::MoveAbort {
            transfer: transfer.to_owned(),
        }))
        .await
    }

    pub async fn add_project(&self, root: &str) -> Result<HostProject, String> {
        self.call(Call::Host(HostCall::AddProject {
            root: root.to_owned(),
        }))
        .await
    }

    pub async fn clone_project(&self, remote_url: &str, path: &str) -> Result<HostProject, String> {
        self.call(Call::Host(HostCall::CloneProject {
            remote_url: remote_url.to_owned(),
            path: path.to_owned(),
        }))
        .await
    }

    pub async fn writer_model(&self) -> Result<Option<ModelChoice>, String> {
        self.call(Call::Host(HostCall::WriterModel)).await
    }

    pub async fn set_writer_model(&self, choice: &ModelChoice) -> Result<(), String> {
        self.call(Call::Host(HostCall::SetWriterModel(choice.clone())))
            .await
    }

    pub async fn config(&self) -> Result<Current, String> {
        self.call(Call::Config(ConfigCall::Current)).await
    }

    pub async fn propose(&self, summary: &str, changes: Vec<Change>) -> Result<u64, String> {
        self.call(Call::Config(ConfigCall::Propose {
            summary: summary.to_owned(),
            changes,
        }))
        .await
    }

    pub async fn propose_revert(&self, revision: u64) -> Result<u64, String> {
        self.call(Call::Config(ConfigCall::Revert(revision))).await
    }

    pub async fn pending(&self) -> Result<Vec<Proposal>, String> {
        self.call(Call::Config(ConfigCall::Pending)).await
    }

    pub async fn approve(&self, number: u64) -> Result<Current, String> {
        self.call(Call::Config(ConfigCall::Approve(number))).await
    }

    pub async fn reject(&self, number: u64) -> Result<(), String> {
        self.call(Call::Config(ConfigCall::Reject(number))).await
    }

    pub async fn log(&self) -> Result<Vec<LogEntry>, String> {
        self.call(Call::Config(ConfigCall::Log)).await
    }

    pub async fn remote_on(&self, port: u16) -> Result<RemoteStatus, String> {
        self.call(Call::Remote(RemoteCall::On { port })).await
    }

    pub async fn remote_off(&self) -> Result<(), String> {
        self.call(Call::Remote(RemoteCall::Off)).await
    }

    pub async fn remote_status(&self) -> Result<Option<RemoteStatus>, String> {
        self.call(Call::Remote(RemoteCall::Status)).await
    }

    pub async fn invite(&self, kind: ClientKind, access: Access) -> Result<String, String> {
        self.call(Call::Invite { kind, access }).await
    }

    pub async fn t3_pairing(&self) -> Result<String, String> {
        self.call(Call::Hosts(HostsCall::T3Pairing)).await
    }

    pub async fn revoke_client(&self, name: &str) -> Result<(), String> {
        self.call(Call::Hosts(HostsCall::Revoke(name.to_owned())))
            .await
    }

    pub async fn host_add(
        &self,
        address: &str,
        code: &str,
        access: Option<Access>,
    ) -> Result<(String, Access), String> {
        self.call(Call::Hosts(HostsCall::Add {
            address: address.to_owned(),
            code: code.to_owned(),
            access,
        }))
        .await
    }

    pub async fn host_remove(&self, name: &str) -> Result<String, String> {
        self.call(Call::Hosts(HostsCall::Remove(name.to_owned())))
            .await
    }

    pub async fn hosts(&self) -> Result<HostList, String> {
        self.call(Call::Hosts(HostsCall::List)).await
    }

    pub async fn host_agents(&self, on: bool) -> Result<(), String> {
        self.call(Call::Hosts(HostsCall::Agents(on))).await
    }

    pub async fn agents(&self) -> Result<Vec<crate::Agent>, String> {
        self.call(Call::Agents(AgentsCall::List)).await
    }

    pub async fn agent_create(&self, new: crate::NewAgent) -> Result<crate::Agent, String> {
        self.call(Call::Agents(AgentsCall::Create(new))).await
    }

    pub async fn agent_remove(&self, name: &str) -> Result<std::path::PathBuf, String> {
        self.call(Call::Agents(AgentsCall::Remove(name.to_owned())))
            .await
    }

    pub async fn agent_routines(&self, name: &str) -> Result<Vec<crate::Automation>, String> {
        self.call(Call::Agents(AgentsCall::Routines(name.to_owned())))
            .await
    }

    pub async fn agent_routine_add(
        &self,
        name: &str,
        routine: crate::NewRoutine,
    ) -> Result<crate::AddedRoutine, String> {
        self.call(Call::Agents(AgentsCall::AddRoutine {
            name: name.to_owned(),
            routine,
        }))
        .await
    }

    pub async fn agent_routine_remove(&self, name: &str, id: &str) -> Result<(), String> {
        self.call(Call::Agents(AgentsCall::RemoveRoutine {
            name: name.to_owned(),
            id: id.to_owned(),
        }))
        .await
    }

    pub async fn agent_update(
        &self,
        name: &str,
        rename: Option<String>,
        model: Option<crate::ModelChoice>,
        permissions: Option<crate::Permissions>,
    ) -> Result<crate::Agent, String> {
        self.call(Call::Agents(AgentsCall::Update {
            name: name.to_owned(),
            rename,
            model,
            permissions,
        }))
        .await
    }

    pub async fn agent_instructions(&self, name: &str) -> Result<String, String> {
        self.call(Call::Agents(AgentsCall::Instructions(name.to_owned())))
            .await
    }

    pub async fn agent_set_instructions(&self, name: &str, text: &str) -> Result<(), String> {
        self.call(Call::Agents(AgentsCall::SetInstructions {
            name: name.to_owned(),
            text: text.to_owned(),
        }))
        .await
    }

    pub async fn agent_changes(&self, name: &str) -> Result<Vec<crate::AgentChange>, String> {
        self.call(Call::Agents(AgentsCall::Changes(name.to_owned())))
            .await
    }

    pub async fn agent_apply(&self, name: &str, change: i64) -> Result<(), String> {
        self.call(Call::Agents(AgentsCall::Apply {
            name: name.to_owned(),
            change,
        }))
        .await
    }

    pub async fn agent_undo(&self, name: &str, change: i64) -> Result<(), String> {
        self.call(Call::Agents(AgentsCall::Undo {
            name: name.to_owned(),
            change,
        }))
        .await
    }

    pub async fn agent_mention(
        &self,
        name: &str,
        root: &str,
        thread: &str,
        text: &str,
    ) -> Result<String, String> {
        self.call(Call::Agents(AgentsCall::Mention {
            name: name.to_owned(),
            root: root.to_owned(),
            thread: thread.to_owned(),
            text: text.to_owned(),
        }))
        .await
    }

    pub async fn agent_mentions(&self, thread: &str) -> Result<Vec<crate::Mention>, String> {
        self.call(Call::Agents(AgentsCall::Mentions(thread.to_owned())))
            .await
    }
}
