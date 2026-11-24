use std::time::Duration;

use super::{Thread, TurnStatus};
use crate::handoff::{self, TRANSCRIPT_BYTES};
use crate::host::projects::remote_url;
use crate::host_move::{Departure, Facts, Movable, MovePackage};
use crate::util::until;
use crate::{GitAction, GitActionRequest, GitProgress, ModelChoice, Permissions};

const TURN_END: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(100);

const PREPARE_WAIT: Duration = Duration::from_secs(10);

const NOT_SETTLED: &str = "A queued message is still being prepared, so the session did not move. Nothing changed; try again.";

const RACED: &str = "The session changed while the move started, so the session did not move. Its queued messages are back in the queue; try again.";

pub(crate) struct Saving<'a> {
    pub save: &'a (dyn Fn(u64, &str) -> Result<(), String> + Sync),
    pub unsave: &'a (dyn Fn(u64) + Sync),
    pub open: &'a (dyn Fn() -> Result<(), String> + Sync),
}

impl Thread {
    pub(crate) async fn move_facts(&self) -> Result<Facts, String> {
        let status = self.git().status().await?;
        let remote_url = remote_url(&self.client, &self.root).await?;
        let (title, model, mode) = {
            let state = self.state();
            let thread = &state.projection.thread;
            (
                thread["title"].as_str().unwrap_or_default().to_owned(),
                ModelChoice::from_t3(&thread["modelSelection"]),
                thread["runtimeMode"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            )
        };
        let turns = self.turns();
        let queued_attachments = self.state().projection.queued_with_attachments();
        Ok(Facts {
            title,
            model,
            permissions: Permissions::from_mode(&mode).unwrap_or(Permissions::Supervised),
            running: turns.iter().any(|turn| turn.status == TurnStatus::Running),
            uncommitted_files: u32::try_from(status.changes.len()).unwrap_or(u32::MAX),
            branch: status.branch,
            remote_url,
            queued_attachments,
        })
    }

    pub(crate) async fn move_out(
        &self,
        movable: Movable,
        departure: Departure<'_>,
        commit: bool,
        message: Option<String>,
        saving: Saving<'_>,
    ) -> Result<MovePackage, String> {
        let Saving { save, unsave, open } = saving;
        let Movable {
            title,
            model,
            harness,
            permissions,
            branch,
            remote_url,
            earlier,
        } = movable;
        self.settle_preparing(PREPARE_WAIT).await?;
        let waiting: Vec<(u64, String)> = self
            .turns()
            .into_iter()
            .filter(|turn| turn.status == TurnStatus::Queued)
            .map(|turn| (turn.number, turn.prompt))
            .collect();
        let running = self
            .turns()
            .iter()
            .any(|turn| turn.status == TurnStatus::Running);
        async {
            for (turn, text) in &waiting {
                open()?;
                save(*turn, text)?;
                if self.cancel_queued(*turn).await.is_err() {
                    unsave(*turn);
                    return Err(RACED.to_owned());
                }
            }
            if running {
                open()?;
                self.stop().await?;
                self.wait_for_turns().await?;
            }
            open()?;
            let status = self.git().status().await?;
            if status.branch.as_deref() != Some(branch.as_str()) {
                return Err(RACED.to_owned());
            }
            let uncommitted = status.changes.len();
            let action = if commit && uncommitted > 0 {
                GitAction::CommitPush
            } else {
                GitAction::Push
            };
            let marks = self.marks();
            open()?;
            self.push(action, message).await?;
            let pushed = self.git().status().await?;
            if pushed.branch.as_deref() != Some(branch.as_str())
                || (commit && !pushed.changes.is_empty())
                || self
                    .turns()
                    .iter()
                    .any(|turn| matches!(turn.status, TurnStatus::Queued | TurnStatus::Running))
                || self.marks() != marks
            {
                return Err(RACED.to_owned());
            }
            let snapshot = self.snapshot();
            let handoff = handoff::render(&branch, &snapshot.transcript);
            let carried = match earlier {
                Some(earlier) => earlier
                    .into_iter()
                    .chain(snapshot.transcript.into_iter().skip(1))
                    .collect(),
                None => snapshot.transcript,
            };
            let (transcript, transcript_capped) =
                handoff::cap_transcript(carried, TRANSCRIPT_BYTES);
            Ok(MovePackage {
                transfer: departure.transfer.to_owned(),
                from_host: departure.from_host.to_owned(),
                title,
                harness,
                model,
                permissions,
                branch,
                remote_url,
                handoff,
                transcript,
                transcript_capped,
                queued: Vec::new(),
                agent: None,
            })
        }
        .await
    }

    pub(crate) async fn pause_automations(&self) -> Result<(), String> {
        let tasks = self.automations().await?;
        for task in tasks.current().iter().filter(|task| task.enabled) {
            self.set_automation_enabled(&task.id, false).await?;
        }
        Ok(())
    }

    pub(crate) async fn settle_preparing(&self, limit: Duration) -> Result<(), String> {
        if until(limit, POLL, || !self.preparing()).await {
            Ok(())
        } else {
            Err(NOT_SETTLED.to_owned())
        }
    }

    fn preparing(&self) -> bool {
        self.state()
            .projection
            .runs
            .iter()
            .any(|run| run["status"] == "preparing")
    }

    fn marks(&self) -> Vec<(u64, TurnStatus)> {
        self.turns()
            .iter()
            .map(|turn| (turn.number, turn.status))
            .collect()
    }

    async fn wait_for_turns(&self) -> Result<(), String> {
        let running = || {
            self.turns()
                .iter()
                .any(|turn| turn.status == TurnStatus::Running)
        };
        if until(TURN_END, POLL, || !running()).await {
            Ok(())
        } else {
            Err("The current turn did not end, so the session did not move.".to_owned())
        }
    }

    async fn push(&self, action: GitAction, message: Option<String>) -> Result<(), String> {
        let mut progress = self
            .git()
            .run(GitActionRequest {
                action,
                message,
                files: None,
                feature_branch: false,
            })
            .await?;
        let mut last = None;
        while let Some(step) = progress.recv().await {
            last = Some(step);
        }
        match last {
            Some(GitProgress::Finished { .. }) => Ok(()),
            Some(GitProgress::Failed(cause)) => Err(format!("The branch was not pushed: {cause}")),
            _ => Err("The branch was not pushed: T3 reported no result.".to_owned()),
        }
    }
}
