use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{ChangeStatus, FileChange, WorkingTree};

/// A throwaway Git repository that ignores the user's Git configuration.
struct Repo(PathBuf);

impl Repo {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("watershed-diff-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("repository directory");
        let repo = Self(dir);
        repo.git(&["init", "-q"]);
        repo
    }

    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(&self.0)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
            .expect("git runs");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("UTF-8 output")
    }

    fn write(&self, path: &str, text: &str) {
        std::fs::write(self.0.join(path), text).expect("file written");
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn count_files(dir: &Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |entries| {
        entries
            .flatten()
            .map(|entry| {
                if entry.path().is_dir() {
                    count_files(&entry.path())
                } else {
                    1
                }
            })
            .sum()
    })
}

fn run<F: std::future::Future>(test: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime starts")
        .block_on(test)
}

#[test]
fn diffs_a_run_against_its_snapshot_without_touching_the_repository() {
    run(async {
        let repo = Repo::new("run");
        repo.write("a.txt", "old\nsame\n");
        repo.write("c.txt", "gone\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "base"]);
        repo.write("pending.txt", "before the run\n");

        let status_before = repo.git(&["status", "--porcelain"]);
        let tree = WorkingTree::open(repo.path())
            .await
            .expect("repository opens");
        let index_before = std::fs::read(repo.path().join(".git/index")).expect("index");
        let objects_before = count_files(&repo.path().join(".git/objects"));
        let snapshot = tree.snapshot().await.expect("snapshot");

        repo.write("a.txt", "new\nsame\n");
        repo.write("b.txt", "added\n");
        std::fs::remove_file(repo.path().join("c.txt")).expect("removed");

        let change = |path: &str, status, added, removed| FileChange {
            path: path.to_owned(),
            status,
            added,
            removed,
        };
        assert_eq!(
            tree.changes(&snapshot).await.expect("changes"),
            [
                change("a.txt", ChangeStatus::Modified, 1, 1),
                change("b.txt", ChangeStatus::Added, 1, 0),
                change("c.txt", ChangeStatus::Deleted, 0, 1),
            ]
        );
        let patch = tree.patch(&snapshot, "a.txt").await.expect("patch");
        assert!(
            patch.contains("-old\n") && patch.contains("+new\n"),
            "{patch}"
        );
        assert!(
            !patch.contains("pending"),
            "changes before the run stay out of the diff"
        );

        assert_eq!(
            std::fs::read(repo.path().join(".git/index")).expect("index"),
            index_before
        );
        assert_eq!(
            count_files(&repo.path().join(".git/objects")),
            objects_before
        );
        assert_ne!(
            repo.git(&["status", "--porcelain"]),
            status_before,
            "the worktree changed"
        );
        assert!(
            repo.git(&["status", "--porcelain"]).contains("?? b.txt"),
            "b.txt stays untracked"
        );
    });
}

#[test]
fn rejects_a_directory_outside_git() {
    run(async {
        let dir = std::env::temp_dir().join(format!("watershed-no-git-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("directory");
        let outside = WorkingTree::open(&dir).await;
        let _ = std::fs::remove_dir_all(&dir);
        assert!(outside.is_err());
    });
}

#[test]
fn keeps_snapshots_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        let repo = Repo::new("private");
        repo.write("a.txt", "a\n");
        let tree = WorkingTree::open(repo.path())
            .await
            .expect("repository opens");
        tree.snapshot().await.expect("snapshot");
        // Other tests' stores may come and go meanwhile; this tree's store stays until the end.
        let prefix = format!("watershed-snapshots-{}-", std::process::id());
        let modes: Vec<u32> = std::fs::read_dir(std::env::temp_dir())
            .expect("temporary directory")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
            .filter_map(|entry| entry.metadata().ok())
            .map(|metadata| metadata.permissions().mode() & 0o777)
            .collect();
        assert!(!modes.is_empty(), "the store exists");
        assert!(modes.iter().all(|&mode| mode == 0o700), "{modes:?}");
    });
}
