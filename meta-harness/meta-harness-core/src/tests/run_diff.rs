use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::{ChangeStatus, DiffError, FileChange, RunDiff, Whitespace};

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
        assert_eq!(
            diff.changes(Whitespace::Shown).await.expect("changes"),
            [],
            "no run yet"
        );

        let capture = diff.capture().await;
        diff.started(capture);
        repo.write("a.txt", "new\n");
        assert_eq!(
            diff.changes(Whitespace::Shown).await.expect("changes"),
            [modified("a.txt")]
        );
        let patch = diff.patch("a.txt", Whitespace::Shown).await.expect("patch");
        assert!(
            patch.contains("-old\n") && patch.contains("+new\n"),
            "{patch}"
        );

        // A message the agent rejects starts no run, so its capture never counts.
        let rejected = diff.capture().await;
        drop(rejected);
        assert_eq!(
            diff.changes(Whitespace::Shown).await.expect("changes"),
            [modified("a.txt")]
        );
    });
}

#[test]
fn a_patch_shows_a_file_as_the_list_of_changes_saw_it() {
    run(async {
        let repo = Repo::new("patch-agrees");
        let diff = RunDiff::open(repo.path()).await;
        diff.started(diff.capture().await);
        repo.write("a.txt", "new\n");
        assert_eq!(
            diff.changes(Whitespace::Shown).await.expect("changes"),
            [modified("a.txt")]
        );
        // Reading one file compares the same states, without a new pass over the project.
        repo.write("a.txt", "newer\n");
        let patch = diff.patch("a.txt", Whitespace::Shown).await.expect("patch");
        assert!(
            patch.contains("+new\n") && !patch.contains("newer"),
            "{patch}"
        );

        // A new run leaves the list as it was until it is read again, and its patches too.
        diff.started(diff.capture().await);
        repo.write("a.txt", "newest\n");
        let patch = diff.patch("a.txt", Whitespace::Shown).await.expect("patch");
        assert!(patch.contains("-old\n+new\n"), "{patch}");
        assert_eq!(
            diff.changes(Whitespace::Shown).await.expect("changes"),
            [modified("a.txt")]
        );
        let patch = diff.patch("a.txt", Whitespace::Shown).await.expect("patch");
        assert!(patch.contains("-newer\n+newest\n"), "{patch}");
    });
}

#[test]
fn a_git_that_does_not_answer_at_open_is_reported() {
    run(async {
        let repo = Repo::new("open-limit");
        // No time for Git to answer, as a hung Git would not.
        let diff = RunDiff::with_timeout(repo.path(), Duration::ZERO).await;
        assert!(matches!(
            diff.changes(Whitespace::Shown).await,
            Err(DiffError::NotCaptured(_))
        ));
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
            diff.changes(Whitespace::Shown).await.expect("changes"),
            [modified("a.txt")],
            "the run that started during the switch keeps its diff"
        );

        diff.switched(diff.runs());
        assert_eq!(diff.changes(Whitespace::Shown).await.expect("changes"), []);
    });
}

#[test]
fn a_read_that_ends_after_a_session_switch_leaves_the_run_ended() {
    run(async {
        let repo = Repo::new("switch-during-read");
        // The project's clean filter says when it runs and then waits for `gate`, so a read can
        // be held inside its snapshot. Git runs it at the top of the work tree.
        repo.git(&[
            "config",
            "filter.gate.clean",
            "touch .git/waiting; until [ -e .git/gate ]; do sleep 0.01; done; cat",
        ]);
        repo.write(".gitattributes", "*.txt filter=gate\n");
        let gate = repo.path().join(".git/gate");
        let waiting = repo.path().join(".git/waiting");
        std::fs::write(&gate, "").expect("gate opens");
        let diff = std::sync::Arc::new(RunDiff::open(repo.path()).await);
        diff.started(diff.capture().await);
        repo.write("a.txt", "new\n");
        assert_eq!(
            diff.changes(Whitespace::Shown).await.expect("changes"),
            [modified("a.txt")]
        );

        std::fs::remove_file(&gate).expect("gate closes");
        let _ = std::fs::remove_file(&waiting);
        repo.write("a.txt", "newer\n");
        let reading = tokio::spawn({
            let diff = diff.clone();
            async move { diff.changes(Whitespace::Shown).await }
        });
        let held = async {
            while !waiting.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), held)
            .await
            .expect("the read reaches its snapshot");
        diff.switched(diff.runs());
        std::fs::write(&gate, "").expect("gate opens");
        let _ = reading.await.expect("the read ends");
        assert_eq!(
            diff.patch("a.txt", Whitespace::Shown).await.expect("patch"),
            "",
            "the switch ended the run, so the read that ended later keeps nothing for patches"
        );
    });
}

