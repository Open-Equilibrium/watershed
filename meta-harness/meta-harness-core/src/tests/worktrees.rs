use super::checkpoint::{Repo, run};
use crate::{Checkpoint, Checkpoints, DiffError, Phase, WorktreeFileState, WorktreeRemoved};

/// A repository without remotes and a linked worktree on branch `linked`, one commit ahead of
/// `main`.
struct Linked {
    repo: Repo,
    linked: Repo,
}

impl Linked {
    fn new(name: &str) -> Self {
        let repo = Repo::new(&format!("worktrees-{name}"));
        repo.git(&["branch", "-M", "main"]);
        let linked = repo.path().with_extension("linked lane");
        let _ = std::fs::remove_dir_all(&linked);
        repo.git(&[
            "worktree",
            "add",
            "-q",
            "-b",
            "linked",
            linked.to_str().unwrap(),
        ]);
        let linked = Repo(linked);
        linked.write("b.txt", "linked\n");
        linked.git(&["add", "b.txt"]);
        linked.git(&["commit", "-q", "-m", "Linked work"]);
        Self { repo, linked }
    }

    fn path(&self) -> String {
        self.linked
            .path()
            .canonicalize()
            .unwrap()
            .display()
            .to_string()
    }
}

#[test]
fn lists_every_worktree_git_registers_with_what_keeps_it() {
    run(async {
        let linked = Linked::new("list");
        let locked = linked.repo.path().with_extension("locked");
        let gone = linked.repo.path().with_extension("gone");
        for (path, branch) in [(&locked, "locked"), (&gone, "gone")] {
            let _ = std::fs::remove_dir_all(path);
            linked.repo.git(&[
                "worktree",
                "add",
                "-q",
                "-b",
                branch,
                path.to_str().unwrap(),
            ]);
        }
        linked
            .repo
            .git(&["worktree", "lock", locked.to_str().unwrap()]);
        std::fs::remove_dir_all(&gone).unwrap();
        let checkpoints = Checkpoints::new(linked.repo.path());
        // A turn runs in the linked worktree.
        let there = Checkpoints::new(linked.linked.path());
        there
            .capture(&Checkpoint {
                session: "s".into(),
                turn: "t1".into(),
                phase: Phase::Start,
            })
            .await
            .unwrap();

        let listed = checkpoints.worktrees().await.unwrap();

        let main = &listed[0];
        assert!(main.main && main.current, "{main:?}");
        assert_eq!(main.branch.as_deref(), Some("main"));
        let find = |branch: &str| {
            listed
                .iter()
                .find(|worktree| worktree.branch.as_deref() == Some(branch))
                .unwrap()
        };
        let running = find("linked");
        assert_eq!(running.path, linked.path(), "a folder with a space");
        assert!(!running.main && !running.current && !running.running.is_empty());
        assert!(find("locked").locked);
        assert!(find("gone").missing);
        let _ = std::fs::remove_dir_all(locked);
    });
}

#[test]
fn removes_a_clean_worktree_and_keeps_its_branch_and_commits() {
    run(async {
        let linked = Linked::new("clean");
        let checkpoints = Checkpoints::new(linked.repo.path());

        let review = checkpoints.worktree_removal(&linked.path()).await.unwrap();

        assert!(review.removable(), "{review:?}");
        // No remote has the base commit or the branch's own.
        assert_eq!(review.unpushed, 2);
        assert_eq!(review.worktree.branch.as_deref(), Some("linked"));
        assert!(matches!(
            checkpoints.remove_worktree(&review).await.unwrap(),
            WorktreeRemoved::Done
        ));
        assert!(!linked.linked.path().exists());
        let log = linked.repo.git(&["log", "-1", "--format=%s", "linked"]);
        assert_eq!(log.trim(), "Linked work");
    });
}

