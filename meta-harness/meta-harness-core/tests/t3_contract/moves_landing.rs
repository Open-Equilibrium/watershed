use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn a_branch_that_tracks_another_is_not_attached_by_a_move() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/tracked"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/tracked"]);
    git(
        &repo_b,
        &["branch", "--track", "feature/tracked", "origin/main"],
    );
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
    let refused = project_b.move_in(package).await.err().unwrap();
    assert!(refused.contains("tracks another branch"), "{refused}");
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_attached_branch_with_unpushed_commits_is_refused() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/unpushed"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/unpushed"]);
    git(&repo_b, &["fetch", "-q", "origin"]);
    git(
        &repo_b,
        &["branch", "feature/unpushed", "origin/feature/unpushed"],
    );
    git(&repo_b, &["checkout", "-q", "feature/unpushed"]);
    std::fs::write(repo_b.join("local.txt"), "local only\n").unwrap();
    git(&repo_b, &["add", "local.txt"]);
    git(&repo_b, &["commit", "-q", "-m", "local only"]);
    git(&repo_b, &["checkout", "-q", "main"]);
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

    let refused = project_b.move_in(package).await.err().unwrap();
    assert!(
        refused.contains("has commits that are not pushed"),
        "{refused}"
    );
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_branch_that_tracks_a_local_branch_is_not_attached_by_a_move() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/local-tracked"]);
    git(
        &repo_a,
        &["push", "-q", "-u", "origin", "feature/local-tracked"],
    );
    git(
        &repo_b,
        &["branch", "--track", "feature/local-tracked", "main"],
    );
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
    let refused = project_b.move_in(package).await.err().unwrap();
    assert!(refused.contains("tracks another branch"), "{refused}");
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_preflight_says_where_a_branch_would_land_and_why_it_cannot() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_b = host_b.project(&repo_b).await.unwrap();
    let root = repo_b.to_string_lossy().into_owned();

    assert_eq!(
        host_b.preflight(&root, "feature/new").await.unwrap(),
        Landing::NewWorktree
    );
    assert_eq!(
        host_b.preflight(&root, "main").await.unwrap(),
        Landing::Continue { path: root.clone() }
    );
    project_b.git().create_branch("feature/side").await.unwrap();
    project_b.git().switch("main").await.unwrap();
    let Landing::Blocked { reason } = host_b.preflight(&root, "feature/side").await.unwrap() else {
        panic!("a branch without an upstream is not attached");
    };
    assert!(reason.contains("has no upstream branch"), "{reason}");
    git(&repo_b, &["push", "-q", "-u", "origin", "feature/side"]);
    let attached = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match host_b.preflight(&root, "feature/side").await.unwrap() {
                Landing::Attach => break,
                _ => tokio::time::sleep(Duration::from_millis(500)).await,
            }
        }
    })
    .await;
    assert!(attached.is_ok(), "the pushed branch is attached");
    std::fs::write(repo_b.join("README.md"), "changed here\n").unwrap();
    let Landing::Blocked { reason } = host_b.preflight(&root, "main").await.unwrap() else {
        panic!("a dirty checkout blocks the move");
    };
    assert!(reason.contains("uncommitted changes"), "{reason}");
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_lands_in_the_clean_checkout_the_target_has_on_its_branch() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
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
    assert_eq!(package.branch, "main");

    std::fs::write(repo_b.join("README.md"), "changed here\n").unwrap();
    let refused = project_b.move_in(package.clone()).await.err().unwrap();
    assert!(refused.contains("uncommitted changes"), "{refused}");
    assert!(project_b.threads().await.unwrap().is_empty());
    git(&repo_b, &["checkout", "-q", "--", "README.md"]);

    let landed = project_b.move_in(package.clone()).await.unwrap();
    assert_eq!(
        landed.worktree().await.unwrap().as_deref(),
        Some(repo_b.as_path()),
        "the session continues in the checkout"
    );
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_onto_a_local_branch_that_is_not_checked_out_attaches_a_worktree() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/attach"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/attach"]);
    git(&repo_b, &["fetch", "-q", "origin"]);
    git(
        &repo_b,
        &["branch", "feature/attach", "origin/feature/attach"],
    );
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

    let landed = project_b.move_in(package.clone()).await.unwrap();
    let worktree = landed
        .worktree()
        .await
        .unwrap()
        .expect("the branch has its own worktree");
    assert_ne!(worktree, repo_b);
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_is_refused_while_a_live_session_uses_the_checkout_it_would_continue_in() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
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
    let live = project_b.start(codex_start("work in b")).await.unwrap();
    wait(&live, completed(1)).await;

    let refused = project_b.move_in(package.clone()).await.err().unwrap();
    assert!(refused.contains("checked out"), "{refused}");
    assert_eq!(project_b.threads().await.unwrap().len(), 1);
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_is_refused_onto_a_checkout_that_tracks_another_branch() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["push", "-q", "origin", "main:refs/heads/side"]);
    git(&repo_b, &["fetch", "-q", "origin"]);
    git(
        &repo_b,
        &["branch", "--set-upstream-to=origin/side", "main"],
    );
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

    let refused = project_b.move_in(package.clone()).await.err().unwrap();
    assert!(refused.contains("tracks"), "{refused}");
    assert!(project_b.threads().await.unwrap().is_empty());
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_branch_in_a_project_with_two_remotes_is_blocked_and_an_absent_branch_still_gets_a_worktree()
 {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(
        &repo_b,
        &["remote", "add", "second", &origin.to_string_lossy()],
    );
    git(&repo_b, &["fetch", "-q", "second"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let root = repo_b.to_string_lossy().into_owned();
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

    let Landing::Blocked { reason } = host_b.preflight(&root, "main").await.unwrap() else {
        panic!("a branch that exists in a project with two remotes is blocked");
    };
    assert!(reason.contains("more than one Git remote"), "{reason}");
    assert_eq!(
        host_b.preflight(&root, "feature/absent").await.unwrap(),
        Landing::NewWorktree
    );
    let refused = project_b.move_in(package.clone()).await.err().unwrap();
    assert!(refused.contains("more than one Git remote"), "{refused}");
    assert!(project_b.threads().await.unwrap().is_empty());
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_reports_a_stable_environment_id_and_a_move_keeps_its_target() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let environment = host_b.environment_id().await.unwrap();
    assert_eq!(environment.len(), 36, "{environment}");
    assert_eq!(environment.matches('-').count(), 4, "{environment}");
    service_b.stop().await;
    let (service_b, host_b) = start_codex(&home_b).await;
    assert_eq!(host_b.environment_id().await.unwrap(), environment);
    service_b.stop().await;

    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let project = host_a.project(&repo_a).await.unwrap();
    let thread = project.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            &environment,
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let moving = thread.move_check().await.unwrap().moving.unwrap();
    assert_eq!(moving.target_environment, environment);
    service_a.stop().await;
}
