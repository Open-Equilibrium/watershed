//! Git for one checkout through T3's VCS and stacked-action replies (ADR-0282).

use std::path::PathBuf;

use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

use super::fake_t3::{Fake, fail, ok};
use crate::git::{
    BranchChanges, BranchPullRequest, ChangedFile, Git, GitAction, GitActionRequest, GitProgress,
    GitStatus, LineCount, Ref, Refs, ResolvedPullRequest,
};
use crate::t3::Client;

const CWD: &str = "/repo";

/// A stand-in whose replies come from `answer`, and Git for the checkout `CWD` of `thread`.
pub(super) async fn checkout_in(
    answer: impl Fn(&str, &Value, &str) -> Vec<Message> + Send + 'static,
    thread: Option<&str>,
) -> (Fake, Git) {
    let fake = Fake::start(answer).await;
    let client = Client::connect(&fake.origin, || Box::pin(async { Ok("token".to_owned()) }))
        .await
        .unwrap();
    (
        fake,
        Git::new(client, CWD.to_owned(), thread.map(str::to_owned)),
    )
}

async fn checkout(
    answer: impl Fn(&str, &Value, &str) -> Vec<Message> + Send + 'static,
) -> (Fake, Git) {
    checkout_in(answer, None).await
}

fn local(file: &str) -> Value {
    json!({
        "isRepo": true, "hasPrimaryRemote": true, "isDefaultRef": true, "refName": "main",
        "sourceControlProvider": {"name": "GitHub"},
        "workingTree": {"files": [{"path": file, "insertions": 2, "deletions": 1}],
            "insertions": 2, "deletions": 1},
    })
}

fn remote(behind: u64, pull_request: Value) -> Value {
    json!({
        "hasUpstream": true, "aheadCount": 0, "behindCount": behind, "aheadOfDefaultCount": 2,
        "pr": pull_request,
    })
}

/// T3's status result: the local and remote parts together.
fn merged(mut local: Value, remote: Value) -> Value {
    local
        .as_object_mut()
        .unwrap()
        .extend(remote.as_object().unwrap().clone());
    local
}

