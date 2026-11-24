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
    let thread = appended(&host_a.project(repo_a).await.unwrap()).await;
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
    handoff(&thread).await
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
    pushed_branch(&pair.repo_a, "feature/unrecorded");
    let home_b = pair.dir.path().join("host-b");
    let (project_a, project_b) = pair.projects().await;
    let package = handoff(&appended(&project_a).await).await;
    let token = package.transfer.as_str();
    refuse(&home_b, "refuse", "BEFORE INSERT ON move_record");

    assert_refused(project_b.move_in(package.clone()).await, "refused");
    assert_eq!(arrival(&pair.host_b, token).await, Arrival::Starting);
    assert_eq!(withdraw(&pair.host_b, token).await, Arrival::Refused);
    assert!(project_b.threads().await.unwrap().is_empty());
    assert_eq!(
        worktree_count(&pair.repo_b),
        2,
        "a move never removes a worktree"
    );
    pair.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn carried_messages_reach_the_target_only_after_it_lands_and_once_each_in_order() {
    let pair = Pair::codex().await;
    pushed_branch(&pair.repo_a, "feature/resumed");
    let home_b = pair.dir.path().join("host-b");
    let project_b = pair.host_b.project(&pair.repo_b).await.unwrap();
    let package = moving_with_queued(&pair.host_a, &pair.repo_a, &["carry one", "carry two"]).await;
    let token = package.transfer.as_str();
    refuse(&home_b, "refuse_record", "BEFORE INSERT ON move_record");
    assert!(project_b.move_in(package.clone()).await.is_err());
    let listed = project_b.threads().await.unwrap();
    let held = project_b.open(&listed[0].id).await.unwrap();
    let snapshot = wait(&held, completed(1)).await;
    assert!(
        carried_prompts(&snapshot).is_empty(),
        "a carried message reached a session that never landed"
    );
    exec_sql(&home_b, "DROP TRIGGER refuse_record");
    refuse(
        &home_b,
        "refuse_progress",
        "BEFORE UPDATE OF delivered ON arrival WHEN NEW.delivered = 1",
    );
    assert!(project_b.move_in(package.clone()).await.is_err());
    assert_eq!(arrival(&pair.host_b, token).await, Arrival::Starting);
    assert_eq!(
        withdraw(&pair.host_b, token).await,
        Arrival::Starting,
        "an abort on a landed transfer never refuses it"
    );
    assert_refused(
        project_b.move_in(package.clone()).await,
        "not all delivered yet",
    );
    exec_sql(&home_b, "DROP TRIGGER refuse_progress");
    assert!(matches!(
        arrival(&pair.host_b, token).await,
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
    pushed_branch(&pair.repo_a, "feature/landed");
    let (project_a, project_b) = pair.projects().await;
    let package = handoff(&appended(&project_a).await).await;
    let token = package.transfer.as_str();

    let (first, second) = tokio::join!(
        project_b.move_in(package.clone()),
        project_b.move_in(package.clone())
    );
    let landed = first.unwrap();
    assert_eq!(second.unwrap().id(), landed.id());
    assert_eq!(project_b.threads().await.unwrap().len(), 1);
    let again = project_b.move_in(package.clone()).await.unwrap();
    assert_eq!(again.id(), landed.id());
    let thread = landed.id().to_owned();
    assert_eq!(
        arrival(&pair.host_b, token).await,
        Arrival::Landed { thread }
    );
    assert_eq!(arrival(&pair.host_b, "unknown").await, Arrival::Unknown);
    pair.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_starting_arrival_launches_its_chosen_thread_id_and_adopts_it_once_it_exists() {
    let pair = Pair::codex().await;
    pushed_branch(&pair.repo_a, "feature/chosen");
    let home_b = pair.dir.path().join("host-b");
    let (project_a, project_b) = pair.projects().await;
    let package = handoff(&appended(&project_a).await).await;
    let token = package.transfer.as_str();
    db(&home_b)
        .execute(
            "INSERT INTO arrival (transfer, state, root, thread, at_ms) \
             VALUES (?1, 'starting', ?2, 'thread-chosen-before-launch', 0)",
            rusqlite::params![token, pair.repo_b.to_string_lossy()],
        )
        .unwrap();

    let landed = project_b.move_in(package.clone()).await.unwrap();
    assert_eq!(landed.id(), "thread-chosen-before-launch");
    let thread = "thread-chosen-before-launch".to_owned();
    assert_eq!(
        arrival(&pair.host_b, token).await,
        Arrival::Landed { thread }
    );

    exec_sql(&home_b, "UPDATE arrival SET state = 'starting'");
    let adopted = project_b.move_in(package.clone()).await.unwrap();
    assert_eq!(adopted.id(), landed.id());
    assert_eq!(project_b.threads().await.unwrap().len(), 1);
    pair.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_abort_removes_the_thread_keeps_the_worktree_and_refuses_the_transfer() {
    let pair = Pair::codex().await;
    pushed_branch(&pair.repo_a, "feature/withdrawn");
    let home_b = pair.dir.path().join("host-b");
    let (project_a, project_b) = pair.projects().await;
    let package = handoff(&appended(&project_a).await).await;
    let token = package.transfer.as_str();
    let landed = project_b.move_in(package.clone()).await.unwrap();
    let worktree = landed.worktree().await.unwrap().expect("its own worktree");
    exec_sql(&home_b, "UPDATE arrival SET state = 'starting'");
    assert_eq!(arrival(&pair.host_b, token).await, Arrival::Starting);

    assert_eq!(withdraw(&pair.host_b, token).await, Arrival::Refused);
    assert_eq!(withdraw(&pair.host_b, token).await, Arrival::Refused);
    assert!(project_b.threads().await.unwrap().is_empty());
    assert!(worktree.exists(), "a move never removes a worktree");
    assert_eq!(arrival(&pair.host_b, token).await, Arrival::Unknown);
    assert!(project_b.move_in(package.clone()).await.is_err());
    pair.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_abort_racing_a_landing_answers_what_landed() {
    let pair = Pair::codex().await;
    pushed_branch(&pair.repo_a, "feature/raced");
    let (project_a, project_b) = pair.projects().await;
    let package = handoff(&appended(&project_a).await).await;

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
    pushed_branch(&pair.repo_a, "feature/recorded");
    let home_b = pair.dir.path().join("host-b");
    let (project_a, project_b) = pair.projects().await;
    let package = handoff(&appended(&project_a).await).await;
    let token = package.transfer.as_str();
    refuse(&home_b, "refuse", "BEFORE INSERT ON move_record");
    assert!(project_b.move_in(package.clone()).await.is_err());
    exec_sql(&home_b, "DROP TRIGGER refuse");

    let landed = project_b.move_in(package.clone()).await.unwrap();
    let listed = project_b.threads().await.unwrap();
    assert!(listed.iter().any(|t| t.id == landed.id()));
    let thread = landed.id().to_owned();
    assert_eq!(
        arrival(&pair.host_b, token).await,
        Arrival::Landed { thread }
    );
    pair.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_withdrawal_interrupted_after_its_delete_refuses_a_late_move_in_and_never_reports_landed()
{
    let pair = Pair::codex().await;
    pushed_branch(&pair.repo_a, "feature/withdrawn-late");
    let home_b = pair.dir.path().join("host-b");
    let (project_a, project_b) = pair.projects().await;
    let package = handoff(&appended(&project_a).await).await;
    let token = package.transfer.as_str();
    let landed = project_b.move_in(package.clone()).await.unwrap();
    let summary = project_b
        .threads()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id == landed.id())
        .unwrap();
    project_b.delete(&summary, false).await.unwrap();
    exec_sql(&home_b, "UPDATE arrival SET state = 'landed'");
    assert_eq!(
        arrival(&pair.host_b, token).await,
        Arrival::Unknown,
        "a deleted thread is never reported as landed"
    );
    exec_sql(&home_b, "UPDATE arrival SET state = 'starting'");
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
    pushed_branch(&pair.repo_a, "feature/loop");
    let (project_a, project_b) = pair.projects().await;
    let thread = appended(&project_a).await;
    let first = handoff(&thread).await;
    let there = project_b.move_in(first.clone()).await.unwrap();
    wait(&there, completed(1)).await;

    let again = move_out(&there, &transfer(), Some("Append again"))
        .await
        .unwrap();
    assert!(
        says(&again.transcript, "append first"),
        "the earlier host's transcript is carried"
    );
    assert!(
        !says(&again.transcript, &first.handoff),
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
    pushed_branch(&pair.repo_a, "feature/late-record");
    local_from_origin(&pair.repo_b, "feature/late-record");
    let home_b = pair.dir.path().join("host-b");
    let (project_a, project_b) = pair.projects().await;
    let package = handoff(&appended(&project_a).await).await;
    refuse(
        &home_b,
        "refuse_row",
        "BEFORE INSERT ON arrival WHEN NEW.state = 'starting'",
    );

    assert_refused(project_b.move_in(package.clone()).await, "refused");
    exec_sql(&home_b, "DROP TRIGGER refuse_row");
    assert_eq!(
        withdraw(&pair.host_b, &package.transfer).await,
        Arrival::Refused
    );
    assert_eq!(
        worktree_count(&pair.repo_b),
        2,
        "the worktree the move made stays"
    );
    pair.stop().await;
}
