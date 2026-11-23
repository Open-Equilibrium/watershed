use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{Host, Stream};
use crate::api::{self, Call, Checkout, GitCall, ProjectCall, ThreadCall};
use crate::{
    Automation, Context, GitActionRequest, GitProgress, GitStatus, Lineage, LineageChanges,
    LocalServer, ModelChoice, MoveCheck, MoveFinish, MovePackage, MoveRecord, Permissions,
    PullRequestDetail, Refs, RequestId, ResolvedPullRequest, Snapshot, StartThread, Statistics,
    ThreadEvent, ThreadSummary, Turn, UsageLimits, Watch,
};

impl Host {
    pub async fn project(&self, root: &Path) -> Result<Project, String> {
        let project = Project {
            host: self.clone(),
            root: root.to_string_lossy().into_owned(),
        };
        project.threads().await?;
        Ok(project)
    }
}

#[derive(Clone)]
pub struct Project {
    host: Host,
    root: String,
}

impl Project {
    async fn call<T: DeserializeOwned>(&self, call: ProjectCall) -> Result<T, String> {
        self.host
            .call(Call::Project {
                root: self.root.clone(),
                call,
            })
            .await
    }

    pub async fn threads(&self) -> Result<Vec<ThreadSummary>, String> {
        self.call(ProjectCall::Threads).await
    }

    pub async fn start(&self, start: StartThread) -> Result<Thread, String> {
        let id: String = self.call(ProjectCall::Start(start)).await?;
        Ok(self.thread(id))
    }

    pub async fn move_in(&self, package: MovePackage) -> Result<Thread, String> {
        let id: String = self.call(ProjectCall::MoveIn(package)).await?;
        Ok(self.thread(id))
    }

    pub async fn open(&self, id: &str) -> Result<Thread, String> {
        self.call::<()>(ProjectCall::Open(id.to_owned())).await?;
        Ok(self.thread(id.to_owned()))
    }

    fn thread(&self, id: String) -> Thread {
        Thread {
            host: self.host.clone(),
            root: self.root.clone(),
            id,
        }
    }

    pub fn git(&self) -> Git {
        Git {
            host: self.host.clone(),
            checkout: Checkout::Project(self.root.clone()),
        }
    }

    pub async fn only_thread_in_worktree(&self, thread: &ThreadSummary) -> Result<bool, String> {
        self.call(ProjectCall::OnlyThreadInWorktree(thread.clone()))
            .await
    }

    pub async fn delete(&self, thread: &ThreadSummary, worktree: bool) -> Result<(), String> {
        self.call(ProjectCall::Delete {
            thread: thread.clone(),
            worktree,
        })
        .await
    }

    pub async fn archive(&self, thread: &str, archived: bool) -> Result<(), String> {
        self.call(ProjectCall::Archive {
            thread: thread.to_owned(),
            archived,
        })
        .await
    }
}

#[derive(Clone)]
pub struct Thread {
    host: Host,
    root: String,
    id: String,
}

impl Thread {
    async fn call<T: DeserializeOwned>(&self, call: ThreadCall) -> Result<T, String> {
        self.host
            .call(Call::Thread {
                root: self.root.clone(),
                id: self.id.clone(),
                call,
            })
            .await
    }