#[test]
fn a_list_that_failed_leaves_patches_as_the_last_list_saw_them() {
    run(async {
        let repo = Repo::new("failed-list");
        let diff = RunDiff::open(repo.path()).await;
        diff.started(diff.capture().await);
        repo.write("a.txt", "new\n");
        assert_eq!(
            diff.changes(Whitespace::Shown).await.expect("changes"),
            [modified("a.txt")]
        );

        // Git still captures the project with this setting but cannot compare it.
        repo.git(&["config", "diff.algorithm", "unknown"]);
        repo.write("a.txt", "newer\n");
        assert!(matches!(
            diff.changes(Whitespace::Shown).await,
            Err(DiffError::Git(_))
        ));
        repo.git(&["config", "--unset", "diff.algorithm"]);
        let patch = diff.patch("a.txt", Whitespace::Shown).await.expect("patch");
        assert!(
            patch.contains("+new\n") && !patch.contains("newer"),
            "{patch}"
        );
    });
}

#[test]
fn project_settings_that_skip_files_hide_no_change() {
    run(async {
        // Files the project marks as unchanged or as outside its work, and a setting that
        // trusts the index over the files; either way Git would skip the file.
        let marked = Repo::new("assume-unchanged");
        marked.git(&["update-index", "--assume-unchanged", "a.txt"]);
        let skipped = Repo::new("skip-worktree");
        skipped.write("b.txt", "other\n");
        skipped.git(&["add", "b.txt"]);
        skipped.git(&["commit", "-q", "-m", "b"]);
        skipped.git(&["update-index", "--skip-worktree", "a.txt"]);
        let trusting = Repo::new("ignore-stat");
        trusting.git(&["config", "core.ignoreStat", "true"]);
        for repo in [marked, skipped, trusting] {
            let diff = RunDiff::open(repo.path()).await;
            diff.started(diff.capture().await);
            repo.write("a.txt", "new\n");
            assert_eq!(
                diff.changes(Whitespace::Shown).await.expect("changes"),
                [modified("a.txt")],
                "{}",
                repo.path().display()
            );
            repo.write("a.txt", "newer\n");
            diff.started(diff.capture().await);
            repo.write("a.txt", "newest\n");
            assert_eq!(
                diff.changes(Whitespace::Shown).await.expect("changes"),
                [modified("a.txt")],
                "a later run too: {}",
                repo.path().display()
            );
        }
    });
}

#[test]
fn a_file_made_executable_shows_although_the_project_ignores_modes() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        let repo = Repo::new("file-mode");
        repo.git(&["config", "core.fileMode", "false"]);
        let diff = RunDiff::open(repo.path()).await;
        diff.started(diff.capture().await);
        let file = repo.path().join("a.txt");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).expect("mode");
        assert_eq!(
            diff.changes(Whitespace::Shown).await.expect("changes"),
            [FileChange {
                added: 0,
                removed: 0,
                ..modified("a.txt")
            }]
        );
    });
}

#[test]
fn a_first_snapshot_that_failed_still_clears_the_projects_marks() {
    run(async {
        let repo = Repo::new("unmark-retry");
        repo.git(&["update-index", "--assume-unchanged", "a.txt"]);
        // The first snapshot runs out of time while it clears the copied index's marks.
        let mut diff = RunDiff::open(repo.path()).await;
        diff.set_timeout(Duration::ZERO);
        diff.started(diff.capture().await);
        assert!(matches!(
            diff.changes(Whitespace::Shown).await,
            Err(DiffError::NotCaptured(_))
        ));

        diff.set_timeout(Duration::from_secs(60));
        diff.started(diff.capture().await);
        repo.write("a.txt", "new\n");
        assert_eq!(
            diff.changes(Whitespace::Shown).await.expect("changes"),
            [modified("a.txt")],
            "the marks are cleared on the next snapshot"
        );
    });
}

