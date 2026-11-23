use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn a_session_moved_again_carries_the_transcript_of_its_earlier_host() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/again"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/again"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let moved = project_b.move_in(package.clone()).await.unwrap();
    wait(&moved, completed(1)).await;

    let again = moved
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append again".to_owned()),
        )
        .await
        .unwrap();
    assert!(
        again
            .transcript
            .iter()
            .any(|item| item.text == "append first"),
        "the earlier host's transcript is carried"
    );
    assert!(
        again
            .transcript
            .iter()
            .all(|item| item.text != package.handoff),
        "the handoff is not carried twice"
    );
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_in_whose_record_cannot_be_kept_stays_retryable_until_it_is_withdrawn() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/unrecorded"]);
    git(
        &repo_a,
        &["push", "-q", "-u", "origin", "feature/unrecorded"],
    );
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    rusqlite::Connection::open(home_b.join("meta-harness.sqlite"))
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER refuse BEFORE INSERT ON move_record \
             BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap();

    let refused = project_b.move_in(package.clone()).await.err().unwrap();
    assert!(refused.contains("refused"), "{refused}");
    assert_eq!(
        host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Starting
    );
    assert_eq!(
        host_b.move_abort(&package.transfer).await.unwrap(),
        Arrival::Refused
    );
    assert!(project_b.threads().await.unwrap().is_empty());
    let worktrees = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&repo_b)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&worktrees.stdout)
            .matches("worktree ")
            .count(),
        2,
        "the launch's worktree stays: a move never removes a worktree"
    );
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_whose_arrival_cannot_be_recorded_keeps_its_worktree_and_refuses() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/late-record"]);
    git(
        &repo_a,
        &["push", "-q", "-u", "origin", "feature/late-record"],
    );
    git(&repo_b, &["fetch", "-q", "origin"]);
    git(
        &repo_b,
        &[
            "branch",
            "feature/late-record",
            "origin/feature/late-record",
        ],
    );
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let raw = rusqlite::Connection::open(home_b.join("meta-harness.sqlite")).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER refuse_row BEFORE INSERT ON arrival \
         WHEN NEW.state = 'starting' BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .unwrap();

    let refused = project_b.move_in(package.clone()).await.err().unwrap();
    assert!(refused.contains("refused"), "{refused}");
    raw.execute_batch("DROP TRIGGER refuse_row").unwrap();
    assert_eq!(
        host_b.move_abort(&package.transfer).await.unwrap(),
        Arrival::Refused
    );
    let worktrees = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&repo_b)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&worktrees.stdout)
            .matches("worktree ")
            .count(),
        2,
        "the worktree the move made stays: a move never removes a worktree"
    );
    service_a.stop().await;
    service_b.stop().await;
}

async fn moving_with_queued(host_a: &remote::Host, repo_a: &Path, texts: &[&str]) -> MovePackage {
    let project_a = host_a.project(repo_a).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    thread.send("wait", false).await.unwrap();
    wait(&thread, |s| {
        s.turns.iter().any(|t| t.status == TurnStatus::Running)
    })
    .await;
    for text in texts {
        thread.send(text, false).await.unwrap();
    }
    let queued = texts.len();
    wait(&thread, move |s| {
        s.turns
            .iter()
            .filter(|t| t.status == TurnStatus::Queued)
            .count()
            == queued
    })
    .await;
    thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap()
}

fn carried_prompts(snapshot: &Snapshot) -> Vec<String> {
    let mut turns: Vec<_> = snapshot
        .turns
        .iter()
        .filter(|t| t.prompt.starts_with("carry "))
        .collect();
    turns.sort_by_key(|t| t.number);
    turns.iter().map(|t| t.prompt.clone()).collect()
}