#[test]
fn keeps_a_worktree_with_local_or_ignored_files() {
    run(async {
        let linked = Linked::new("dirty");
        let checkpoints = Checkpoints::new(linked.repo.path());
        linked.linked.write("a.txt", "staged\n");
        linked.linked.git(&["add", "a.txt"]);
        linked.linked.write("b.txt", "unstaged\n");
        linked.linked.write("new.txt", "untracked\n");

        let review = checkpoints.worktree_removal(&linked.path()).await.unwrap();

        assert!(!review.removable());
        let files: Vec<(&str, WorktreeFileState)> = review
            .files
            .iter()
            .map(|file| (file.path.as_str(), file.state))
            .collect();
        assert_eq!(
            files,
            [
                ("a.txt", WorktreeFileState::Staged),
                ("b.txt", WorktreeFileState::Unstaged),
                ("new.txt", WorktreeFileState::Untracked),
            ]
        );
        assert!(checkpoints.remove_worktree(&review).await.is_err());
        assert!(linked.linked.path().exists());

        // Ignored files alone keep it too: Git would delete them with the folder.
        linked
            .linked
            .git(&["checkout", "-q", "HEAD", "--", "a.txt", "b.txt"]);
        std::fs::remove_file(linked.linked.path().join("new.txt")).unwrap();
        linked.linked.write(".gitignore", "*.log\n");
        linked.linked.git(&["add", ".gitignore"]);
        linked.linked.git(&["commit", "-q", "-m", "Ignore logs"]);
        linked.linked.write("run.log", "kept\n");
        let review = checkpoints.worktree_removal(&linked.path()).await.unwrap();
        assert!(review.files.is_empty(), "{review:?}");
        assert_eq!(review.ignored, 1);
        assert!(!review.removable());
        assert!(checkpoints.remove_worktree(&review).await.is_err());
        assert!(linked.linked.path().join("run.log").exists());

        // Git names an ignored folder without walking it, so its files count only as more; an
        // empty one holds nothing.
        std::fs::remove_file(linked.linked.path().join("run.log")).unwrap();
        linked.linked.write(".gitignore", "*.log\nbuild/\ncache/\n");
        linked.linked.git(&["commit", "-q", "-am", "Ignore builds"]);
        for dir in ["build/deep", "cache"] {
            std::fs::create_dir_all(linked.linked.path().join(dir)).unwrap();
        }
        linked.linked.write("build/deep/out.o", "");
        linked.linked.write("build/out.o", "");
        let review = checkpoints.worktree_removal(&linked.path()).await.unwrap();
        assert_eq!((review.ignored, review.more_ignored), (0, true));
        assert!(!review.removable());
    });
}

#[test]
fn never_removes_the_main_or_own_checkout_or_an_unlisted_folder() {
    run(async {
        let linked = Linked::new("refused");
        let main = Checkpoints::new(linked.repo.path());
        let own = Checkpoints::new(linked.linked.path());

        let review = main
            .worktree_removal(
                &linked
                    .repo
                    .path()
                    .canonicalize()
                    .unwrap()
                    .display()
                    .to_string(),
            )
            .await
            .unwrap();
        assert!(review.worktree.main && !review.removable());
        let review = own.worktree_removal(&linked.path()).await.unwrap();
        assert!(review.worktree.current && !review.removable());
        assert!(own.remove_worktree(&review).await.is_err());
        assert!(matches!(
            main.worktree_removal("/tmp/not-a-worktree").await,
            Err(DiffError::Git(_))
        ));
        assert!(linked.linked.path().exists());
    });
}

#[test]
fn a_worktree_that_changed_since_its_review_is_reviewed_again() {
    run(async {
        let linked = Linked::new("changed");
        let checkpoints = Checkpoints::new(linked.repo.path());
        let review = checkpoints.worktree_removal(&linked.path()).await.unwrap();
        linked.linked.write("late.txt", "late\n");

        let removed = checkpoints.remove_worktree(&review).await.unwrap();

        let WorktreeRemoved::Changed(now) = removed else {
            panic!("removed despite the change");
        };
        assert_eq!(now.files.len(), 1);
        assert!(linked.linked.path().exists());
    });
}

#[test]
fn keeps_a_worktree_whose_edits_git_is_told_to_ignore() {
    run(async {
        let linked = Linked::new("marked");
        let checkpoints = Checkpoints::new(linked.repo.path());
        for (mark, file) in [
            ("--skip-worktree", "a.txt"),
            ("--assume-unchanged", "b.txt"),
        ] {
            linked.linked.git(&["update-index", mark, file]);
            linked
                .linked
                .write(file, "edited where Git does not look\n");
        }

        let review = checkpoints.worktree_removal(&linked.path()).await.unwrap();

        let files: Vec<(&str, WorktreeFileState)> = review
            .files
            .iter()
            .map(|file| (file.path.as_str(), file.state))
            .collect();
        assert_eq!(
            files,
            [
                ("a.txt", WorktreeFileState::Marked),
                ("b.txt", WorktreeFileState::Marked),
            ]
        );
        assert!(checkpoints.remove_worktree(&review).await.is_err());
        assert!(linked.linked.path().exists());
    });
}

