use super::*;

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
    let statistics = thread.statistics().await.unwrap();
    assert_eq!(statistics.turns.len(), 1);
    assert_eq!(statistics.turns[0].prompt, "append first");
    let edits: u32 = statistics
        .tools
        .iter()
        .map(|t| t.succeeded + t.failed + t.cancelled + t.open)
        .sum();
    assert!(edits >= 1, "the append's tool call counts: {statistics:?}");
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
    assert_eq!(
        changed.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
        ["contract.txt"]
    );

    assert_eq!(thread.revert(2, true).await.unwrap(), "append second");
    wait(&thread, |s| s.turns.len() == 1).await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\n");

    thread.send("append third", false).await.unwrap();
    wait(&thread, completed(2)).await;
    let numbers: Vec<u64> = thread
        .turns()
        .await
        .unwrap()
        .iter()
        .map(|t| t.number)
        .collect();
    assert_eq!(numbers, [1, 3]);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\nthird\n");
    assert!(thread.diff(3, 3, false).await.unwrap().contains("+third"));
    assert_eq!(thread.revert(3, true).await.unwrap(), "append third");
    wait(&thread, |s| s.turns.len() == 1).await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\n");

    assert_eq!(thread.revert(1, true).await.unwrap(), "append first");
    wait(&thread, |s| s.turns.is_empty()).await;
    assert!(!file.exists());

    thread.set_branch("main", Some(&repo)).await.unwrap();
    assert_eq!(
        thread.worktree().await.unwrap().as_deref(),
        Some(repo.as_path())
    );

    let plan = project
        .start(start("plan", worktree(), Permissions::FullAccess))
        .await
        .unwrap();
    let snapshot = wait(&plan, completed(1)).await;
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
    assert!(after.transcript.iter().any(|item| item.text == "Appended."));
    host.shutdown().await;
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
    let (asked, mut events) = {
        let snapshot = wait(&thread, |s| !s.requests.is_empty()).await;
        (
            snapshot.requests[0].clone(),
            thread.subscribe().await.unwrap().1,
        )
    };
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
    assert!(done.transcript.iter().any(|item| item.text == "Approved."));
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_sent_while_a_turn_runs_queues_and_can_be_removed() {
    let (_dir, repo, home) = fixture();
    let (service, host) = start_service(&home).await;
    let project = host.project(&repo).await.unwrap();
    let thread = project
        .start(start("wait", Workspace::Checkout, Permissions::FullAccess))
        .await
        .unwrap();
    until_running(&thread).await;
    thread.send("append later", false).await.unwrap();
    thread.send("append more", false).await.unwrap();
    let queued = wait(&thread, |s| {
        s.turns
            .iter()
            .filter(|t| t.status == TurnStatus::Queued && !t.prompt.is_empty())
            .count()
            == 2
    })
    .await;
    let [later, more] = [0, 1].map(|i| {
        queued
            .turns
            .iter()
            .filter(|t| t.status == TurnStatus::Queued)
            .nth(i)
            .unwrap()
            .clone()
    });
    assert_eq!(
        (later.prompt.as_str(), more.prompt.as_str()),
        ("append later", "append more")
    );
    thread
        .edit_queued(more.number, "append edited")
        .await
        .unwrap();
    wait(&thread, |s| {
        s.turns.iter().any(|t| t.prompt == "append edited")
    })
    .await;
    thread.cancel_queued(more.number).await.unwrap();
    thread.cancel_queued(later.number).await.unwrap();
    wait(&thread, |s| {
        s.turns.iter().all(|t| t.status != TurnStatus::Queued)
    })
    .await;
    thread.stop().await.unwrap();
    wait(&thread, |s| {
        s.turns.iter().all(|t| t.status == TurnStatus::Aborted)
    })
    .await;
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
    assert!(thread.servers().await.unwrap().current().is_empty());
    assert!(thread.automations().await.unwrap().current().is_empty());
    thread.usage_limits().await.unwrap();
    thread.send("append second", false).await.unwrap();
    wait(&thread, completed(2)).await;

    let fork_id = thread.fork(1).await.unwrap();
    let fork = project.open(&fork_id).await.unwrap();
    let forked = wait(&fork, |s| {
        s.transcript.iter().any(|i| i.text == "append first")
    })
    .await;
    assert!(forked.transcript.iter().all(|i| i.text != "append second"));
    fork.send("append in fork", false).await.unwrap();
    wait(&fork, |s| {
        s.turns.iter().any(|t| {
            t.prompt == "append in fork" && t.status == TurnStatus::Completed && t.checkpoint
        })
    })
    .await;

    let parent = thread.lineage().await.unwrap();
    assert!(parent.rows.iter().any(|row| row.thread == fork_id
        && row.relation == Relation::Fork
        && row.group == Group::Related));
    let lineage = fork.lineage().await.unwrap();
    let parent_row = lineage.rows.iter().find(|row| row.thread == thread.id());
    assert!(parent_row.is_some_and(|row| row.merge_target));
    assert!(lineage.can_merge);
    fork.merge_back().await.unwrap();
    service.stop().await;
}
