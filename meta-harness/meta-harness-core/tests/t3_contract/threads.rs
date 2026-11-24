use super::*;
use meta_harness_core::{ItemKind, StepStatus, TaskStep, Thread, ThreadEvent};

async fn wait_core(thread: &Thread, done: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let waited = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let (snapshot, mut events) = thread.subscribe();
            if done(&snapshot) {
                return snapshot;
            }
            events.recv().await;
        }
    })
    .await;
    waited.unwrap_or_else(|_| panic!("not there within a minute: {:#?}", thread.snapshot()))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_runs_turns_and_reverts_files_in_its_worktree() {
    let (_dir, repo, home) = fixture();
    let (service, host) = start_service(&home).await;
    let project = host.project(&repo).await.unwrap();
    let thread = project
        .start(start("append first", worktree(), Permissions::FullAccess))
        .await
        .unwrap();
    wait(&thread, completed(1)).await;
    assert_eq!(thread.statistics().await.unwrap().turns.len(), 1);
    thread.send("append second", false).await.unwrap();
    wait(&thread, completed(2)).await;

    let folder = thread
        .worktree()
        .await
        .unwrap()
        .expect("the thread has its own folder");
    let file = folder.join("contract.txt");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\nsecond\n");
    let turns = thread.turns().await.unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(
        (turns[1].number, turns[1].added, turns[1].removed),
        (2, 1, 0)
    );
    assert!(thread.diff(2, 2, false).await.unwrap().contains("+second"));
    let changed = thread.git().status().await.unwrap().changes;
    let paths: Vec<&str> = changed.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(paths, ["contract.txt"]);

    assert_eq!(thread.revert(2, true).await.unwrap(), "append second");
    wait(&thread, |s| s.turns.len() == 1).await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\n");

    thread.send("append third", false).await.unwrap();
    wait(&thread, completed(2)).await;
    let turns = thread.turns().await.unwrap();
    let numbers: Vec<u64> = turns.iter().map(|t| t.number).collect();
    assert_eq!(numbers, [1, 3]);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\nthird\n");
    assert!(thread.diff(3, 3, false).await.unwrap().contains("+third"));
    assert_eq!(thread.revert(3, true).await.unwrap(), "append third");
    wait(&thread, |s| s.turns.len() == 1).await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\n");

    assert_eq!(thread.revert(1, true).await.unwrap(), "append first");
    wait(&thread, |s| s.turns.is_empty()).await;
    assert!(!file.exists());

    thread.send("plan", false).await.unwrap();
    let snapshot = wait(&thread, completed(1)).await;
    let tasks = snapshot
        .transcript
        .iter()
        .find(|item| item.kind == ItemKind::TaskList)
        .expect("the plan is a task list item");
    let step = |text: &str| TaskStep {
        text: text.to_owned(),
        status: StepStatus::Completed,
    };
    assert_eq!(
        tasks.detail,
        ItemDetail::Tasks {
            steps: vec![step("Explore"), step("Explore › scraper-1")]
        }
    );

    use std::os::unix::ffi::OsStrExt;
    let not_utf8 = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff"));
    assert_refused(host.project(not_utf8).await, "not valid UTF-8");
    assert_refused(
        thread.set_branch("main", Some(not_utf8)).await,
        "not valid UTF-8",
    );
    thread.set_branch("main", Some(&repo)).await.unwrap();
    assert_eq!(
        thread.worktree().await.unwrap().as_deref(),
        Some(repo.as_path())
    );

    let owned = project
        .start(start("append one", worktree(), Permissions::FullAccess))
        .await
        .unwrap();
    wait(&owned, completed(1)).await;
    let listed = project.threads().await.unwrap();
    let summary = listed.iter().find(|t| t.id == owned.id()).unwrap();
    let folder = summary.worktree.clone().unwrap();
    assert!(project.only_thread_in_worktree(summary).await.unwrap());
    project.delete(summary, true).await.unwrap();
    assert!(!folder.exists());
    assert!(
        project
            .threads()
            .await
            .unwrap()
            .iter()
            .all(|t| t.id != owned.id())
    );
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_supervised_thread_waits_for_the_person_to_approve() {
    let (_dir, repo, home) = fixture();
    let (service, host) = start_service(&home).await;
    let project = host.project(&repo).await.unwrap();
    let thread = project
        .start(start("ask", Workspace::Checkout, Permissions::Supervised))
        .await
        .unwrap();
    let asked = wait(&thread, |s| !s.requests.is_empty()).await.requests[0].clone();
    let mut events = thread.subscribe().await.unwrap().1;
    let Request::Approval { options, .. } = &asked.request else {
        panic!("an approval, not {asked:?}");
    };
    assert!(options.iter().any(|o| o.decision == "accept"));
    thread.respond(&asked.id, "accept").await.unwrap();
    let resolved = events.recv().await.unwrap();
    let mut seen = vec![resolved.event];
    let done = wait(&thread, completed(1)).await;
    seen.extend(std::iter::from_fn(|| events.recv().now_or_never().flatten()).map(|e| e.event));
    assert!(seen.contains(&AgentEvent::RequestResolved {
        request: asked.id.clone()
    }));
    assert!(done.requests.is_empty());
    assert!(says(&done.transcript, "Approved."));
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_details_card_reads_t3_and_a_fork_merges_back() {
    let (_dir, repo, home) = fixture();
    let (service, host) = start_service(&home).await;
    assert!(!host.environment_name().await.unwrap().is_empty());
    let project = host.project(&repo).await.unwrap();
    let thread = project
        .start(start("append first", worktree(), Permissions::FullAccess))
        .await
        .unwrap();
    wait(&thread, completed(1)).await;
    thread.usage_limits().await.unwrap();
    thread.send("append second", false).await.unwrap();
    wait(&thread, completed(2)).await;

    let fork_id = thread.fork(1).await.unwrap();
    let fork = project.open(&fork_id).await.unwrap();
    let forked = wait(&fork, |s| says(&s.transcript, "append first")).await;
    assert!(!says(&forked.transcript, "append second"));
    fork.send("append in fork", false).await.unwrap();
    wait(&fork, |s| {
        s.turns.iter().any(|t| {
            t.prompt == "append in fork" && t.status == TurnStatus::Completed && t.checkpoint
        })
    })
    .await;

    fork.merge_back().await.unwrap();
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_snapshot_names_the_events_it_already_contains() {
    let (_dir, repo, home) = fixture();
    let host = host(&home).await;
    let project = host.project(&repo).await.unwrap();
    let thread = project
        .start(start("append once", worktree(), Permissions::FullAccess))
        .await
        .unwrap();
    let (before, mut events) = thread.subscribe();
    wait_core(&thread, completed(1)).await;
    let received: Vec<ThreadEvent> = std::iter::from_fn(|| events.try_recv().ok()).collect();
    let after = thread.snapshot();
    assert!(
        received
            .iter()
            .all(|e| before.sequence < e.sequence && e.sequence <= after.sequence)
    );
    assert!(
        received
            .windows(2)
            .all(|pair| pair[0].sequence <= pair[1].sequence)
    );
    let finished_before = before
        .turns
        .iter()
        .any(|t| t.status == TurnStatus::Completed);
    let completions_after = received
        .iter()
        .filter(|e| matches!(e.event, AgentEvent::TurnCompleted { .. }))
        .count();
    assert_eq!(
        usize::from(finished_before) + completions_after,
        1,
        "the completion is in the snapshot or the stream, exactly once"
    );
    assert!(says(&after.transcript, "Appended."));
    host.shutdown().await;
}
