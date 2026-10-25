use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::{ChangeStatus, DiffError, FileChange, WorkingTree};

/// A throwaway Git repository that ignores the user's Git configuration.
struct Repo(PathBuf);

impl Repo {
    fn new(name: &str) -> Self {
        Self::at(std::env::temp_dir().join(format!("watershed-diff-{name}-{}", std::process::id())))
    }

    fn at(dir: PathBuf) -> Self {
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

#[test]
fn snapshots_without_running_the_repositorys_hooks() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        let repo = Repo::new("hooks");
        repo.write("a.txt", "a\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "base"]);
        let hook = repo.path().join(".git/hooks/post-index-change");
        std::fs::write(&hook, "#!/bin/sh\ntouch hook-ran\n").expect("hook");
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).expect("mode");
        repo.write("a.txt", "changed\n");
        let tree = WorkingTree::open(repo.path())
            .await
            .expect("repository opens");
        tree.snapshot().await.expect("snapshot");
        assert!(
            !repo.path().join("hook-ran").exists(),
            "a snapshot leaves the project alone"
        );
    });
}

#[test]
fn shows_a_patch_without_running_the_projects_textconv() {
    run(async {
        let repo = Repo::new("textconv");
        repo.write("a.txt", "a\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "base"]);
        let ran = repo.path().join("ran");
        repo.git(&[
            "config",
            "diff.evil.textconv",
            &format!("touch '{}'; cat", ran.display()),
        ]);
        repo.write(".gitattributes", "*.txt diff=evil\n");
        let tree = WorkingTree::open(repo.path())
            .await
            .expect("repository opens");
        let before = tree.snapshot().await.expect("snapshot");
        repo.write("a.txt", "changed\n");
        let patch = tree.patch(&before, "a.txt").await.expect("patch");
        assert!(patch.contains("+changed"), "{patch}");
        assert!(!ran.exists(), "the project's textconv did not run");
    });
}

#[test]
fn diffs_a_project_given_by_a_relative_path() {
    run(async {
        // Below the working directory, so the path stays relative wherever Git resolves it.
        let relative = PathBuf::from(format!("watershed-diff-relative-{}", std::process::id()));
        let repo = Repo::at(
            std::env::current_dir()
                .expect("working directory")
                .join(&relative),
        );
        repo.write("a.txt", "old\n");
        repo.write("b.txt", "kept\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "base"]);
        // Clean index entries, so the snapshot reuses the repository's objects for b.txt.
        let past = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        for name in ["a.txt", "b.txt"] {
            std::fs::File::options()
                .write(true)
                .open(repo.path().join(name))
                .and_then(|file| file.set_modified(past))
                .expect("file time");
        }
        repo.git(&["update-index", "--refresh"]);
        let tree = WorkingTree::open(&relative)
            .await
            .expect("repository opens");
        let snapshot = tree.snapshot().await.expect("snapshot");
        repo.write("a.txt", "new\n");
        assert_eq!(
            tree.changes(&snapshot).await.expect("changes"),
            [FileChange {
                path: "a.txt".to_owned(),
                status: ChangeStatus::Modified,
                added: 1,
                removed: 1,
            }]
        );
    });
}

#[test]
fn reads_the_patch_of_a_file_whose_name_looks_like_a_pattern() {
    run(async {
        let repo = Repo::new("pathspec");
        repo.write("a.txt", "one\n");
        repo.write("a*.txt", "one\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "base"]);
        let tree = WorkingTree::open(repo.path())
            .await
            .expect("repository opens");
        let snapshot = tree.snapshot().await.expect("snapshot");
        repo.write("a.txt", "plain\n");
        repo.write("a*.txt", "starred\n");
        let patch = tree.patch(&snapshot, "a*.txt").await.expect("patch");
        assert!(patch.contains("+starred"), "{patch}");
        assert!(!patch.contains("+plain"), "only the named file: {patch}");
    });
}

#[test]
fn reports_a_diff_too_large_to_read_instead_of_cutting_it() {
    run(async {
        let repo = Repo::new("large");
        repo.write("big.txt", "");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "base"]);
        let tree = WorkingTree::open(repo.path())
            .await
            .expect("repository opens");
        let snapshot = tree.snapshot().await.expect("snapshot");
        // More than the 4 MiB Git output limit.
        repo.write("big.txt", &"a line of changed text\n".repeat(200_000));
        assert!(matches!(
            tree.patch(&snapshot, "big.txt").await,
            Err(DiffError::TooLarge)
        ));
    });
}

#[test]
fn snapshots_a_project_whose_git_warns_a_lot() {
    run(async {
        let repo = Repo::new("warnings");
        repo.git(&["config", "core.autocrlf", "true"]);
        // Each file makes `git add` warn about line endings, far more than a pipe holds.
        for number in 0..1500 {
            repo.write(&format!("file-with-a-long-name-{number}.txt"), "line\n");
        }
        let tree = WorkingTree::open(repo.path())
            .await
            .expect("repository opens");
        let snapshot = tokio::time::timeout(Duration::from_secs(30), tree.snapshot())
            .await
            .expect("snapshot finishes")
            .expect("snapshot");
        assert_eq!(tree.changes(&snapshot).await.expect("changes"), []);
    });
}
