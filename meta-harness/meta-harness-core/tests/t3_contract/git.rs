use super::*;

async fn run(git: &remote::Git, action: GitAction, message: Option<&str>) -> GitProgress {
    let mut progress = git
        .run(GitActionRequest {
            action,
            message: message.map(str::to_owned),
            files: None,
            feature_branch: false,
        })
        .await
        .unwrap();
    let mut last = None;
    while let Some(step) = progress.recv().await {
        last = Some(step);
    }
    last.expect("the action reports how it ended")
}

#[tokio::test(flavor = "multi_thread")]
async fn git_commits_branches_and_pushes_as_t3_does() {
    let (dir, repo, home) = fixture();
    let origin = dir.path().join("origin.git");
    git(dir.path(), &["init", "-q", "--bare", "origin.git"]);
    git(
        &repo,
        &["remote", "add", "origin", &origin.to_string_lossy()],
    );
    git(&repo, &["push", "-q", "-u", "origin", "main"]);
    let (service, host) = start_service(&home).await;
    let project = host.project(&repo).await.unwrap();
    let git = project.git();

    std::fs::write(repo.join("README.md"), "contract\nchanged\n").unwrap();
    let status = git.status().await.unwrap();
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert!(status.repository && status.remote && status.default_branch);
    assert_eq!(status.changes.len(), 1);
    assert!(
        git.uncommitted_diff(false)
            .await
            .unwrap()
            .contains("+changed")
    );

    git.create_branch("feature").await.unwrap();
    let refs = git.refs("feat", 0, 50).await.unwrap();
    assert!(refs.refs.iter().any(|r| r.name == "feature" && r.current));
    let ended = run(&git, GitAction::CommitPush, Some("Change the readme")).await;
    assert!(matches!(ended, GitProgress::Finished { .. }), "{ended:?}");
    let status = git.status().await.unwrap();
    assert!(status.changes.is_empty() && status.upstream);
    assert_eq!(status.ahead, 0);
    assert_eq!(pushed_subject(&origin, "feature"), "Change the readme");

    git.switch("main").await.unwrap();
    assert_eq!(git.status().await.unwrap().branch.as_deref(), Some("main"));
    let plain = dir.path().join("plain");
    std::fs::create_dir(&plain).unwrap();
    let plain_git = host.project(&plain).await.unwrap().git();
    assert!(!plain_git.status().await.unwrap().repository);
    plain_git.init().await.unwrap();
    assert!(plain_git.status().await.unwrap().repository);
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_changes_and_the_branch_diff_compare_the_branch_with_its_base_as_t3_does() {
    let (dir, repo, home) = fixture();
    let origin = dir.path().join("origin.git");
    git(dir.path(), &["init", "-q", "--bare", "origin.git"]);
    git(
        &repo,
        &["remote", "add", "origin", &origin.to_string_lossy()],
    );
    git(&repo, &["push", "-q", "-u", "origin", "main"]);
    let (service, host) = start_service(&home).await;
    let checkout = host.project(&repo).await.unwrap().git();

    checkout.create_branch("feature").await.unwrap();
    std::fs::write(repo.join("README.md"), "contract\nbranch\n").unwrap();
    git(&repo, &["commit", "-qam", "Change the readme"]);
    std::fs::write(repo.join("README.md"), "contract\nbranch\nmore\n").unwrap();

    let status = checkout.status().await.unwrap();
    assert_eq!(
        status.working_tree_lines,
        LineCount {
            added: 1,
            removed: 0
        }
    );
    let branch = status
        .branch_changes
        .expect("T3 counts the branch against its base");
    assert_eq!(branch.base.as_deref(), Some("origin/main"));
    assert_eq!(
        branch.lines,
        LineCount {
            added: 2,
            removed: 0
        }
    );
    let diff = checkout.branch_diff(false).await.unwrap();
    assert!(diff.contains("+branch") && diff.contains("+more"), "{diff}");
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_clones_a_remote_into_a_project_and_names_it() {
    let dir = tempdir::Dir::new();
    let (origin, _url) = shared_remote(dir.path());
    let (service, host) = start_codex(&dir.path().join("host")).await;
    let destination = dir.path().join("cloned");
    let cloned = host
        .clone_project(&origin.to_string_lossy(), &destination.to_string_lossy())
        .await
        .unwrap();
    assert_eq!(cloned.root, destination.to_string_lossy());
    assert!(destination.join("README.md").exists());
    let folder = dir.path().join("plain");
    std::fs::create_dir(&folder).unwrap();
    let added = host.add_project(&folder.to_string_lossy()).await.unwrap();
    assert_eq!(added.root, folder.to_string_lossy().into_owned());
    assert!(
        host.projects()
            .await
            .unwrap()
            .iter()
            .any(|p| p.root == cloned.root)
    );
    service.stop().await;
}