#[test]
fn snapshots_write_nothing_into_the_projects_git_directory() {
    run(async {
        let repo = Repo::new("split-index");
        repo.git(&["config", "core.splitIndex", "true"]);
        let listing = || {
            let mut names: Vec<_> = std::fs::read_dir(repo.path().join(".git"))
                .expect(".git")
                .map(|entry| entry.expect("entry").file_name())
                .collect();
            names.sort();
            names
        };
        let before = listing();
        let diff = RunDiff::open(repo.path()).await;
        diff.started(diff.capture().await);
        repo.write("a.txt", "new\n");
        assert_eq!(
            diff.changes(Whitespace::Shown).await.expect("changes"),
            [modified("a.txt")]
        );
        assert_eq!(listing(), before);
    });
}

#[test]
fn a_submodules_diff_runs_none_of_its_commands() {
    run(async {
        let sub = Repo::new("submodule-inner");
        // The submodule's own textconv would write a marker when its diff runs.
        let marker = sub.path().join("ran");
        let textconv = format!("touch {}; cat", super::shell_word(marker.display()));
        sub.git(&["config", "diff.mark.textconv", &textconv]);
        sub.write(".gitattributes", "*.txt diff=mark\n");
        sub.git(&["add", "."]);
        sub.git(&["commit", "-q", "-m", "attributes"]);
        let repo = Repo::new("submodule-outer");
        repo.git(&["config", "diff.submodule", "diff"]);
        repo.git(&[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            &sub.path().display().to_string(),
            "sub",
        ]);
        repo.git(&["commit", "-q", "-m", "submodule"]);
        let inner = repo.path().join("sub");
        let git_in = |args: &[&str]| {
            let status = Command::new("git")
                .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
                .args(args)
                .current_dir(&inner)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .status()
                .expect("git runs");
            assert!(status.success(), "{args:?}");
        };
        git_in(&["config", "diff.mark.textconv", &textconv]);
        let diff = RunDiff::open(repo.path()).await;
        diff.started(diff.capture().await);
        std::fs::write(inner.join("a.txt"), "changed\n").expect("file");
        git_in(&["commit", "-q", "-am", "inner change"]);

        let changes = diff.changes(Whitespace::Shown).await.expect("changes");
        assert_eq!(changes.len(), 1, "{changes:?}");
        let _ = diff.patch("sub", Whitespace::Shown).await.expect("patch");
        assert!(!marker.exists(), "the submodule's textconv ran");
        git_in(&["diff", "--output=/dev/null", "HEAD~1", "HEAD"]);
        assert!(marker.exists(), "the textconv runs where Git uses it");
    });
}

