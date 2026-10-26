use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::{ChangeStatus, DiffError, FileChange, RunDiff};

/// A throwaway Git repository with one committed file, `a.txt`.
struct Repo(PathBuf);

impl Repo {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("watershed-run-diff-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("repository directory");
        let repo = Self(dir);
        repo.git(&["init", "-q"]);
        repo.write("a.txt", "old\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "base"]);
        repo
    }

    fn git(&self, args: &[&str]) {
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
            .output()
            .expect("git runs");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
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

fn run<F: std::future::Future>(test: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime starts")
        .block_on(test)
}

fn modified(path: &str) -> FileChange {
    FileChange {
        path: path.to_owned(),
        status: ChangeStatus::Modified,
        added: 1,
        removed: 1,
    }
}

#[test]
fn a_run_compares_against_the_project_before_it_until_the_next_run_starts() {
    run(async {
        let repo = Repo::new("runs");
        let diff = RunDiff::open(repo.path()).await;
        assert_eq!(diff.changes().await.expect("changes"), [], "no run yet");

        let capture = diff.capture().await;
        diff.started(capture);
        repo.write("a.txt", "new\n");
        assert_eq!(diff.changes().await.expect("changes"), [modified("a.txt")]);
        let patch = diff.patch("a.txt").await.expect("patch");
        assert!(
            patch.contains("-old\n") && patch.contains("+new\n"),
            "{patch}"
        );

        // A message the agent rejects starts no run, so its capture never counts.
        let rejected = diff.capture().await;
        drop(rejected);
        assert_eq!(diff.changes().await.expect("changes"), [modified("a.txt")]);
    });
}

#[test]
fn a_session_switch_ends_the_diff_unless_a_run_started_meanwhile() {
    run(async {
        let repo = Repo::new("switch");
        let diff = RunDiff::open(repo.path()).await;
        diff.started(diff.capture().await);
        repo.write("a.txt", "new\n");

        let runs = diff.runs();
        diff.started(diff.capture().await);
        repo.write("a.txt", "newer\n");
        diff.switched(runs);
        assert_eq!(
            diff.changes().await.expect("changes"),
            [modified("a.txt")],
            "the run that started during the switch keeps its diff"
        );

        diff.switched(diff.runs());
        assert_eq!(diff.changes().await.expect("changes"), []);
    });
}

#[test]
fn a_project_outside_git_says_so_instead_of_showing_no_changes() {
    run(async {
        let dir =
            std::env::temp_dir().join(format!("watershed-run-diff-no-git-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("directory");
        let diff = RunDiff::open(&dir).await;
        diff.started(diff.capture().await);
        std::fs::write(dir.join("a.txt"), "new\n").expect("file");

        assert!(matches!(
            diff.changes().await,
            Err(DiffError::NotARepository)
        ));
        assert!(matches!(
            diff.patch("a.txt").await,
            Err(DiffError::NotARepository)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    });
}

#[test]
fn a_project_that_could_not_be_captured_is_reported_instead_of_unchanged() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        let repo = Repo::new("unreadable");
        let secret = repo.path().join("secret.txt");
        std::fs::write(&secret, "x\n").expect("file");
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o000)).expect("mode");
        let diff = RunDiff::open(repo.path()).await;
        diff.started(diff.capture().await);
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).expect("mode");

        let error = diff.changes().await.expect_err("no diff without a capture");
        assert!(matches!(error, DiffError::NotCaptured(_)), "{error}");
        assert!(diff.patch("a.txt").await.is_err());
    });
}

#[test]
fn a_stalled_git_gives_up_and_leaves_later_snapshots_working() {
    run(async {
        let repo = Repo::new("stalled");
        // A clean filter runs during `git add`; this one takes far longer than the limit.
        repo.git(&["config", "filter.slow.clean", "sleep 5; cat"]);
        repo.write(".gitattributes", "*.txt filter=slow\n");
        let diff = RunDiff::with_timeout(repo.path(), Duration::from_millis(300)).await;
        repo.write("a.txt", "new\n");

        let capture = tokio::time::timeout(Duration::from_secs(4), diff.capture())
            .await
            .expect("the capture gives up within its limit");
        diff.started(capture);
        assert!(matches!(
            diff.changes().await,
            Err(DiffError::NotCaptured(_))
        ));

        repo.git(&["config", "--unset", "filter.slow.clean"]);
        diff.started(diff.capture().await);
        repo.write("a.txt", "newer\n");
        assert_eq!(
            diff.changes().await.expect("a later snapshot works"),
            [modified("a.txt")]
        );
    });
}

#[test]
fn closing_removes_the_private_snapshots() {
    run(async {
        let repo = Repo::new("close");
        let diff = RunDiff::open(repo.path()).await;
        diff.started(diff.capture().await);
        let store = diff.store().expect("a Git project has a store");
        assert!(store.exists());

        diff.close().await;
        assert!(!store.exists(), "{}", store.display());
        assert_eq!(diff.changes().await.expect("changes"), []);
    });
}

#[test]
fn closing_stops_a_read_in_progress_and_removes_its_snapshots() {
    run(async {
        let repo = Repo::new("close-slow");
        repo.git(&["config", "filter.slow.clean", "sleep 5; cat"]);
        repo.write(".gitattributes", "*.txt filter=slow\n");
        let diff = std::sync::Arc::new(RunDiff::open(repo.path()).await);
        let store = diff.store().expect("a Git project has a store");
        repo.write("a.txt", "new\n");
        let reading = tokio::spawn({
            let diff = diff.clone();
            async move { diff.capture().await }
        });
        tokio::time::sleep(Duration::from_millis(300)).await;

        tokio::time::timeout(Duration::from_secs(2), diff.close())
            .await
            .expect("closing does not wait for the slow read");
        assert!(!store.exists(), "{}", store.display());
        diff.started(reading.await.expect("the read ends"));
        assert!(
            diff.changes().await.expect("closed").is_empty(),
            "a closed diff shows nothing"
        );
    });
}