#[tokio::test(flavor = "multi_thread")]
async fn status_reports_the_checkout_and_its_pull_request() {
    let pull_request = json!({"number": 42, "title": "Add", "url": "https://example.test/pull/42",
        "state": "open", "isDraft": true, "baseRef": "main"});
    let status = merged(local("README.md"), remote(3, pull_request));
    let (fake, git) = checkout(move |_, _, id| vec![ok(id, status.clone())]).await;
    assert_eq!(
        git.status().await.unwrap(),
        GitStatus {
            repository: true,
            branch: Some("main".to_owned()),
            default_branch: true,
            remote: true,
            provider: Some("GitHub".to_owned()),
            changes: vec![ChangedFile {
                path: "README.md".to_owned(),
                added: 2,
                removed: 1
            }],
            working_tree_lines: LineCount {
                added: 2,
                removed: 1,
            },
            branch_changes: None,
            upstream: true,
            ahead: 0,
            behind: 3,
            ahead_of_default: Some(2),
            pull_request: Some(BranchPullRequest {
                number: 42,
                title: "Add".to_owned(),
                url: "https://example.test/pull/42".to_owned(),
                state: "open".to_owned(),
                draft: true,
                base: "main".to_owned(),
            }),
        }
    );
    assert_eq!(
        fake.payloads("vcs.refreshStatus"),
        vec![json!({"cwd": CWD})]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn status_maps_the_branch_changes_and_the_working_tree_totals_as_t3_sends_them() {
    let mut status = local("README.md");
    status["workingTree"]["insertions"] = json!(5);
    status["workingTree"]["deletions"] = json!(3);
    status["branchChanges"] = json!({"baseRef": "origin/main", "insertions": 9, "deletions": 4});
    let status = merged(status, remote(0, Value::Null));
    let (_, git) = checkout(move |_, _, id| vec![ok(id, status.clone())]).await;
    let status = git.status().await.unwrap();
    assert_eq!(
        status.working_tree_lines,
        LineCount {
            added: 5,
            removed: 3
        }
    );
    assert_eq!(
        status.branch_changes,
        Some(BranchChanges {
            base: Some("origin/main".to_owned()),
            lines: LineCount {
                added: 9,
                removed: 4
            },
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_diff_asks_t3_for_the_patch_of_the_branch_range() {
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n+added\n";
    let preview = json!({
        "cwd": CWD, "generatedAt": "2026-11-16T10:00:00.000Z",
        "sources": [{"id": "branch-range", "kind": "branch-range", "title": "Changes vs main",
            "baseRef": "main", "headRef": "feature", "diff": patch, "diffHash": "h",
            "truncated": false}],
    });
    let (fake, git) = checkout(move |_, _, id| vec![ok(id, preview.clone())]).await;
    assert_eq!(git.branch_diff(true).await.unwrap(), patch);
    assert_eq!(
        fake.payloads("review.getDiffPreview"),
        vec![json!({"cwd": CWD, "sourceKind": "branch-range", "ignoreWhitespace": true})]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn status_of_a_folder_outside_any_repository_has_no_branch_or_pull_request() {
    let status = json!({"isRepo": false, "hasPrimaryRemote": false, "isDefaultRef": false,
        "workingTree": {"files": []}, "hasUpstream": false, "aheadCount": 0, "behindCount": 0,
        "pr": null});
    let (_, git) = checkout(move |_, _, id| vec![ok(id, status.clone())]).await;
    assert_eq!(git.status().await.unwrap(), GitStatus::default());
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_reports_each_change_of_the_local_or_remote_status() {
    let (fake, git) = checkout(|_, _, _| Vec::new()).await;
    let mut statuses = git.watch().await.unwrap();
    fake.received("subscribeVcsStatus", 1).await;
    fake.push(
        "subscribeVcsStatus",
        vec![
            json!({"_tag": "snapshot", "local": local("README.md"), "remote": remote(0, Value::Null)}),
            json!({"_tag": "localUpdated", "local": local("src/lib.rs")}),
            json!({"_tag": "unknown"}),
            json!({"_tag": "remoteUpdated", "remote": remote(3, Value::Null)}),
        ],
    );
    let mut seen = Vec::new();
    for _ in 0..3 {
        seen.push(statuses.recv().await.unwrap());
    }
    let shown: Vec<_> = seen
        .iter()
        .map(|s| (s.changes[0].path.as_str(), s.behind))
        .collect();
    assert_eq!(
        shown,
        vec![("README.md", 0), ("src/lib.rs", 0), ("src/lib.rs", 3)]
    );
    assert_eq!(
        fake.payloads("subscribeVcsStatus"),
        vec![json!({"cwd": CWD, "includeRemote": true})]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn refs_are_listed_a_page_at_a_time_with_the_worktrees_they_are_in() {
    let page = json!({
        "refs": [
            {"name": "main", "current": true, "isDefault": true},
            {"name": "main", "isRemote": true, "remoteName": "origin", "isDefault": true},
            {"name": "feature", "worktreePath": "/repo/wt"},
        ],
        "nextCursor": 3, "totalCount": 9, "isRepo": true, "hasPrimaryRemote": true,
    });
    let (fake, git) = checkout(move |_, _, id| vec![ok(id, page.clone())]).await;
    let refs = git.refs("  fea ", 0, 3).await.unwrap();
    assert_eq!(
        refs,
        Refs {
            refs: vec![
                Ref {
                    name: "main".to_owned(),
                    remote: None,
                    current: true,
                    default: true,
                    worktree: None
                },
                Ref {
                    name: "main".to_owned(),
                    remote: Some("origin".to_owned()),
                    current: false,
                    default: true,
                    worktree: None
                },
                Ref {
                    name: "feature".to_owned(),
                    remote: None,
                    current: false,
                    default: false,
                    worktree: Some(PathBuf::from("/repo/wt"))
                },
            ],
            next: Some(3),
            total: 9,
        }
    );
    git.refs(" ", 3, 50).await.unwrap();
    assert_eq!(
        fake.payloads("vcs.listRefs"),
        vec![
            json!({"cwd": CWD, "cursor": 0, "limit": 3, "query": "fea"}),
            json!({"cwd": CWD, "cursor": 3, "limit": 50}),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn pull_says_whether_it_pulled_or_was_up_to_date() {
    let pulled = json!({"status": "pulled", "refName": "main", "upstreamRef": "origin/main"});
    let (_, git) = checkout(move |_, _, id| vec![ok(id, pulled.clone())]).await;
    assert_eq!(
        git.pull().await.unwrap(),
        (
            "Pulled".to_owned(),
            "Updated main from origin/main".to_owned()
        )
    );
    let current = json!({"status": "skipped_up_to_date", "refName": "main", "upstreamRef": null});
    let (_, git) = checkout(move |_, _, id| vec![ok(id, current.clone())]).await;
    assert_eq!(
        git.pull().await.unwrap(),
        (
            "Already up to date".to_owned(),
            "main is already synchronized.".to_owned()
        )
    );
}

/// A stacked action's progress item of the action `action-1`.
fn action(mut item: Value) -> Value {
    item["actionId"] = json!("action-1");
    item
}

/// Collects a stacked action's progress until it ends.
async fn progress(mut receiver: tokio::sync::mpsc::Receiver<GitProgress>) -> Vec<GitProgress> {
    let mut all = Vec::new();
    while let Some(step) = receiver.recv().await {
        all.push(step);
    }
    all
}

#[tokio::test(flavor = "multi_thread")]
async fn every_stacked_action_is_sent_under_its_t3_name() {
    let (fake, git) = checkout(|_, _, _| Vec::new()).await;
    let actions = [
        (GitAction::Commit, "commit"),
        (GitAction::Push, "push"),
        (GitAction::CreatePullRequest, "create_pr"),
        (GitAction::CommitPush, "commit_push"),
        (GitAction::CommitPushPullRequest, "commit_push_pr"),
    ];
    for (count, (action, name)) in actions.into_iter().enumerate() {
        let request = GitActionRequest {
            action,
            message: None,
            files: None,
            feature_branch: false,
        };
        let _progress = git.run(request).await.unwrap();
        fake.received("git.runStackedAction", count + 1).await;
        assert_eq!(fake.payloads("git.runStackedAction")[count]["action"], name);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stacked_action_reports_its_phases_hooks_and_result() {
    let (fake, git) = checkout_in(|_, _, _| Vec::new(), Some("thread-1")).await;
    let receiver = git
        .run(GitActionRequest {
            action: GitAction::CommitPushPullRequest,
            message: Some("  ".to_owned()),
            files: Some(vec!["README.md".to_owned()]),
            feature_branch: true,
        })
        .await
        .unwrap();
    fake.received("git.runStackedAction", 1).await;
    let mut sent = fake.payloads("git.runStackedAction").remove(0);
    sent.as_object_mut().unwrap().remove("actionId");
    assert_eq!(
        sent,
        json!({"cwd": CWD, "action": "commit_push_pr", "filePaths": ["README.md"], "featureBranch": true,
            "threadId": "thread-1"})
    );
    fake.push(
        "git.runStackedAction",
        vec![
            action(json!({"kind": "action_started", "phases": ["commit", "push"]})),
            action(json!({"kind": "phase_started", "phase": "commit", "label": "Committing..."})),
            action(json!({"kind": "hook_started", "hookName": "pre-commit"})),
            action(json!({"kind": "hook_output", "hookName": "pre-commit", "stream": "stdout",
                "text": "linting\nfailed: style"})),
            action(json!({"kind": "hook_finished", "hookName": "pre-commit", "exitCode": 1, "durationMs": 5})),
            action(json!({"kind": "phase_started", "phase": "push", "label": "Pushing..."})),
            action(json!({"kind": "action_finished", "result": {
                "action": "commit_push_pr",
                "toast": {"title": "Committed 3f2a91c", "description": "to feature",
                    "cta": {"kind": "none"}},
                "pr": {"url": "https://example.test/pull/1"},
            }})),
            action(json!({"kind": "action_failed", "phase": null, "message": "not sent"})),
        ],
    );
    assert_eq!(
        progress(receiver).await,
        vec![
            GitProgress::Phase("Committing...".to_owned()),
            GitProgress::Hook("pre-commit".to_owned()),
            GitProgress::Output("failed: style".to_owned()),
            GitProgress::Phase("Pushing...".to_owned()),
            GitProgress::Finished {
                title: "Committed 3f2a91c".to_owned(),
                description: Some("to feature".to_owned()),
                pull_request: Some("https://example.test/pull/1".to_owned()),
            },
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stacked_action_that_fails_says_why_and_ends() {
    let (fake, git) = checkout(|_, _, _| Vec::new()).await;
    let receiver = git
        .run(GitActionRequest {
            action: GitAction::Commit,
            message: Some("fix".to_owned()),
            files: None,
            feature_branch: false,
        })
        .await
        .unwrap();
    fake.received("git.runStackedAction", 1).await;
    fake.push(
        "git.runStackedAction",
        vec![
            json!({"kind": "action_failed", "phase": "commit", "message": "nothing to commit",
            "actionId": "action-1"}),
        ],
    );
    assert_eq!(
        progress(receiver).await,
        vec![GitProgress::Failed("nothing to commit".to_owned())]
    );
    let sent = &fake.payloads("git.runStackedAction")[0];
    assert_eq!(sent["commitMessage"], "fix");
    assert!(sent.get("filePaths").is_none() && sent.get("featureBranch").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stacked_action_that_t3_fails_is_reported_as_failed_with_its_reason() {
    // T3 ends the action with its failure, sending no progress event.
    let (_fake, git) = checkout(|_, _, id| {
        vec![fail(
            id,
            json!({"_tag": "Fail", "error": "nothing to commit"}),
        )]
    })
    .await;
    let receiver = git
        .run(GitActionRequest {
            action: GitAction::Commit,
            message: None,
            files: None,
            feature_branch: false,
        })
        .await
        .unwrap();
    let ended = tokio::time::timeout(std::time::Duration::from_secs(10), progress(receiver))
        .await
        .expect("the action ends");
    assert!(
        matches!(ended.as_slice(), [GitProgress::Failed(message)] if message.contains("nothing to commit")),
        "{ended:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pull_request_reference_resolves_and_checks_out_in_the_thread_or_locally() {
    let pull = json!({"pullRequest": {"number": 42, "title": "Add", "url": "https://example.test/pull/42",
        "baseBranch": "main", "headBranch": "feature", "state": "open"}});
    let in_worktree = json!({"branch": "feature", "worktreePath": "/repo/wt"});
    let in_folder = json!({"branch": "feature", "worktreePath": null});
    let fake = Fake::start(move |tag, payload, id| {
        let reply = match tag {
            "git.resolvePullRequest" => pull.clone(),
            "git.preparePullRequestThread" if payload["mode"] == "worktree" => in_worktree.clone(),
            _ => in_folder.clone(),
        };
        vec![ok(id, reply)]
    })
    .await;
    let client = Client::connect(&fake.origin, || Box::pin(async { Ok("token".to_owned()) }))
        .await
        .unwrap();
    let git = Git::new(client.clone(), CWD.to_owned(), Some("thread-1".to_owned()));
    assert_eq!(
        git.resolve_pull_request("#42").await.unwrap(),
        ResolvedPullRequest {
            number: 42,
            title: "Add".to_owned(),
            url: "https://example.test/pull/42".to_owned(),
            base: "main".to_owned(),
            head: "feature".to_owned(),
            state: "open".to_owned(),
        }
    );
    assert_eq!(
        git.check_out_pull_request("#42", true).await.unwrap(),
        ("feature".to_owned(), Some(PathBuf::from("/repo/wt")))
    );
    let local = Git::new(client, CWD.to_owned(), Some("thread-1".to_owned()));
    assert_eq!(
        local.check_out_pull_request("#42", false).await.unwrap(),
        ("feature".to_owned(), None)
    );
    assert_eq!(
        fake.payloads("git.resolvePullRequest"),
        vec![json!({"cwd": CWD, "reference": "#42"})]
    );
    let prepared = fake.payloads("git.preparePullRequestThread");
    assert_eq!(
        prepared[0],
        json!({"cwd": CWD, "reference": "#42", "mode": "worktree", "threadId": "thread-1"})
    );
    assert_eq!(
        prepared[1],
        json!({"cwd": CWD, "reference": "#42", "mode": "local"})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn publishing_names_the_provider_repository_and_visibility() {
    let (fake, git) = checkout(|_, _, id| vec![ok(id, json!({}))]).await;
    git.publish("github", "owner/repo", true).await.unwrap();
    git.publish("github", "owner/private", false).await.unwrap();
    assert_eq!(
        fake.payloads("sourceControl.publishRepository"),
        vec![
            json!({"cwd": CWD, "provider": "github", "repository": "owner/repo", "visibility": "public"}),
            json!({"cwd": CWD, "provider": "github", "repository": "owner/private", "visibility": "private"}),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_the_status_receiver_interrupts_the_watch_on_t3() {
    let (fake, git) = checkout(|_, _, _| Vec::new()).await;
    let statuses = git.watch().await.unwrap();
    fake.received("subscribeVcsStatus", 1).await;
    let id = fake.request_id("subscribeVcsStatus");
    // T3 pushes nothing, so the watch must notice the missing listener by itself.
    drop(statuses);
    tokio::time::timeout(std::time::Duration::from_secs(10), fake.interrupted(&id))
        .await
        .expect("T3 receives the interrupt");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stacked_action_is_never_sent_again_after_t3_drops_the_connection() {
    let (fake, git) = checkout_in(|_, _, _| Vec::new(), None).await;
    let receiver = git
        .run(GitActionRequest {
            action: GitAction::CommitPush,
            message: None,
            files: None,
            feature_branch: false,
        })
        .await
        .unwrap();
    fake.received("git.runStackedAction", 1).await;
    fake.drop_connection();
    // T3 may have committed or pushed already, so the action ends as failed instead of running
    // twice.
    let ended = tokio::time::timeout(std::time::Duration::from_secs(10), progress(receiver))
        .await
        .expect("the action ends with the connection");
    assert!(
        matches!(ended.as_slice(), [GitProgress::Failed(message)] if message.contains("connection")),
        "{ended:?}"
    );
    fake.accepted(2).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(fake.payloads("git.runStackedAction").len(), 1);
}