#[test]
fn reads_that_wait_for_each_other_count_only_their_own_git_against_the_limit() {
    run(async {
        let repo = Repo::new("queued");
        // Each snapshot takes about 0.4 s in this project's clean filter.
        repo.git(&["config", "filter.slow.clean", "sleep 0.4; cat"]);
        repo.write(".gitattributes", "*.txt filter=slow\n");
        let diff = std::sync::Arc::new(
            RunDiff::with_timeout(repo.path(), Duration::from_millis(1500)).await,
        );
        diff.started(diff.capture().await);
        // The file keeps changing, so every snapshot runs the filter again.
        let file = repo.path().join("a.txt");
        let writing = std::thread::spawn(move || {
            for round in 0..60 {
                std::fs::write(&file, format!("round {round}\n")).expect("file");
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let reads: Vec<_> = (0..5)
            .map(|_| {
                let diff = diff.clone();
                tokio::spawn(async move {
                    diff.changes(Whitespace::Shown)
                        .await
                        .map(|changes| changes.len())
                        .map_err(|error| error.to_string())
                })
            })
            .collect();
        for read in reads {
            assert_eq!(read.await.expect("read ends"), Ok(1));
        }
        writing.join().expect("writer ends");
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
            diff.changes(Whitespace::Shown).await,
            Err(DiffError::NotARepository)
        ));
        assert!(matches!(
            diff.patch("a.txt", Whitespace::Shown).await,
            Err(DiffError::NotARepository)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    });
}

#[test]
fn a_git_that_cannot_read_the_project_is_reported_instead_of_outside_git() {
    run(async {
        let repo = Repo::new("broken-config");
        let config = repo.path().join(".git/config");
        let mut text = std::fs::read_to_string(&config).expect("config");
        text.push_str("[broken\n");
        std::fs::write(&config, text).expect("config");
        let diff = RunDiff::open(repo.path()).await;
        let error = diff
            .changes(Whitespace::Shown)
            .await
            .expect_err("Git cannot read the project");
        assert!(
            error.to_string().contains("bad config"),
            "Git's own reason: {error}"
        );
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

        let error = diff
            .changes(Whitespace::Shown)
            .await
            .expect_err("no diff without a capture");
        assert!(matches!(error, DiffError::NotCaptured(_)), "{error}");
        assert!(diff.patch("a.txt", Whitespace::Shown).await.is_err());
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
            diff.changes(Whitespace::Shown).await,
            Err(DiffError::NotCaptured(_))
        ));

        repo.git(&["config", "--unset", "filter.slow.clean"]);
        diff.started(diff.capture().await);
        repo.write("a.txt", "newer\n");
        assert_eq!(
            diff.changes(Whitespace::Shown)
                .await
                .expect("a later snapshot works"),
            [modified("a.txt")]
        );
    });
}

/// Captures until the project's clean filter wrote a program's id to `pid`, then tells whether
/// that program still runs after Git was stopped at its limit; one that does is killed.
async fn outlives_its_git(diff: &RunDiff, pid: &Path) -> bool {
    let started = std::time::Instant::now();
    while !pid.exists() && started.elapsed() < Duration::from_secs(4) {
        diff.started(diff.capture().await);
    }
    let program = std::fs::read_to_string(pid).expect("the filter ran");
    let _ = std::fs::remove_file(pid);
    let alive = || {
        std::process::Command::new("kill")
            .args(["-0", program.trim()])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    };
    let stopped = std::time::Instant::now();
    while alive() && stopped.elapsed() < Duration::from_secs(2) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let survived = alive();
    if survived {
        let _ = std::process::Command::new("kill")
            .args(["-9", program.trim()])
            .status();
    }
    survived
}

#[test]
fn a_git_past_its_limit_ends_with_the_programs_it_started() {
    run(async {
        let repo = Repo::new("group");
        let pid = std::env::temp_dir().join(format!("watershed-filter-{}", std::process::id()));
        // The project's clean filter starts a program that would outlive Git.
        repo.git(&[
            "config",
            "filter.slow.clean",
            &format!(
                "echo $$ > {}; exec sleep 30",
                super::shell_word(pid.display())
            ),
        ]);
        repo.write(".gitattributes", "*.txt filter=slow\n");
        repo.write("a.txt", "new\n");
        let diff = RunDiff::with_timeout(repo.path(), Duration::from_millis(300)).await;
        assert!(
            !outlives_its_git(&diff, &pid).await,
            "the filter outlived its Git"
        );
    });
}

#[test]
fn a_failed_git_past_its_limit_ends_the_programs_that_hold_its_errors() {
    run(async {
        let repo = Repo::new("error-holder");
        // The project's required clean filter fails at once but leaves a program that holds
        // Git's error output. Git runs it at the top of the work tree.
        repo.git(&[
            "config",
            "filter.leaky.clean",
            "sleep 30 >&2 & echo $! > .git/holder; exit 1",
        ]);
        repo.git(&["config", "filter.leaky.required", "true"]);
        repo.write(".gitattributes", "*.txt filter=leaky\n");
        repo.write("a.txt", "new\n");
        let diff = RunDiff::with_timeout(repo.path(), Duration::from_millis(300)).await;
        assert!(
            !outlives_its_git(&diff, &repo.path().join(".git/holder")).await,
            "the program holding Git's errors outlived it"
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
        assert_eq!(diff.changes(Whitespace::Shown).await.expect("changes"), []);
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
            diff.changes(Whitespace::Shown)
                .await
                .expect("closed")
                .is_empty(),
            "a closed diff shows nothing"
        );
    });
}
