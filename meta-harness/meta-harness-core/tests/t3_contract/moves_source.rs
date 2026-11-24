use super::*;

async fn one_host(dir: &tempdir::Dir) -> (PathBuf, Service, remote::Host) {
    let (origin, url) = shared_remote(dir.path());
    let repo = clone_project(dir.path(), &origin, &url, "a");
    let (service, host) = start_codex(&dir.path().join("host-a")).await;
    (repo, service, host)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_moves_to_another_host_with_its_branch_and_its_source_is_sealed() {
    let pair = Pair::codex().await;
    pushed_branch(&pair.repo_a, "feature/move");
    let (project_a, project_b) = pair.projects().await;
    let thread = appended(&project_a).await;
    let listed = project_a.threads().await.unwrap();
    let summary = listed.iter().find(|t| t.id == thread.id()).unwrap();
    assert_eq!(summary.harness, Some(Harness::Codex));
    assert!(summary.harness_known, "the service reports the harness");

    hold_turn(&thread).await;
    thread.send("append one", false).await.unwrap();
    thread.send("append two", false).await.unwrap();
    wait(&thread, |s| {
        s.turns
            .iter()
            .filter(|t| t.status == TurnStatus::Queued && !t.prompt.is_empty())
            .count()
            == 2
    })
    .await;
    let check = thread.move_check().await.unwrap();
    assert_eq!(check.harness, Some(Harness::Codex));
    assert_eq!(check.branch.as_deref(), Some("feature/move"));
    assert_eq!((check.uncommitted_files, check.running), (1, true));
    assert_eq!(check.remote_url.as_deref(), Some(pair.url.as_str()));
    assert_eq!(check.blocked, None);

    let package = move_out(&thread, &transfer(), Some("Append the contract text"))
        .await
        .unwrap();
    assert_eq!(package.branch, "feature/move");
    assert_eq!(package.remote_url, *pair.url);
    assert_eq!(package.queued, ["append one", "append two"]);
    let turns = thread.turns().await.unwrap();
    assert!(turns.iter().all(|t| t.status != TurnStatus::Queued));
    assert!(package.handoff.contains("feature/move"));
    assert!(
        package
            .transcript
            .iter()
            .any(|item| item.text == "append first")
    );
    assert!(!package.transcript_capped);
    assert_eq!(
        pushed_subject(&pair.origin, "feature/move"),
        "Append the contract text"
    );
    let refused = thread.send("append late", false).await.unwrap_err();
    assert!(refused.contains("moving to another host"), "{refused}");

    let moved = project_b.move_in(package.clone()).await.unwrap();
    let file = moved
        .worktree()
        .await
        .unwrap()
        .expect("the branch has its own worktree")
        .join("contract.txt");
    wait(&moved, completed(3)).await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\none\ntwo\n");
    assert!(
        moved
            .snapshot()
            .await
            .unwrap()
            .transcript
            .iter()
            .any(|item| item.text == package.handoff)
    );
    let record = moved
        .move_record()
        .await
        .unwrap()
        .expect("the new host keeps the move");
    assert_eq!(record.from_host, package.from_host);
    assert_eq!(
        record.to_host,
        pair.host_b.environment_name().await.unwrap().to_lowercase()
    );
    assert!(
        record
            .transcript
            .iter()
            .any(|item| item.text == "append first")
    );

    let id = thread.id().to_owned();
    let moving = project_a
        .threads()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id == id)
        .unwrap();
    thread
        .move_finish(
            &package.transfer,
            MoveFinish::Seal {
                to_host: record.to_host.clone(),
            },
        )
        .await
        .unwrap();
    assert!(
        project_a
            .threads()
            .await
            .unwrap()
            .iter()
            .all(|t| t.id != thread.id())
    );
    let sealed = thread.send("append late", false).await.unwrap_err();
    assert!(sealed.contains("moved to another host"), "{sealed}");
    assert!(thread.move_check().await.unwrap().blocked.is_some());
    project_a
        .archive(&id, true)
        .await
        .expect("a sealed session can be archived");
    project_a
        .delete(&moving, false)
        .await
        .expect("a sealed session can be deleted");
    pair.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_the_new_host_refuses_leaves_the_session_on_its_host() {
    let pair = Pair::codex().await;
    let (project_a, project_b) = pair.projects().await;
    let thread = appended(&project_a).await;
    let package = handoff(&thread).await;

    let mut unknown = package.clone();
    unknown.branch = "no-such-branch".to_owned();
    assert!(project_b.move_in(unknown.clone()).await.is_err());
    assert_eq!(
        arrival(&pair.host_b, &unknown.transfer).await,
        Arrival::Starting
    );
    assert_eq!(
        withdraw(&pair.host_b, &unknown.transfer).await,
        Arrival::Refused
    );
    assert!(project_b.threads().await.unwrap().is_empty());
    let other_url = format!("{}-other", pair.url);
    let other = clone_project(pair.dir.path(), &pair.origin, &other_url, "c");
    let project_c = pair.host_b.project(&other).await.unwrap();
    let mut foreign = package.clone();
    foreign.transfer = transfer();
    let refused = project_c.move_in(foreign).await.err();
    assert_eq!(
        refused.as_deref(),
        Some(
            "This project's Git remote is not the session's remote, so the session cannot move here."
        )
    );
    let listed = pair.host_b.projects().await.unwrap();
    assert!(listed.iter().any(|p| {
        p.root == other.to_string_lossy().into_owned()
            && p.remote_url.as_deref() == Some(other_url.as_str())
    }));

    release(&thread, &package.transfer).await;
    assert_eq!(thread.move_check().await.unwrap().blocked, None);
    thread.send("append again", false).await.unwrap();
    wait(&thread, completed(2)).await;
    pair.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_survives_restarts_while_preparing_and_once_handed_until_its_finish() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let home_a = dir.path().join("host-a");
    let (service, host) = start_codex(&home_a).await;
    let project = host.project(&repo_a).await.unwrap();
    let thread = appended(&project).await;
    hold_turn(&thread).await;
    send_queued(&thread, "append one").await;
    let id = thread.id().to_owned();
    let preparing = transfer();
    move_out(&thread, &preparing, Some("Append")).await.unwrap();
    service.stop().await;
    db(&home_a)
        .execute("UPDATE moved_thread SET state = 'preparing'", [])
        .unwrap();

    let (service, host) = start_codex(&home_a).await;
    let thread = host
        .project(&repo_a)
        .await
        .unwrap()
        .open(&id)
        .await
        .unwrap();
    assert_eq!(
        thread
            .move_check()
            .await
            .unwrap()
            .moving
            .map(|m| m.transfer),
        Some(preparing.clone())
    );
    assert!(thread.send("append late", false).await.is_err());
    let resumed = move_out(&thread, &preparing, Some("Append"))
        .await
        .expect("a repeat resumes the preparing move");
    assert_eq!(resumed.transfer, preparing);
    release(&thread, &preparing).await;
    assert_eq!(thread.move_check().await.unwrap().blocked, None);
    ran(&thread, "append one").await;
    let handed = transfer();
    move_out(&thread, &handed, Some("Append")).await.unwrap();
    service.stop().await;

    let (service, host) = start_codex(&home_a).await;
    let thread = host
        .project(&repo_a)
        .await
        .unwrap()
        .open(&id)
        .await
        .unwrap();
    assert!(thread.move_check().await.unwrap().blocked.is_some());
    assert!(thread.send("append late", false).await.is_err());
    release(&thread, &handed).await;
    assert_eq!(thread.move_check().await.unwrap().blocked, None);
    thread
        .send("append again", false)
        .await
        .expect("the session takes messages once its move is released");
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finish_repeats_refuses_the_other_outcome_and_a_released_message_runs_once() {
    let dir = tempdir::Dir::new();
    let (repo, service, host) = one_host(&dir).await;
    let project = host.project(&repo).await.unwrap();
    let thread = appended(&project).await;
    hold_turn(&thread).await;
    send_queued(&thread, "append one").await;

    let released = transfer();
    let package = move_out(&thread, &released, Some("Append")).await.unwrap();
    assert_eq!(package.queued, ["append one"]);
    release(&thread, &released).await;
    thread
        .move_finish(&released, MoveFinish::Release)
        .await
        .expect("a repeated release is answered again");
    let refused = thread
        .move_finish(
            &released,
            MoveFinish::Seal {
                to_host: "studio".to_owned(),
            },
        )
        .await
        .unwrap_err();
    assert!(refused.contains("released"), "{refused}");
    ran(&thread, "append one").await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let runs = thread
        .turns()
        .await
        .unwrap()
        .iter()
        .filter(|t| {
            t.prompt == "append one"
                && matches!(t.status, TurnStatus::Running | TurnStatus::Completed)
        })
        .count();
    assert_eq!(runs, 1, "the released message runs once");

    let sealed = transfer();
    let package = move_out(&thread, &sealed, Some("Append")).await.unwrap();
    let seal = MoveFinish::Seal {
        to_host: "studio".to_owned(),
    };
    thread.move_finish(&sealed, seal.clone()).await.unwrap();
    thread
        .move_finish(&sealed, seal)
        .await
        .expect("a repeated seal is answered again");
    let refused = thread
        .move_finish(&sealed, MoveFinish::Release)
        .await
        .unwrap_err();
    assert!(refused.contains("sealed"), "{refused}");
    assert_eq!(package.transfer, sealed);
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn repeats_of_one_move_out_return_the_same_package_and_a_second_transfer_is_refused() {
    let dir = tempdir::Dir::new();
    let (repo_a, service_a, host_a) = one_host(&dir).await;
    pushed_branch(&repo_a, "feature/repeated");
    let project_a = host_a.project(&repo_a).await.unwrap();
    let thread = appended(&project_a).await;
    hold_turn(&thread).await;
    thread.send("append second", false).await.unwrap();
    wait(&thread, |s| {
        s.turns.iter().any(|t| t.status == TurnStatus::Queued)
    })
    .await;
    let first_transfer = transfer();
    let (first, second) = tokio::join!(
        move_out(&thread, &first_transfer, Some("Append")),
        move_out(&thread, &first_transfer, Some("Append")),
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first.handoff, second.handoff);
    assert_eq!(first.queued, second.queued);
    let again = move_out(&thread, &first_transfer, Some("Append"))
        .await
        .expect("a repeat of the same transfer returns its package");
    assert_eq!(again, first);
    let other = move_out(&thread, &transfer(), None).await.unwrap_err();
    assert!(other.contains("already moving"), "{other}");
    service_a.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_release_during_a_move_out_waits_for_it_and_loses_no_message() {
    let dir = tempdir::Dir::new();
    let (repo, service, host) = one_host(&dir).await;
    pushed_branch(&repo, "feature/released-late");
    let project = host.project(&repo).await.unwrap();
    let thread = appended(&project).await;
    hold_turn(&thread).await;
    thread.send("keep one", false).await.unwrap();
    thread.send("keep two", false).await.unwrap();
    wait(&thread, |s| {
        s.turns
            .iter()
            .filter(|t| t.status == TurnStatus::Queued)
            .count()
            == 2
    })
    .await;
    let transfer = transfer();
    let (moved, released) = tokio::join!(move_out(&thread, &transfer, Some("Append")), async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        thread.move_finish(&transfer, MoveFinish::Release).await
    });
    let package = moved.unwrap();
    released.unwrap();
    assert_eq!(package.queued, ["keep one", "keep two"]);
    wait(&thread, |s| {
        ["keep one", "keep two"]
            .iter()
            .all(|text| s.turns.iter().any(|t| t.prompt == *text))
    })
    .await;
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_moving_session_stays_listed_with_its_open_move_until_it_is_released() {
    let dir = tempdir::Dir::new();
    let (repo, service, host) = one_host(&dir).await;
    pushed_branch(&repo, "feature/held");
    let project = host.project(&repo).await.unwrap();
    let thread = appended(&project).await;
    hold_turn(&thread).await;
    send_queued(&thread, "append one").await;
    let id = thread.id().to_owned();
    let transfer = transfer();
    let package = move_out(&thread, &transfer, Some("Append")).await.unwrap();
    let open = thread
        .move_check()
        .await
        .unwrap()
        .moving
        .expect("the handed move is open on the session");
    assert_eq!(open.transfer, package.transfer);

    let summary = project
        .threads()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id == id)
        .expect("a moving session stays listed");
    let moving = summary
        .moving
        .as_ref()
        .expect("the summary names the open move");
    assert_eq!(moving.transfer, transfer);
    assert_eq!(moving.phase, MovePhase::Handed);
    assert_eq!(moving.target_address, "host-b");
    assert_eq!(moving.target_name, "studio");
    assert_eq!(
        thread.move_check().await.unwrap().moving.map(|m| m.phase),
        Some(MovePhase::Handed)
    );

    let deleted = project.delete(&summary, false).await.err().unwrap();
    assert!(deleted.contains("moving to another host"), "{deleted}");
    let archived = project.archive(&id, true).await.err().unwrap();
    assert!(archived.contains("moving to another host"), "{archived}");
    thread.stop().await.expect("stop stays allowed");

    release(&thread, &transfer).await;
    let after = thread.move_check().await.unwrap();
    assert_eq!((after.moving, after.blocked), (None, None));
    wait(&thread, |s| {
        s.turns
            .iter()
            .any(|t| t.prompt == "append one" && t.status == TurnStatus::Completed)
    })
    .await;
    project
        .delete(&summary, false)
        .await
        .expect("a released session can be deleted");
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_departure_with_another_sessions_preparing_transfer_changes_no_session() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo = clone_project(dir.path(), &origin, &url, "a");
    let home = dir.path().join("host-a");
    let (service, host) = start_codex(&home).await;
    let project = host.project(&repo).await.unwrap();
    let owner = appended(&project).await;
    let id = owner.id().to_owned();
    hold_turn(&owner).await;
    send_queued(&owner, "append one").await;
    let preparing = transfer();
    move_out(&owner, &preparing, Some("Append")).await.unwrap();
    service.stop().await;
    db(&home)
        .execute("UPDATE moved_thread SET state = 'preparing'", [])
        .unwrap();

    let (service, host) = start_codex(&home).await;
    let project = host.project(&repo).await.unwrap();
    let owner = project.open(&id).await.unwrap();
    let matching = appended(&project).await;
    hold_turn(&matching).await;
    send_queued(&matching, "append two").await;
    let absent = appended(&project).await;
    for other in [&matching, &absent] {
        let refused = move_out(other, &preparing, Some("Append"))
            .await
            .unwrap_err();
        assert!(refused.contains("another session"), "{refused}");
    }
    let turns = matching.turns().await.unwrap();
    assert!(
        turns
            .iter()
            .any(|t| t.prompt == "append two" && t.status == TurnStatus::Queued)
    );
    release(&owner, &preparing).await;
    ran(&owner, "append one").await;
    service.stop().await;
}
