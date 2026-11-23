use super::*;

fn worktree_count(repo: &Path) -> usize {
    let listing = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo)
        .output()
        .unwrap();
    String::from_utf8_lossy(&listing.stdout)
        .matches("worktree ")
        .count()
}

async fn moving_with_queued(host_a: &remote::Host, repo_a: &Path, texts: &[&str]) -> MovePackage {
    let project_a = host_a.project(repo_a).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    hold_turn(&thread).await;
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
    move_out(&thread, &transfer(), Some("Append"))
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
async fn a_move_in_whose_record_cannot_be_kept_stays_retryable_until_it_is_withdrawn() {
    let pair = Pair::codex().await;
    git(
        &pair.repo_a,
        &["checkout", "-q", "-b", "feature/unrecorded"],
    );
    git(
        &pair.repo_a,
        &["push", "-q", "-u", "origin", "feature/unrecorded"],
    );
    let project_a = pair.host_a.project(&pair.repo_a).await.unwrap();
    let project_b = pair.host_b.project(&pair.repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = move_out(&thread, &transfer(), Some("Append"))
        .await
        .unwrap();
    rusqlite::Connection::open(pair.dir.path().join("host-b/meta-harness.sqlite"))
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER refuse BEFORE INSERT ON move_record \
             BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap();

    let refused = project_b.move_in(package.clone()).await.err().unwrap();
    assert!(refused.contains("refused"), "{refused}");
    assert_eq!(
        pair.host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Starting
    );
    assert_eq!(
        pair.host_b.move_abort(&package.transfer).await.unwrap(),
        Arrival::Refused
    );
    assert!(project_b.threads().await.unwrap().is_empty());
    assert_eq!(
        worktree_count(&pair.repo_b),
        2,
        "the launch's worktree stays: a move never removes a worktree"
    );
    pair.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn carried_messages_reach_the_target_only_after_it_lands_and_once_each_in_order() {
    let pair = Pair::codex().await;
    git(&pair.repo_a, &["checkout", "-q", "-b", "feature/resumed"]);
    git(
        &pair.repo_a,
        &["push", "-q", "-u", "origin", "feature/resumed"],
    );
    let project_b = pair.host_b.project(&pair.repo_b).await.unwrap();
    let home_b = pair.dir.path().join("host-b");
    let package = moving_with_queued(&pair.host_a, &pair.repo_a, &["carry one", "carry two"]).await;
    let raw = rusqlite::Connection::open(home_b.join("meta-harness.sqlite")).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER refuse_record BEFORE INSERT ON move_record \
         BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .unwrap();
    assert!(project_b.move_in(package.clone()).await.is_err());
    let listed = project_b.threads().await.unwrap();
    let held = project_b.open(&listed[0].id).await.unwrap();
    let snapshot = wait(&held, completed(1)).await;
    assert!(
        carried_prompts(&snapshot).is_empty(),
        "a carried message reached a session that never landed"
    );
    raw.execute_batch("DROP TRIGGER refuse_record").unwrap();
    raw.execute_batch(
        "CREATE TRIGGER refuse_progress BEFORE UPDATE OF delivered ON arrival \
         WHEN NEW.delivered = 1 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .unwrap();
    assert!(project_b.move_in(package.clone()).await.is_err());
    assert_eq!(
        pair.host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Starting
    );
    assert_eq!(
        pair.host_b.move_abort(&package.transfer).await.unwrap(),
        Arrival::Starting,
        "an abort on a landed transfer never refuses it"
    );
    let refused = project_b.move_in(package.clone()).await.err().unwrap();
    assert!(
        refused.contains("not all delivered yet"),
        "a repeat does not finish the move early: {refused}"
    );
    raw.execute_batch("DROP TRIGGER refuse_progress").unwrap();
    assert!(matches!(
        pair.host_b.move_status(&package.transfer).await.unwrap(),
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
    pair.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_in_repeated_or_concurrent_lands_the_session_once() {
    let pair = Pair::codex().await;
    git(&pair.repo_a, &["checkout", "-q", "-b", "feature/landed"]);
    git(
        &pair.repo_a,
        &["push", "-q", "-u", "origin", "feature/landed"],
    );
    let project_a = pair.host_a.project(&pair.repo_a).await.unwrap();
    let project_b = pair.host_b.project(&pair.repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = move_out(&thread, &transfer(), Some("Append"))
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
        pair.host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Landed {
            thread: landed.id().to_owned()
        }
    );
    assert_eq!(
        pair.host_b.move_status("unknown").await.unwrap(),
        Arrival::Unknown
    );
    pair.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_starting_arrival_launches_its_chosen_thread_id_and_adopts_it_once_it_exists() {
    let pair = Pair::codex().await;
    git(&pair.repo_a, &["checkout", "-q", "-b", "feature/chosen"]);
    git(
        &pair.repo_a,
        &["push", "-q", "-u", "origin", "feature/chosen"],
    );
    let home_b = pair.dir.path().join("host-b");
    let project_a = pair.host_a.project(&pair.repo_a).await.unwrap();
    let project_b = pair.host_b.project(&pair.repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = move_out(&thread, &transfer(), Some("Append"))
        .await
        .unwrap();
    rusqlite::Connection::open(home_b.join("meta-harness.sqlite"))
        .unwrap()
        .execute(
            "INSERT INTO arrival (transfer, state, root, thread, at_ms) \
             VALUES (?1, 'starting', ?2, 'thread-chosen-before-launch', 0)",
            rusqlite::params![package.transfer, pair.repo_b.to_string_lossy()],
        )
        .unwrap();

    let landed = project_b.move_in(package.clone()).await.unwrap();
    assert_eq!(landed.id(), "thread-chosen-before-launch");
    assert_eq!(
        pair.host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Landed {
            thread: "thread-chosen-before-launch".to_owned()
        }
    );

    rusqlite::Connection::open(home_b.join("meta-harness.sqlite"))
        .unwrap()
        .execute("UPDATE arrival SET state = 'starting'", [])
        .unwrap();
    let adopted = project_b.move_in(package.clone()).await.unwrap();
    assert_eq!(adopted.id(), landed.id());
    assert_eq!(project_b.threads().await.unwrap().len(), 1);
    pair.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_abort_removes_the_thread_keeps_the_worktree_and_refuses_the_transfer() {
    let pair = Pair::codex().await;
    git(&pair.repo_a, &["checkout", "-q", "-b", "feature/withdrawn"]);
    git(
        &pair.repo_a,
        &["push", "-q", "-u", "origin", "feature/withdrawn"],
    );
    let home_b = pair.dir.path().join("host-b");
    let project_a = pair.host_a.project(&pair.repo_a).await.unwrap();
    let project_b = pair.host_b.project(&pair.repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = move_out(&thread, &transfer(), Some("Append"))
        .await
        .unwrap();
    let landed = project_b.move_in(package.clone()).await.unwrap();
    let worktree = landed.worktree().await.unwrap().expect("its own worktree");
    rusqlite::Connection::open(home_b.join("meta-harness.sqlite"))
        .unwrap()
        .execute("UPDATE arrival SET state = 'starting'", [])
        .unwrap();
    assert_eq!(
        pair.host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Starting
    );

    assert_eq!(
        pair.host_b.move_abort(&package.transfer).await.unwrap(),
        Arrival::Refused
    );
    assert_eq!(
        pair.host_b.move_abort(&package.transfer).await.unwrap(),
        Arrival::Refused
    );
    assert!(project_b.threads().await.unwrap().is_empty());
    assert!(worktree.exists(), "a move never removes a worktree");
    assert_eq!(
        pair.host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Unknown
    );
    assert!(project_b.move_in(package.clone()).await.is_err());
    pair.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_abort_racing_a_landing_answers_what_landed() {
    let pair = Pair::codex().await;
    git(&pair.repo_a, &["checkout", "-q", "-b", "feature/raced"]);
    git(
        &pair.repo_a,
        &["push", "-q", "-u", "origin", "feature/raced"],
    );
    let project_a = pair.host_a.project(&pair.repo_a).await.unwrap();
    let project_b = pair.host_b.project(&pair.repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = move_out(&thread, &transfer(), Some("Append"))
        .await
        .unwrap();

    let (arrived, withdrawn) = tokio::join!(
        project_b.move_in(package.clone()),
        pair.host_b.move_abort(&package.transfer)
    );
    match (arrived, withdrawn.unwrap()) {
        (Ok(landed), Arrival::Landed { thread }) => assert_eq!(landed.id(), thread),
        (Err(_), Arrival::Refused) => {}
        (arrived, withdrawn) => panic!(
            "landing {} and abort {withdrawn:?} disagree",
            arrived.is_ok()
        ),
    }
    pair.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_record_store_is_retried_on_the_same_thread_and_lands_listed() {
    let pair = Pair::codex().await;
    git(&pair.repo_a, &["checkout", "-q", "-b", "feature/recorded"]);
    git(
        &pair.repo_a,
        &["push", "-q", "-u", "origin", "feature/recorded"],
    );
    let home_b = pair.dir.path().join("host-b");
    let project_a = pair.host_a.project(&pair.repo_a).await.unwrap();
    let project_b = pair.host_b.project(&pair.repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = move_out(&thread, &transfer(), Some("Append"))
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
        pair.host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Landed {
            thread: landed.id().to_owned()
        }
    );
    pair.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_withdrawal_interrupted_after_its_delete_refuses_a_late_move_in_and_never_reports_landed()
{
    let pair = Pair::codex().await;
    git(
        &pair.repo_a,
        &["checkout", "-q", "-b", "feature/withdrawn-late"],
    );
    git(
        &pair.repo_a,
        &["push", "-q", "-u", "origin", "feature/withdrawn-late"],
    );
    let home_b = pair.dir.path().join("host-b");
    let project_a = pair.host_a.project(&pair.repo_a).await.unwrap();
    let project_b = pair.host_b.project(&pair.repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = move_out(&thread, &transfer(), Some("Append"))
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
        pair.host_b.move_status(&package.transfer).await.unwrap(),
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
    pair.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_moved_again_carries_its_transcript_and_moves_back_to_the_host_it_left() {
    let pair = Pair::codex().await;
    git(&pair.repo_a, &["checkout", "-q", "-b", "feature/loop"]);
    git(
        &pair.repo_a,
        &["push", "-q", "-u", "origin", "feature/loop"],
    );
    let project_a = pair.host_a.project(&pair.repo_a).await.unwrap();
    let project_b = pair.host_b.project(&pair.repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let first = move_out(&thread, &transfer(), Some("Append"))
        .await
        .unwrap();
    let there = project_b.move_in(first.clone()).await.unwrap();
    wait(&there, completed(1)).await;

    let again = move_out(&there, &transfer(), Some("Append again"))
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
            .all(|item| item.text != first.handoff),
        "the handoff is not carried twice"
    );

    thread
        .move_finish(
            &first.transfer,
            MoveFinish::Seal {
                to_host: pair.host_b.environment_name().await.unwrap(),
            },
        )
        .await
        .unwrap();
    there
        .move_finish(
            &again.transfer,
            MoveFinish::Seal {
                to_host: pair.host_a.environment_name().await.unwrap(),
            },
        )
        .await
        .unwrap();
    let home = project_a.move_in(again.clone()).await.unwrap();
    assert_eq!(
        home.worktree().await.unwrap().as_deref(),
        Some(pair.repo_a.as_path())
    );
    pair.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_whose_arrival_cannot_be_recorded_keeps_its_worktree_and_refuses() {
    let pair = Pair::codex().await;
    git(
        &pair.repo_a,
        &["checkout", "-q", "-b", "feature/late-record"],
    );
    git(
        &pair.repo_a,
        &["push", "-q", "-u", "origin", "feature/late-record"],
    );
    git(&pair.repo_b, &["fetch", "-q", "origin"]);
    git(
        &pair.repo_b,
        &[
            "branch",
            "feature/late-record",
            "origin/feature/late-record",
        ],
    );
    let home_b = pair.dir.path().join("host-b");
    let project_a = pair.host_a.project(&pair.repo_a).await.unwrap();
    let project_b = pair.host_b.project(&pair.repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = move_out(&thread, &transfer(), Some("Append"))
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
        pair.host_b.move_abort(&package.transfer).await.unwrap(),
        Arrival::Refused
    );
    assert_eq!(
        worktree_count(&pair.repo_b),
        2,
        "the worktree the move made stays: a move never removes a worktree"
    );
    pair.stop().await;
}
