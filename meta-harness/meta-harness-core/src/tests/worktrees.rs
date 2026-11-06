use super::checkpoint::{Repo, run};
use crate::{Checkpoint, Checkpoints, DiffError, Phase, WorktreeFileState, WorktreeRemoved};
use std::path::{Path, PathBuf};

/// A repository without remotes and a linked worktree on branch `linked`, one commit ahead of
/// `main`.
struct Linked {
    repo: Repo,
    linked: PathBuf,
}

impl Linked {
    fn new(name: &str) -> Self {
        let repo = Repo::new(&format!("worktrees-{name}"));
        repo.git(&["branch", "-M", "main"]);
        let linked = repo.path().with_extension("linked");
        let _ = std::fs::remove_dir_all(&linked);
        repo.git(&[
            "worktree",
            "add",
            "-q",
            "-b",
            "linked",
            linked.to_str().unwrap(),
        ]);
        std::fs::write(linked.join("b.txt"), "linked\n").unwrap();
        git_in(&linked, &["add", "b.txt"]);
        git_in(&linked, &["commit", "-q", "-m", "Linked work"]);
        Self { repo, linked }
    }

    fn path(&self) -> String {
        self.linked.canonicalize().unwrap().display().to_string()
    }
}

impl Drop for Linked {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.linked);
    }
}

fn git_in(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
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
        let there = Checkpoints::new(&linked.linked);
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
        assert_eq!(running.path, linked.path());
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
        assert!(!linked.linked.exists());
        let log = linked.repo.git(&["log", "-1", "--format=%s", "linked"]);
        assert_eq!(log.trim(), "Linked work");
    });
}

#[test]
fn keeps_a_worktree_with_local_or_ignored_files() {
    run(async {
        let linked = Linked::new("dirty");
        let checkpoints = Checkpoints::new(linked.repo.path());
        std::fs::write(linked.linked.join("a.txt"), "staged\n").unwrap();
        git_in(&linked.linked, &["add", "a.txt"]);
        std::fs::write(linked.linked.join("b.txt"), "unstaged\n").unwrap();
        std::fs::write(linked.linked.join("new.txt"), "untracked\n").unwrap();

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
        assert!(linked.linked.exists());

        // Ignored files alone keep it too: Git would delete them with the folder.
        git_in(
            &linked.linked,
            &["checkout", "-q", "HEAD", "--", "a.txt", "b.txt"],
        );
        std::fs::remove_file(linked.linked.join("new.txt")).unwrap();
        std::fs::write(linked.linked.join(".gitignore"), "*.log\n").unwrap();
        git_in(&linked.linked, &["add", ".gitignore"]);
        git_in(&linked.linked, &["commit", "-q", "-m", "Ignore logs"]);
        std::fs::write(linked.linked.join("run.log"), "kept\n").unwrap();
        let review = checkpoints.worktree_removal(&linked.path()).await.unwrap();
        assert!(review.files.is_empty(), "{review:?}");
        assert_eq!(review.ignored, 1);
        assert!(!review.removable());
        assert!(checkpoints.remove_worktree(&review).await.is_err());
        assert!(linked.linked.join("run.log").exists());
    });
}

#[test]
fn never_removes_the_main_or_own_checkout_or_an_unlisted_folder() {
    run(async {
        let linked = Linked::new("refused");
        let main = Checkpoints::new(linked.repo.path());
        let own = Checkpoints::new(&linked.linked);

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
        assert!(linked.linked.exists());
    });
}

#[test]
fn a_worktree_that_changed_since_its_review_is_reviewed_again() {
    run(async {
        let linked = Linked::new("changed");
        let checkpoints = Checkpoints::new(linked.repo.path());
        let review = checkpoints.worktree_removal(&linked.path()).await.unwrap();
        std::fs::write(linked.linked.join("late.txt"), "late\n").unwrap();

        let removed = checkpoints.remove_worktree(&review).await.unwrap();

        let WorktreeRemoved::Changed(now) = removed else {
            panic!("removed despite the change");
        };
        assert_eq!(now.files.len(), 1);
        assert!(linked.linked.exists());
    });
}