fn carried_done(snapshot: &Snapshot) -> usize {
    snapshot
        .turns
        .iter()
        .filter(|t| t.prompt.starts_with("carry ") && t.status == TurnStatus::Completed)
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn carried_messages_reach_the_target_only_after_it_lands() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/carried"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/carried"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_b = host_b.project(&repo_b).await.unwrap();
    let package = moving_with_queued(&host_a, &repo_a, &["carry one"]).await;
    let raw = rusqlite::Connection::open(home_b.join("meta-harness.sqlite")).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER refuse_record BEFORE INSERT ON move_record \
         BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .unwrap();
    assert!(project_b.move_in(package.clone()).await.is_err());
    raw.execute_batch("DROP TRIGGER refuse_record").unwrap();
    let listed = project_b.threads().await.unwrap();
    let held = project_b.open(&listed[0].id).await.unwrap();
    let snapshot = wait(&held, completed(1)).await;
    assert!(
        carried_prompts(&snapshot).is_empty(),
        "a carried message reached a session that never landed"
    );
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn carried_messages_run_on_the_landed_session_once_each_and_in_order() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/ordered"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/ordered"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_b = host_b.project(&repo_b).await.unwrap();
    let package = moving_with_queued(&host_a, &repo_a, &["carry one", "carry two"]).await;
    let landed = project_b.move_in(package.clone()).await.unwrap();
    wait(&landed, |s| carried_done(s) == 2).await;
    let snapshot = wait(&landed, |s| carried_done(s) == 2).await;
    assert_eq!(carried_prompts(&snapshot), ["carry one", "carry two"]);
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interrupted_delivery_resumes_on_move_status_and_a_repeated_move_in() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/resumed"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/resumed"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_b = host_b.project(&repo_b).await.unwrap();
    let package = moving_with_queued(&host_a, &repo_a, &["carry one", "carry two"]).await;
    let raw = rusqlite::Connection::open(home_b.join("meta-harness.sqlite")).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER refuse_progress BEFORE UPDATE OF delivered ON arrival \
         WHEN NEW.delivered = 1 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .unwrap();
    assert!(project_b.move_in(package.clone()).await.is_err());
    raw.execute_batch("DROP TRIGGER refuse_progress").unwrap();
    assert!(matches!(
        host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Landed { .. }
    ));
    let listed = project_b.threads().await.unwrap();
    let landed = project_b.open(&listed[0].id).await.unwrap();
    let snapshot = wait(&landed, |s| carried_done(s) == 2).await;
    assert_eq!(carried_prompts(&snapshot), ["carry one", "carry two"]);
    let again = project_b.move_in(package.clone()).await.unwrap();
    assert_eq!(again.id(), landed.id());
    let snapshot = wait(&landed, |s| carried_done(s) == 2).await;
    assert_eq!(carried_prompts(&snapshot), ["carry one", "carry two"]);
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_landed_move_with_an_undelivered_message_answers_starting_until_it_is_delivered() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/unfinished"]);
    git(
        &repo_a,
        &["push", "-q", "-u", "origin", "feature/unfinished"],
    );
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_b = host_b.project(&repo_b).await.unwrap();
    let package = moving_with_queued(&host_a, &repo_a, &["carry one", "carry two"]).await;
    let raw = rusqlite::Connection::open(home_b.join("meta-harness.sqlite")).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER refuse_progress BEFORE UPDATE OF delivered ON arrival \
         WHEN NEW.delivered = 1 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .unwrap();
    assert!(project_b.move_in(package.clone()).await.is_err());
    assert_eq!(
        host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Starting
    );
    assert_eq!(
        host_b.move_abort(&package.transfer).await.unwrap(),
        Arrival::Starting,
        "an abort on a landed transfer never refuses it"
    );
    raw.execute_batch("DROP TRIGGER refuse_progress").unwrap();
    assert!(matches!(
        host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Landed { .. }
    ));
    let listed = project_b.threads().await.unwrap();
    let landed = project_b.open(&listed[0].id).await.unwrap();
    let snapshot = wait(&landed, |s| carried_done(s) == 2).await;
    assert_eq!(carried_prompts(&snapshot), ["carry one", "carry two"]);
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repeated_move_in_is_refused_while_a_carried_message_is_undelivered() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/refused-early"]);
    git(
        &repo_a,
        &["push", "-q", "-u", "origin", "feature/refused-early"],
    );
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_b = host_b.project(&repo_b).await.unwrap();
    let package = moving_with_queued(&host_a, &repo_a, &["carry one", "carry two"]).await;
    let raw = rusqlite::Connection::open(home_b.join("meta-harness.sqlite")).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER refuse_progress BEFORE UPDATE OF delivered ON arrival \
         WHEN NEW.delivered = 1 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .unwrap();
    assert!(project_b.move_in(package.clone()).await.is_err());
    let refused = project_b.move_in(package.clone()).await.err().unwrap();
    assert!(
        refused.contains("not all delivered yet"),
        "a repeat does not finish the move early: {refused}"
    );
    raw.execute_batch("DROP TRIGGER refuse_progress").unwrap();
    let landed = project_b.move_in(package.clone()).await.unwrap();
    let snapshot = wait(&landed, |s| carried_done(s) == 2).await;
    assert_eq!(carried_prompts(&snapshot), ["carry one", "carry two"]);
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_in_repeated_or_concurrent_lands_the_session_once() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/landed"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/landed"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();

    let (first, second) = tokio::join!(
        project_b.move_in(package.clone()),
        project_b.move_in(package.clone())
    );
    let landed = first.unwrap();
    assert_eq!(second.unwrap().id(), landed.id());
    assert_eq!(project_b.threads().await.unwrap().len(), 1);
    let again = project_b.move_in(package.clone()).await.unwrap();
    assert_eq!(again.id(), landed.id());
    assert_eq!(
        host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Landed {
            thread: landed.id().to_owned()
        }
    );
    assert_eq!(
        host_b.move_status("unknown").await.unwrap(),
        Arrival::Unknown
    );
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_moved_back_to_the_host_it_left_continues_in_its_checkout() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/loop"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/loop"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let first = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let there = project_b.move_in(first.clone()).await.unwrap();
    wait(&there, completed(1)).await;
    thread
        .move_finish(
            &first.transfer,
            MoveFinish::Seal {
                to_host: host_b.environment_name().await.unwrap(),
            },
        )
        .await
        .unwrap();

    let back = there
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append again".to_owned()),
        )
        .await
        .unwrap();
    there
        .move_finish(
            &back.transfer,
            MoveFinish::Seal {
                to_host: host_a.environment_name().await.unwrap(),
            },
        )
        .await
        .unwrap();
    let home = project_a.move_in(back.clone()).await.unwrap();
    assert_eq!(
        home.worktree().await.unwrap().as_deref(),
        Some(repo_a.as_path())
    );
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_starting_arrival_launches_its_chosen_thread_id_when_no_thread_exists_yet() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/chosen"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/chosen"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    rusqlite::Connection::open(home_b.join("meta-harness.sqlite"))
        .unwrap()
        .execute(
            "INSERT INTO arrival (transfer, state, root, thread, at_ms) \
             VALUES (?1, 'starting', ?2, 'thread-chosen-before-launch', 0)",
            rusqlite::params![package.transfer, repo_b.to_string_lossy()],
        )
        .unwrap();

    let landed = project_b.move_in(package.clone()).await.unwrap();
    assert_eq!(landed.id(), "thread-chosen-before-launch");
    assert_eq!(
        host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Landed {
            thread: "thread-chosen-before-launch".to_owned()
        }
    );
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_starting_arrival_adopts_the_thread_it_already_launched() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/adopted"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/adopted"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let landed = project_b.move_in(package.clone()).await.unwrap();
    rusqlite::Connection::open(home_b.join("meta-harness.sqlite"))
        .unwrap()
        .execute("UPDATE arrival SET state = 'starting'", [])
        .unwrap();

    let adopted = project_b.move_in(package.clone()).await.unwrap();
    assert_eq!(adopted.id(), landed.id());
    assert_eq!(project_b.threads().await.unwrap().len(), 1);
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_abort_removes_the_thread_keeps_the_worktree_and_refuses_the_transfer() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/withdrawn"]);
    git(
        &repo_a,
        &["push", "-q", "-u", "origin", "feature/withdrawn"],
    );
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let landed = project_b.move_in(package.clone()).await.unwrap();
    let worktree = landed.worktree().await.unwrap().expect("its own worktree");
    rusqlite::Connection::open(home_b.join("meta-harness.sqlite"))
        .unwrap()
        .execute("UPDATE arrival SET state = 'starting'", [])
        .unwrap();
    assert_eq!(
        host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Starting
    );

    assert_eq!(
        host_b.move_abort(&package.transfer).await.unwrap(),
        Arrival::Refused
    );
    assert_eq!(
        host_b.move_abort(&package.transfer).await.unwrap(),
        Arrival::Refused
    );
    assert!(project_b.threads().await.unwrap().is_empty());
    assert!(worktree.exists(), "a move never removes a worktree");
    assert_eq!(
        host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Unknown
    );
    assert!(project_b.move_in(package.clone()).await.is_err());
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_abort_racing_a_landing_answers_what_landed() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/raced"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/raced"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();

    let (arrived, withdrawn) = tokio::join!(
        project_b.move_in(package.clone()),
        host_b.move_abort(&package.transfer)
    );
    match (arrived, withdrawn.unwrap()) {
        (Ok(landed), Arrival::Landed { thread }) => assert_eq!(landed.id(), thread),
        (Err(_), Arrival::Refused) => {}
        (arrived, withdrawn) => panic!(
            "landing {} and abort {withdrawn:?} disagree",
            arrived.is_ok()
        ),
    }
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_record_store_is_retried_on_the_same_thread_and_lands_listed() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/recorded"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/recorded"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let raw = rusqlite::Connection::open(home_b.join("meta-harness.sqlite")).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER refuse BEFORE INSERT ON move_record BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .unwrap();
    assert!(project_b.move_in(package.clone()).await.is_err());
    raw.execute_batch("DROP TRIGGER refuse").unwrap();

    let landed = project_b.move_in(package.clone()).await.unwrap();
    let listed = project_b.threads().await.unwrap();
    assert!(listed.iter().any(|t| t.id == landed.id()));
    assert_eq!(
        host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Landed {
            thread: landed.id().to_owned()
        }
    );
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_withdrawal_interrupted_after_its_delete_refuses_a_late_move_in_and_never_reports_landed()
{
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/withdrawn-late"]);
    git(
        &repo_a,
        &["push", "-q", "-u", "origin", "feature/withdrawn-late"],
    );
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let landed = project_b.move_in(package.clone()).await.unwrap();
    let summary = project_b
        .threads()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id == landed.id())
        .unwrap();
    project_b.delete(&summary, false).await.unwrap();
    let raw = rusqlite::Connection::open(home_b.join("meta-harness.sqlite")).unwrap();
    raw.execute("UPDATE arrival SET state = 'landed'", [])
        .unwrap();
    assert_eq!(
        host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Unknown,
        "a deleted thread is never reported as landed"
    );
    raw.execute("UPDATE arrival SET state = 'starting'", [])
        .unwrap();
    let late = project_b.move_in(package.clone()).await;
    assert!(
        late.as_ref().map_or(true, |t| t.id() != landed.id()),
        "a deleted thread is not adopted"
    );
    service_a.stop().await;
    service_b.stop().await;
}
