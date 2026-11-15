//! Spike (throwaway): Meta-Harness drives a pinned T3 Code server (ADR-0280, ADR-0281).

mod client;
mod server;

pub use client::{ORCHESTRATION_PROTOCOL, T3Client};
pub use server::{QUIET_ENV, T3_DARWIN_ARM64_SHA256, T3_VERSION, T3Server};

use serde_json::{Value, json};

fn fresh_id(prefix: &str) -> String {
    let mut bytes = [0u8; 12];
    let _ = getrandom_fill(&mut bytes);
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{prefix}-{hex}")
}

fn getrandom_fill(bytes: &mut [u8]) -> std::io::Result<()> {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")?.read_exact(bytes)
}

/// One project on a T3 server and the calls Liquid needs for one thread.
#[derive(Clone)]
pub struct T3Project {
    client: T3Client,
    /// T3's project id.
    pub project_id: String,
}

impl T3Project {
    /// Finds the T3 project for `workspace_root`, or creates it.
    pub async fn open(client: T3Client, workspace_root: &str) -> Result<Self, String> {
        let mut shell = client
            .stream("orchestration.subscribeShell", json!({}))
            .await?;
        let snapshot = shell
            .recv()
            .await
            .ok_or("subscribeShell ended without a snapshot")?;
        let existing = snapshot["snapshot"]["projects"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|p| p["workspaceRoot"].as_str() == Some(workspace_root))
            .and_then(|p| p["id"].as_str().map(str::to_owned));
        let project_id = match existing {
            Some(id) => id,
            None => {
                let id = fresh_id("project");
                client
                    .call(
                        "projects.mutate",
                        json!({
                            "type": "project.create",
                            "commandId": fresh_id("cmd"),
                            "projectId": id,
                            "title": workspace_root.rsplit('/').next().unwrap_or("project"),
                            "workspaceRoot": workspace_root,
                            "createWorkspaceRootIfMissing": false,
                            "defaultModelSelection": null,
                        }),
                    )
                    .await?;
                id
            }
        };
        Ok(Self { client, project_id })
    }

    /// Starts a thread with `text` as its first turn; returns its id.
    pub async fn launch_thread(
        &self,
        instance_id: &str,
        model: &str,
        text: &str,
        worktree_base: Option<&str>,
    ) -> Result<String, String> {
        // T3 restores files only for a thread in its own worktree.
        let strategy = match worktree_base {
            Some(base) => json!({"type": "worktree", "baseRef": base}),
            None => json!({"type": "root"}),
        };
        let thread_id = fresh_id("thread");
        self.client
            .call(
                "orchestration.launchThread",
                json!({
                    "commandId": fresh_id("cmd"),
                    "threadId": thread_id,
                    "projectId": self.project_id,
                    "title": text.lines().next().unwrap_or("Chat"),
                    "modelSelection": {"instanceId": instance_id, "model": model},
                    "runtimeMode": "full-access",
                    "interactionMode": "default",
                    "workspaceStrategy": strategy,
                    "initialMessage": {"text": text, "attachments": []},
                }),
            )
            .await?;
        Ok(thread_id)
    }

    /// Sends `text` as the next turn; a busy thread queues it.
    pub async fn send(&self, thread_id: &str, text: &str) -> Result<(), String> {
        self.client
            .call(
                "orchestration.dispatchCommand",
                json!({
                    "type": "message.dispatch",
                    "commandId": fresh_id("cmd"),
                    "threadId": thread_id,
                    "messageId": fresh_id("msg"),
                    "text": text,
                    "attachments": [],
                    "createdBy": "user",
                    "creationSource": "web",
                    "dispatchMode": {"type": "queue_after_active"},
                }),
            )
            .await
            .map(|_| ())
    }

    /// Interrupts the thread's running turn and stops its delegated tasks.
    pub async fn stop(&self, thread_id: &str) -> Result<(), String> {
        self.client
            .call(
                "orchestration.dispatchCommand",
                json!({"type": "thread.stop", "commandId": fresh_id("cmd"), "threadId": thread_id}),
            )
            .await
            .map(|_| ())
    }

    /// The thread's snapshot with its stream position, then its live events (ADR-0290).
    pub async fn subscribe(
        &self,
        thread_id: &str,
    ) -> Result<tokio::sync::mpsc::UnboundedReceiver<Value>, String> {
        self.client
            .stream(
                "orchestration.subscribeThread",
                json!({"threadId": thread_id, "requestCompletionMarker": true}),
            )
            .await
    }

    /// The unified diff from the end of turn `from` to the end of turn `to`; turn 0 is the
    /// thread's start.
    pub async fn turn_diff(&self, thread_id: &str, from: u64, to: u64) -> Result<String, String> {
        let result = self
            .client
            .call(
                "orchestration.getTurnDiff",
                json!({"threadId": thread_id, "fromTurnCount": from, "toTurnCount": to}),
            )
            .await?;
        Ok(result["diff"].as_str().unwrap_or_default().to_owned())
    }

    /// Restores the files to the end of turn `turn` (0: before the first turn), as T3's
    /// "edit from here" does, with the checkpoint `projection` lists for it.
    pub async fn revert_to(
        &self,
        thread_id: &str,
        projection: &Value,
        turn: u64,
    ) -> Result<(), String> {
        let checkpoint = projection["checkpoints"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|c| {
                c["status"] == "ready"
                    && if turn == 0 {
                        c["ordinalWithinScope"] == 0 && c["appRunOrdinal"].is_null()
                    } else {
                        c["appRunOrdinal"].as_u64() == Some(turn)
                    }
            })
            .ok_or(format!("no ready checkpoint for turn {turn}"))?;
        self.client
            .call(
                "orchestration.dispatchCommand",
                json!({
                    "type": "checkpoint.rollback",
                    "commandId": fresh_id("cmd"),
                    "threadId": thread_id,
                    "scopeId": checkpoint["scopeId"],
                    "checkpointId": checkpoint["id"],
                    "restoreFiles": true,
                }),
            )
            .await
            .map(|_| ())
    }

    /// The thread's current projection, read through a fresh subscription.
    pub async fn projection(&self, thread_id: &str) -> Result<(u64, Value), String> {
        let mut stream = self.subscribe(thread_id).await?;
        while let Some(item) = stream.recv().await {
            if item["kind"] == "snapshot" {
                return Ok((
                    item["snapshotSequence"].as_u64().unwrap_or_default(),
                    item["projection"].clone(),
                ));
            }
        }
        Err("subscribeThread ended without a snapshot".to_owned())
    }
}
