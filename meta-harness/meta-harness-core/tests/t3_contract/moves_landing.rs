use super::*;

type Steps = &'static [&'static [&'static str]];

#[tokio::test(flavor = "multi_thread")]
async fn a_move_is_refused_onto_a_branch_or_checkout_that_tracks_another_branch() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let cases: [(&str, Steps, Steps, &str); 3] = [
        (
            "feature/tracked",
            &[
                &["checkout", "-q", "-b", "feature/tracked"],
                &["push", "-q", "-u", "origin", "feature/tracked"],
            ],
            &[&["branch", "--track", "feature/tracked", "origin/main"]],
            "tracks another branch",
        ),
        (
            "feature/local-tracked",
            &[
                &["checkout", "-q", "-b", "feature/local-tracked"],
                &["push", "-q", "-u", "origin", "feature/local-tracked"],
            ],
            &[&["branch", "--track", "feature/local-tracked", "main"]],
            "tracks another branch",
        ),
        (
            "main",
            &[&["push", "-q", "origin", "main:refs/heads/side"]],
            &[
                &["fetch", "-q", "origin"],
                &["branch", "--set-upstream-to=origin/side", "main"],
            ],
            "tracks",
        ),
    ];
    for (index, (branch, a_steps, b_steps, refusal)) in cases.into_iter().enumerate() {
        let repo_a = clone_project(dir.path(), &origin, &url, &format!("a{index}"));
        let repo_b = clone_project(dir.path(), &origin, &url, &format!("b{index}"));
        for args in a_steps {
            git(&repo_a, args);
        }
        for args in b_steps {
            git(&repo_b, args);
        }
        let project_a = host_a.project(&repo_a).await.unwrap();
        let project_b = host_b.project(&repo_b).await.unwrap();
        let package = handoff(&appended(&project_a).await).await;
        assert_eq!(package.branch, branch);
        let refused = project_b.move_in(package).await.err().unwrap();
        assert!(refused.contains(refusal), "{branch}: {refused}");
        assert!(project_b.threads().await.unwrap().is_empty());
    }
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_attached_branch_with_unpushed_commits_is_refused() {
    let pair = Pair::codex().await;
    pushed_branch(&pair.repo_a, "feature/unpushed");
    local_from_origin(&pair.repo_b, "feature/unpushed");
    git(&pair.repo_b, &["checkout", "-q", "feature/unpushed"]);
    std::fs::write(pair.repo_b.join("local.txt"), "local only\n").unwrap();
    git(&pair.repo_b, &["add", "local.txt"]);
    git(&pair.repo_b, &["commit", "-q", "-m", "local only"]);
    git(&pair.repo_b, &["checkout", "-q", "main"]);
    let (project_a, project_b) = pair.projects().await;
    let package = handoff(&appended(&project_a).await).await;

    assert_refused(
        project_b.move_in(package).await,
        "has commits that are not pushed",
    );
    pair.stop().await;
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
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_lands_in_the_clean_checkout_the_target_has_on_its_branch() {
    let pair = Pair::codex().await;
    let (project_a, project_b) = pair.projects().await;
    let package = handoff(&appended(&project_a).await).await;
    assert_eq!(package.branch, "main");

    std::fs::write(pair.repo_b.join("README.md"), "changed here\n").unwrap();
    assert_refused(
        project_b.move_in(package.clone()).await,
        "uncommitted changes",
    );
    assert!(project_b.threads().await.unwrap().is_empty());
    git(&pair.repo_b, &["checkout", "-q", "--", "README.md"]);

    let landed = project_b.move_in(package).await.unwrap();
    assert_eq!(
        landed.worktree().await.unwrap().as_deref(),
        Some(pair.repo_b.as_path()),
        "the session continues in the checkout"
    );
    pair.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_onto_a_local_branch_that_is_not_checked_out_attaches_a_worktree() {
    let pair = Pair::codex().await;
    pushed_branch(&pair.repo_a, "feature/attach");
    local_from_origin(&pair.repo_b, "feature/attach");
    let (project_a, project_b) = pair.projects().await;
    let package = handoff(&appended(&project_a).await).await;

    let landed = project_b.move_in(package).await.unwrap();
    let worktree = landed
        .worktree()
        .await
        .unwrap()
        .expect("the branch has its own worktree");
    assert_ne!(worktree, *pair.repo_b);
    pair.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_is_refused_while_a_live_session_uses_the_checkout_it_would_continue_in() {
    let pair = Pair::codex().await;
    let (project_a, project_b) = pair.projects().await;
    let package = handoff(&appended(&project_a).await).await;
    let live = project_b.start(codex_start("work in b")).await.unwrap();
    wait(&live, completed(1)).await;

    assert_refused(project_b.move_in(package).await, "checked out");
    assert_eq!(project_b.threads().await.unwrap().len(), 1);
    pair.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_branch_in_a_project_with_two_remotes_is_blocked_and_an_absent_branch_still_gets_a_worktree()
 {
    let pair = Pair::codex().await;
    git(
        &pair.repo_b,
        &["remote", "add", "second", &pair.origin.to_string_lossy()],
    );
    git(&pair.repo_b, &["fetch", "-q", "second"]);
    let (project_a, project_b) = pair.projects().await;
    let root = pair.repo_b.to_string_lossy().into_owned();
    let package = handoff(&appended(&project_a).await).await;

    let Landing::Blocked { reason } = pair.host_b.preflight(&root, "main").await.unwrap() else {
        panic!("a branch that exists in a project with two remotes is blocked");
    };
    assert!(reason.contains("more than one Git remote"), "{reason}");
    assert_eq!(
        pair.host_b
            .preflight(&root, "feature/absent")
            .await
            .unwrap(),
        Landing::NewWorktree
    );
    assert_refused(project_b.move_in(package).await, "more than one Git remote");
    assert!(project_b.threads().await.unwrap().is_empty());
    pair.stop().await;
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
    let thread = appended(&project).await;
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
