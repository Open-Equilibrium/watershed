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
async fn git_commits_branches_pushes_and_compares_branches_with_their_base_as_t3_does() {
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
    let checkout = project.git();

    std::fs::write(repo.join("README.md"), "contract\nchanged\n").unwrap();
    let status = checkout.status().await.unwrap();
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert!(status.repository && status.remote && status.default_branch);
    assert_eq!(status.changes.len(), 1);
    assert!(
        checkout
            .uncommitted_diff(false)
            .await
            .unwrap()
            .contains("+changed")
    );

    checkout.create_branch("feature").await.unwrap();
    let refs = checkout.refs("feat", 0, 50).await.unwrap();
    assert!(refs.refs.iter().any(|r| r.name == "feature" && r.current));
    let ended = run(&checkout, GitAction::CommitPush, Some("Change the readme")).await;
    assert!(matches!(ended, GitProgress::Finished { .. }), "{ended:?}");
    let status = checkout.status().await.unwrap();
    assert!(status.changes.is_empty() && status.upstream);
    assert_eq!(status.ahead, 0);
    assert_eq!(pushed_subject(&origin, "feature"), "Change the readme");

    checkout.switch("main").await.unwrap();
    assert_eq!(
        checkout.status().await.unwrap().branch.as_deref(),
        Some("main")
    );
    let plain = dir.path().join("plain");
    std::fs::create_dir(&plain).unwrap();
    let plain_git = host.project(&plain).await.unwrap().git();
    assert!(!plain_git.status().await.unwrap().repository);
    plain_git.init().await.unwrap();
    assert!(plain_git.status().await.unwrap().repository);

    let destination = dir.path().join("cloned");
    let cloned = host
        .clone_project(&origin.to_string_lossy(), &destination.to_string_lossy())
        .await
        .unwrap();
    assert_eq!(cloned.root, destination.to_string_lossy());
    assert!(destination.join("README.md").exists());
    let added = dir.path().join("added");
    std::fs::create_dir(&added).unwrap();
    let listed = host.add_project(&added.to_string_lossy()).await.unwrap();
    assert_eq!(listed.root, added.to_string_lossy().into_owned());
    assert!(
        host.projects()
            .await
            .unwrap()
            .iter()
            .any(|p| p.root == cloned.root)
    );

    checkout.create_branch("compare").await.unwrap();
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