    fn names(&self) -> (String, String) {
        (self.root.clone(), self.id.clone())
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn git(&self) -> Git {
        let (root, id) = self.names();
        Git {
            host: self.host.clone(),
            checkout: Checkout::Thread { root, id },
        }
    }

    pub async fn subscribe(&self) -> Result<(Snapshot, Stream<ThreadEvent>), String> {
        let (root, id) = self.names();
        self.host.stream(api::Stream::Subscribe { root, id }).await
    }

    pub async fn servers(&self) -> Result<Watch<Vec<LocalServer>>, String> {
        let (root, id) = self.names();
        let (first, stream) = self.host.stream(api::Stream::Servers { root, id }).await?;
        Ok(Watch::from_stream(first, stream))
    }

    pub async fn automations(&self) -> Result<Watch<Vec<Automation>>, String> {
        let (root, id) = self.names();
        let (first, stream) = self
            .host
            .stream(api::Stream::Automations { root, id })
            .await?;
        Ok(Watch::from_stream(first, stream))
    }

    pub async fn lineage_changes(&self) -> Result<LineageChanges, String> {
        let (root, id) = self.names();
        let ((), stream) = self
            .host
            .stream(api::Stream::LineageChanges { root, id })
            .await?;
        Ok(LineageChanges::from_stream(stream))
    }
}

macro_rules! reads {
    ($($(#[$doc:meta])* $name:ident: $call:ident -> $out:ty;)*) => {
        impl Thread {
            $($(#[$doc])* pub async fn $name(&self) -> Result<$out, String> {
                self.call(ThreadCall::$call).await
            })*
        }
    };
}

reads! {
    title: Title -> String;
    worktree: Worktree -> Option<PathBuf>;
    ended: Ended -> bool;
    snapshot: Snapshot -> Snapshot;
    turns: Turns -> Vec<Turn>;
    statistics: Statistics -> Statistics;
    context: Context -> Option<Context>;
    model: Model -> Option<ModelChoice>;
    record: Record -> Value;
    stop: Stop -> ();
    usage_limits: UsageLimits -> Option<UsageLimits>;
    lineage: Lineage -> Lineage;
    merge_back: MergeBack -> ();
    disconnect: Disconnect -> ();
}

impl Thread {
    pub async fn move_check(&self) -> Result<MoveCheck, String> {
        self.call(ThreadCall::MoveCheck).await
    }

    pub async fn move_out(
        &self,
        transfer: &str,
        target_address: &str,
        target_name: &str,
        target_environment: &str,
        commit: bool,
        message: Option<String>,
    ) -> Result<MovePackage, String> {
        self.call(ThreadCall::MoveOut {
            transfer: transfer.to_owned(),
            target_address: target_address.to_owned(),
            target_name: target_name.to_owned(),
            target_environment: target_environment.to_owned(),
            commit,
            message,
        })
        .await
    }

    pub async fn move_finish(&self, transfer: &str, finish: MoveFinish) -> Result<(), String> {
        self.call(ThreadCall::MoveFinish {
            transfer: transfer.to_owned(),
            finish,
        })
        .await
    }

    pub async fn move_record(&self) -> Result<Option<MoveRecord>, String> {
        self.call(ThreadCall::MoveRecord).await
    }

    pub async fn send(&self, text: &str, steer: bool) -> Result<(), String> {
        self.call(ThreadCall::Send {
            text: text.to_owned(),
            steer,
        })
        .await
    }

    pub async fn diff(
        &self,
        from: u64,
        to: u64,
        ignore_whitespace: bool,
    ) -> Result<String, String> {
        self.call(ThreadCall::Diff {
            from,
            to,
            ignore_whitespace,
        })
        .await
    }

    pub async fn revert(&self, turn: u64, restore_files: bool) -> Result<String, String> {
        self.call(ThreadCall::Revert {
            turn,
            restore_files,
        })
        .await
    }

    pub async fn fork(&self, turn: u64) -> Result<String, String> {
        self.call(ThreadCall::Fork(turn)).await
    }

    pub async fn set_permissions(&self, permissions: Permissions) -> Result<(), String> {
        self.call(ThreadCall::SetPermissions(permissions)).await
    }

    pub async fn set_model(&self, model: &ModelChoice) -> Result<(), String> {
        self.call(ThreadCall::SetModel(model.clone())).await
    }

    pub async fn send_queued_now(&self, turn: u64) -> Result<(), String> {
        self.call(ThreadCall::SendQueuedNow(turn)).await
    }

    pub async fn edit_queued(&self, turn: u64, text: &str) -> Result<(), String> {
        self.call(ThreadCall::EditQueued {
            turn,
            text: text.to_owned(),
        })
        .await
    }

    pub async fn cancel_queued(&self, turn: u64) -> Result<(), String> {
        self.call(ThreadCall::CancelQueued(turn)).await
    }

    pub async fn respond(&self, request: &RequestId, decision: &str) -> Result<(), String> {
        self.call(ThreadCall::Respond {
            request: request.0.clone(),
            decision: decision.to_owned(),
        })
        .await
    }

    pub async fn answer(
        &self,
        request: &RequestId,
        answers: &[(String, Vec<String>)],
    ) -> Result<(), String> {
        self.call(ThreadCall::Answer {
            request: request.0.clone(),
            answers: answers.to_vec(),
        })
        .await
    }

    pub async fn dismiss(&self, request: &RequestId) -> Result<(), String> {
        self.call(ThreadCall::Dismiss(request.0.clone())).await
    }

    pub async fn set_branch(&self, branch: &str, worktree: Option<&Path>) -> Result<(), String> {
        self.call(ThreadCall::SetBranch {
            branch: branch.to_owned(),
            worktree: worktree.map(|p| p.to_string_lossy().into_owned()),
        })
        .await
    }

    pub async fn stop_server(&self, pid: u32, port: u16) -> Result<(), String> {
        self.call(ThreadCall::StopServer { pid, port }).await
    }

    pub async fn set_automation_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        self.call(ThreadCall::SetAutomationEnabled {
            id: id.to_owned(),
            enabled,
        })
        .await
    }

    pub async fn run_automation_now(&self, id: &str) -> Result<(), String> {
        self.call(ThreadCall::RunAutomationNow(id.to_owned())).await
    }

    pub async fn visit(&self) -> Result<(), String> {
        self.call(ThreadCall::Visit).await
    }

    pub async fn stop_subagent(&self, subagent: &str) -> Result<(), String> {
        self.call(ThreadCall::StopSubagent(subagent.to_owned()))
            .await
    }
}

#[derive(Clone)]
pub struct Git {
    host: Host,
    checkout: Checkout,
}

impl Git {
    async fn call<T: DeserializeOwned>(&self, call: GitCall) -> Result<T, String> {
        self.host
            .call(Call::Git {
                checkout: self.checkout.clone(),
                call,
            })
            .await
    }

    pub async fn status(&self) -> Result<GitStatus, String> {
        self.call(GitCall::Status).await
    }

    pub async fn watch(&self) -> Result<Stream<GitStatus>, String> {
        let ((), stream) = self
            .host
            .stream(api::Stream::GitWatch(self.checkout.clone()))
            .await?;
        Ok(stream)
    }

    pub async fn refs(&self, query: &str, cursor: u64, limit: u64) -> Result<Refs, String> {
        self.call(GitCall::Refs {
            query: query.to_owned(),
            cursor,
            limit,
        })
        .await
    }

    pub async fn switch(&self, name: &str) -> Result<(), String> {
        self.call(GitCall::Switch(name.to_owned())).await
    }

    pub async fn create_branch(&self, name: &str) -> Result<(), String> {
        self.call(GitCall::CreateBranch(name.to_owned())).await
    }

    pub async fn init(&self) -> Result<(), String> {
        self.call(GitCall::Init).await
    }

    pub async fn pull(&self) -> Result<(String, String), String> {
        self.call(GitCall::Pull).await
    }

    pub async fn run(&self, request: GitActionRequest) -> Result<Stream<GitProgress>, String> {
        let ((), stream) = self
            .host
            .stream(api::Stream::GitRun {
                checkout: self.checkout.clone(),
                request,
            })
            .await?;
        Ok(stream)
    }

    pub async fn branch_diff(&self, ignore_whitespace: bool) -> Result<String, String> {
        self.call(GitCall::BranchDiff(ignore_whitespace)).await
    }

    pub async fn uncommitted_diff(&self, ignore_whitespace: bool) -> Result<String, String> {
        self.call(GitCall::UncommittedDiff(ignore_whitespace)).await
    }

    pub async fn resolve_pull_request(
        &self,
        reference: &str,
    ) -> Result<ResolvedPullRequest, String> {
        self.call(GitCall::ResolvePullRequest(reference.to_owned()))
            .await
    }

    pub async fn check_out_pull_request(
        &self,
        reference: &str,
        worktree: bool,
    ) -> Result<(String, Option<PathBuf>), String> {
        self.call(GitCall::CheckOutPullRequest {
            reference: reference.to_owned(),
            worktree,
        })
        .await
    }

    pub async fn publish(
        &self,
        provider: &str,
        repository: &str,
        public: bool,
    ) -> Result<(), String> {
        self.call(GitCall::Publish {
            provider: provider.to_owned(),
            repository: repository.to_owned(),
            public,
        })
        .await
    }

    pub async fn pull_request(&self, number: u64) -> Result<PullRequestDetail, String> {
        self.call(GitCall::PullRequest(number)).await
    }

    pub async fn run_pull_request_action(
        &self,
        number: u64,
        action: &str,
        merge_method: Option<&str>,
    ) -> Result<(), String> {
        self.call(GitCall::RunPullRequestAction {
            number,
            action: action.to_owned(),
            merge_method: merge_method.map(str::to_owned),
        })
        .await
    }

    pub async fn pull_request_worktree(
        &self,
        reference: &str,
    ) -> Result<(String, Option<PathBuf>), String> {
        self.call(GitCall::PullRequestWorktree(reference.to_owned()))
            .await
    }
}