#[test]
fn keeps_a_detached_worktree_with_commits_no_branch_holds() {
    run(async {
        let repo = Repo::new("worktrees-detached");
        let detached = repo.path().with_extension("detached");
        let _ = std::fs::remove_dir_all(&detached);
        repo.git(&[
            "worktree",
            "add",
            "-q",
            "--detach",
            detached.to_str().unwrap(),
        ]);
        let detached = Repo(detached);
        detached.write("c.txt", "only here\n");
        detached.git(&["add", "c.txt"]);
        detached.git(&["commit", "-q", "-m", "Only here"]);
        let checkpoints = Checkpoints::new(repo.path());
        let path = detached
            .path()
            .canonicalize()
            .unwrap()
            .display()
            .to_string();

        let review = checkpoints.worktree_removal(&path).await.unwrap();

        assert!(review.worktree.branch.is_none());
        assert_eq!(review.stranded, 1);
        assert!(!review.removable());
        assert!(checkpoints.remove_worktree(&review).await.is_err());
        assert!(detached.path().exists());
    });
}

#[test]
fn a_turn_running_in_any_worktree_keeps_the_others_too() {
    run(async {
        let linked = Linked::new("busy");
        let checkpoints = Checkpoints::new(linked.repo.path());
        let review = checkpoints.worktree_removal(&linked.path()).await.unwrap();
        assert!(review.removable());
        let main = Checkpoints::new(linked.repo.path());
        main.capture(&Checkpoint {
            session: "busy".into(),
            turn: "t1".into(),
            phase: Phase::Start,
        })
        .await
        .unwrap();

        let refused = checkpoints.remove_worktree(&review).await;

        assert!(matches!(refused, Err(DiffError::AgentRunning(sessions)) if sessions == ["busy"]));
        assert!(linked.linked.path().exists());
    });
}

#[test]
fn a_turn_starting_while_a_worktree_is_removed_does_not_hold_its_checkout() {
    use crate::checkout::Checkout;
    run(async {
        let root =
            std::env::temp_dir().join(format!("watershed-checkout-order-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let checkout = Checkout::new(root.join("checkout"), root.join("repository"));
        // Removal holds the repository, then asks for the worktree's checkout.
        let removal = checkout.repository_alone(Vec::new).await.unwrap();
        let (waiting, started) = tokio::sync::oneshot::channel();
        let turn = tokio::spawn({
            let checkout = Checkout::new(root.join("checkout"), root.join("repository"));
            async move {
                checkout
                    .run("busy", move || {
                        let _ = waiting.send(());
                    })
                    .await
            }
        });
        started.await.unwrap();
        // The waiting turn must not keep the checkout that removal needs.
        let alone = checkout.alone().await;
        assert!(alone.is_ok(), "{:?}", alone.as_ref().err());
        drop(alone);
        drop(removal);
        turn.await.unwrap().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    });
}

#[test]
fn a_sparse_worktree_without_hidden_edits_can_be_removed() {
    run(async {
        let linked = Linked::new("sparse");
        let checkpoints = Checkpoints::new(linked.repo.path());
        linked
            .linked
            .git(&["sparse-checkout", "set", "--no-cone", "/b.txt"]);
        assert!(!linked.linked.path().join("a.txt").exists());

        let review = checkpoints.worktree_removal(&linked.path()).await.unwrap();

        assert!(review.files.is_empty(), "{review:?}");
        assert!(review.removable());
    });
}

#[test]
fn a_worktree_gone_since_its_review_is_reviewed_again() {
    run(async {
        let linked = Linked::new("gone");
        let checkpoints = Checkpoints::new(linked.repo.path());
        let review = checkpoints.worktree_removal(&linked.path()).await.unwrap();
        std::fs::remove_dir_all(linked.linked.path()).unwrap();

        let removed = checkpoints.remove_worktree(&review).await.unwrap();

        let WorktreeRemoved::Changed(now) = removed else {
            panic!("removed a missing worktree");
        };
        assert!(now.worktree.missing);
    });
}

#[test]
fn more_marked_files_than_named_are_not_called_local_changes() {
    run(async {
        let linked = Linked::new("many-marked");
        let checkpoints = Checkpoints::new(linked.repo.path());
        let names: Vec<String> = (0..205).map(|n| format!("m{n:03}.txt")).collect();
        for name in &names {
            linked.linked.write(name, "kept\n");
        }
        linked.linked.git(&["add", "."]);
        linked.linked.git(&["commit", "-q", "-m", "Many"]);
        let mut args = vec!["update-index", "--assume-unchanged"];
        args.extend(names.iter().map(String::as_str));
        linked.linked.git(&args);

        let review = checkpoints.worktree_removal(&linked.path()).await.unwrap();

        assert!(review.more_marked && !review.more_files, "{review:?}");
        assert!(
            review
                .files
                .iter()
                .all(|file| file.state == WorktreeFileState::Marked)
        );
        assert!(!review.removable());
    });
}
