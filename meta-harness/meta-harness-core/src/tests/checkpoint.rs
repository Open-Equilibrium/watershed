use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::{
    ChangeStatus, Checkpoint, Checkpoints, DiffError, FileChange, Phase, RevertPlan, Whitespace,
};

/// A throwaway Git repository with one committed file, `a.txt`.
pub(super) struct Repo(pub(super) PathBuf);

impl Repo {
    pub(super) fn new(name: &str) -> Self {
        Self::at(std::env::temp_dir().join(format!(
            "watershed-checkpoint-{name}-{}",
            std::process::id()
        )))
    }

    /// A repository at `dir` with `a.txt` committed as `old`.
    pub(super) fn at(dir: PathBuf) -> Self {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("repository directory");
        let repo = Self(dir);
        repo.git(&["init", "-q"]);
        repo.write("a.txt", "old\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "base"]);
        repo
    }

    pub(super) fn git(&self, args: &[&str]) -> String {
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
        String::from_utf8(output.stdout).expect("UTF-8 output")
    }

    pub(super) fn write(&self, path: &str, text: &str) {
        std::fs::write(self.0.join(path), text).expect("file written");
    }

    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(super) fn run<F: std::future::Future>(test: F) -> F::Output {
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

/// Turn `turn`'s checkpoint at `phase` in the session `s1`.
pub(super) fn at(turn: &str, phase: Phase) -> Checkpoint {
    Checkpoint {
        session: "s1".to_owned(),
        turn: turn.to_owned(),
        phase,
    }
}

/// Takes turn `turn`'s start checkpoint, lets `work` change the project and takes its end.
async fn turn(checkpoints: &Checkpoints, turn: &str, work: impl FnOnce()) {
    checkpoints
        .capture(&at(turn, Phase::Start))
        .await
        .expect("start captured");
    work();
    checkpoints
        .capture(&at(turn, Phase::End))
        .await
        .expect("end captured");
}

async fn changes_of(checkpoints: &Checkpoints, id: &str) -> Result<Vec<FileChange>, DiffError> {
    checkpoints
        .changes(
            &at(id, Phase::Start),
            &at(id, Phase::End),
            Whitespace::Shown,
        )
        .await
}

#[test]
fn a_turn_compares_its_end_with_its_start() {
    run(async {
        let repo = Repo::new("turns");
        let checkpoints = Checkpoints::new(repo.path());
        assert!(matches!(checkpoints.in_repository().await, Ok(true)));
        turn(&checkpoints, "t1", || repo.write("a.txt", "new\n")).await;
        // Later changes belong to no turn yet.
        repo.write("a.txt", "newer\n");
        assert_eq!(
            changes_of(&checkpoints, "t1").await.expect("changes"),
            [modified("a.txt")]
        );
        let patch = checkpoints
            .patch(
                &at("t1", Phase::Start),
                &at("t1", Phase::End),
                "a.txt",
                Whitespace::Shown,
            )
            .await
            .expect("patch");
        assert!(patch.contains("-old\n+new\n"), "{patch}");

        // Changes between turns, such as the user's own edits, belong to neither.
        turn(&checkpoints, "t2", || repo.write("a.txt", "newest\n")).await;
        let patch = checkpoints
            .patch(
                &at("t2", Phase::Start),
                &at("t2", Phase::End),
                "a.txt",
                Whitespace::Shown,
            )
            .await
            .expect("patch");
        assert!(patch.contains("-newer\n+newest\n"), "{patch}");
        // All turns: the first turn's start against the last turn's end.
        let all = checkpoints
            .patch(
                &at("t1", Phase::Start),
                &at("t2", Phase::End),
                "a.txt",
                Whitespace::Shown,
            )
            .await
            .expect("patch");
        assert!(all.contains("-old\n+newest\n"), "{all}");
    });
}

#[test]
fn checkpoints_are_commits_under_watershed_refs_that_leave_the_project_alone() {
    run(async {
        let repo = Repo::new("refs");
        repo.write(".gitignore", "secret.env\n");
        repo.git(&["add", ".gitignore"]);
        repo.git(&["commit", "-q", "-m", "ignore"]);
        repo.write("secret.env", "TOKEN=1\n");
        repo.write("b.txt", "untracked\n");
        repo.git(&["branch", "feature"]);
        let index = std::fs::read(repo.path().join(".git/index")).expect("index");
        let (head, status) = (
            repo.git(&["rev-parse", "HEAD"]),
            repo.git(&["status", "--porcelain"]),
        );
        let branches = repo.git(&["for-each-ref", "refs/heads", "refs/tags"]);

        let checkpoints = Checkpoints::new(repo.path());
        checkpoints
            .capture(&at("t1", Phase::Start))
            .await
            .expect("captured");

        let reference = "refs/watershed/checkpoints/s1/t1/start";
        assert_eq!(repo.git(&["rev-parse", &format!("{reference}^")]), head);
        let files = repo.git(&["ls-tree", "-r", "--name-only", reference]);
        assert_eq!(
            files.lines().collect::<Vec<_>>(),
            [".gitignore", "a.txt", "b.txt"],
            "new files count, ignored ones stay out"
        );
        assert_eq!(
            std::fs::read(repo.path().join(".git/index")).expect("index"),
            index
        );
        assert_eq!(repo.git(&["rev-parse", "HEAD"]), head);
        assert_eq!(repo.git(&["status", "--porcelain"]), status);
        assert_eq!(
            repo.git(&["for-each-ref", "refs/heads", "refs/tags"]),
            branches
        );
        assert!(
            repo.git(&["log", "--all", "--oneline"])
                .contains("Watershed checkpoint"),
            "the commit exists"
        );
        assert!(
            !repo
                .git(&["log", "--oneline", "feature"])
                .contains("Watershed checkpoint"),
            "it sits on no branch"
        );
    });
}

#[test]
fn checkpoints_run_none_of_the_projects_hooks() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        let repo = Repo::new("hooks");
        for hook in ["reference-transaction", "post-index-change", "post-commit"] {
            let path = repo.path().join(".git/hooks").join(hook);
            std::fs::write(&path, format!("#!/bin/sh\ntouch ran-{hook}\n")).expect("hook");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("mode");
        }
        let checkpoints = Checkpoints::new(repo.path());
        turn(&checkpoints, "t1", || repo.write("a.txt", "new\n")).await;
        let ran: Vec<_> = std::fs::read_dir(repo.path())
            .expect("project")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("ran-"))
            .collect();
        assert!(ran.is_empty(), "{ran:?}");
    });
}

#[test]
fn a_file_ignored_after_an_earlier_checkpoint_stays_out_of_later_ones() {
    run(async {
        let repo = Repo::new("newly-ignored");
        repo.write("notes.txt", "harmless\n");
        let checkpoints = Checkpoints::new(repo.path());
        checkpoints
            .capture(&at("t1", Phase::Start))
            .await
            .expect("captured");
        // The project now ignores the file, which holds a secret.
        repo.write(".gitignore", "notes.txt\n");
        repo.write("notes.txt", "TOKEN=secret\n");
        checkpoints
            .capture(&at("t2", Phase::Start))
            .await
            .expect("captured");
        let files = repo.git(&[
            "ls-tree",
            "-r",
            "--name-only",
            "refs/watershed/checkpoints/s1/t2/start",
        ]);
        assert_eq!(files.lines().collect::<Vec<_>>(), [".gitignore", "a.txt"]);
    });
}

#[test]
fn a_project_whose_path_holds_a_line_break_keeps_its_checkpoints() {
    run(async {
        // Read line by line, the project's path would name the repository `outer`.
        let base = std::env::temp_dir().join(format!("watershed-newline-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("base");
        let outer = Repo::at(base.join("outer"));
        let project = Repo::at(base.join("outer\nproject"));
        let checkpoints = Checkpoints::new(project.path());
        turn(&checkpoints, "t1", || project.write("a.txt", "new\n")).await;
        assert_eq!(
            changes_of(&checkpoints, "t1").await.expect("changes"),
            [modified("a.txt")]
        );
        assert_eq!(outer.git(&["for-each-ref", "refs/watershed"]), "");
        assert_ne!(project.git(&["for-each-ref", "refs/watershed"]), "");
        drop((outer, project));
        let _ = std::fs::remove_dir_all(&base);
    });
}

#[test]
fn a_project_without_commits_gets_checkpoints_without_a_parent() {
    run(async {
        let dir =
            std::env::temp_dir().join(format!("watershed-checkpoint-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("directory");
        let repo = Repo(dir);
        repo.git(&["init", "-q"]);
        let checkpoints = Checkpoints::new(repo.path());
        turn(&checkpoints, "t1", || repo.write("a.txt", "new\n")).await;
        assert_eq!(
            changes_of(&checkpoints, "t1").await.expect("changes"),
            [FileChange {
                path: "a.txt".to_owned(),
                status: ChangeStatus::Added,
                added: 1,
                removed: 0,
            }]
        );
    });
}

#[test]
fn ids_that_cannot_name_a_ref_are_refused() {
    run(async {
        let repo = Repo::new("ids");
        let checkpoints = Checkpoints::new(repo.path());
        let long = "x".repeat(129);
        for (session, turn) in [
            ("..", "t"),
            ("s/../x", "t"),
            ("s", "a/b"),
            ("s", ""),
            ("s", "t.lock"),
            ("s", "t\n"),
            ("s", "@{1}"),
            ("s", long.as_str()),
        ] {
            let checkpoint = Checkpoint {
                session: session.to_owned(),
                turn: turn.to_owned(),
                phase: Phase::Start,
            };
            let refused = checkpoints.capture(&checkpoint).await;
            assert!(
                matches!(refused, Err(DiffError::InvalidCheckpoint(_))),
                "{session:?} {turn:?}: {refused:?}"
            );
        }
        assert_eq!(repo.git(&["for-each-ref", "refs/watershed"]), "");
    });
}

#[test]
fn a_missing_checkpoint_is_reported_instead_of_unchanged() {
    run(async {
        let repo = Repo::new("missing");
        let checkpoints = Checkpoints::new(repo.path());
        checkpoints
            .capture(&at("t1", Phase::Start))
            .await
            .expect("start captured");
        assert!(matches!(
            changes_of(&checkpoints, "t1").await,
            Err(DiffError::NoCheckpoint)
        ));
    });
}

#[test]
fn a_git_that_does_not_answer_at_open_is_reported() {
    run(async {
        let repo = Repo::new("open-limit");
        // No time for Git to answer, as a hung Git would not.
        let checkpoints = Checkpoints::with_timeout(repo.path(), Duration::ZERO);
        assert!(matches!(
            checkpoints.capture(&at("t1", Phase::Start)).await,
            Err(DiffError::NotCaptured(_))
        ));
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
            let checkpoints = Checkpoints::new(repo.path());
            turn(&checkpoints, "t1", || repo.write("a.txt", "new\n")).await;
            repo.write("a.txt", "newer\n");
            turn(&checkpoints, "t2", || repo.write("a.txt", "newest\n")).await;
            for id in ["t1", "t2"] {
                assert_eq!(
                    changes_of(&checkpoints, id).await.expect("changes"),
                    [modified("a.txt")],
                    "{id} in {}",
                    repo.path().display()
                );
            }
        }
    });
}

#[test]
fn a_file_made_executable_shows_although_the_project_ignores_modes() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        let repo = Repo::new("file-mode");
        repo.git(&["config", "core.fileMode", "false"]);
        let checkpoints = Checkpoints::new(repo.path());
        let file = repo.path().join("a.txt");
        turn(&checkpoints, "t1", || {
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).expect("mode")
        })
        .await;
        assert_eq!(
            changes_of(&checkpoints, "t1").await.expect("changes"),
            [FileChange {
                added: 0,
                removed: 0,
                ..modified("a.txt")
            }]
        );
    });
}

#[test]
fn a_first_capture_that_failed_still_clears_the_projects_marks() {
    run(async {
        let repo = Repo::new("unmark-retry");
        repo.git(&["update-index", "--assume-unchanged", "a.txt"]);
        let mut checkpoints = Checkpoints::new(repo.path());
        // Opened in time; then the first capture runs out of time while it clears the marks.
        let _ = checkpoints.store().await;
        checkpoints.set_timeout(Duration::ZERO);
        assert!(checkpoints.capture(&at("t0", Phase::Start)).await.is_err());

        checkpoints.set_timeout(Duration::from_secs(60));
        turn(&checkpoints, "t1", || repo.write("a.txt", "new\n")).await;
        assert_eq!(
            changes_of(&checkpoints, "t1").await.expect("changes"),
            [modified("a.txt")],
            "the marks are cleared on the next capture"
        );
    });
}

#[test]
fn checkpoints_add_only_objects_refs_and_their_locks_to_the_projects_git_directory() {
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
        let checkpoints = Checkpoints::new(repo.path());
        turn(&checkpoints, "t1", || repo.write("a.txt", "new\n")).await;
        assert_eq!(
            changes_of(&checkpoints, "t1").await.expect("changes"),
            [modified("a.txt")]
        );
        // Besides objects and refs, only the locks that tell which agents run there.
        let mut expected = before;
        expected.push("watershed".into());
        expected.sort();
        assert_eq!(listing(), expected);
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
        let checkpoints = Checkpoints::new(repo.path());
        turn(&checkpoints, "t1", || {
            std::fs::write(inner.join("a.txt"), "changed\n").expect("file");
            git_in(&["commit", "-q", "-am", "inner change"]);
        })
        .await;

        let changes = changes_of(&checkpoints, "t1").await.expect("changes");
        assert_eq!(changes.len(), 1, "{changes:?}");
        let _ = checkpoints
            .patch(
                &at("t1", Phase::Start),
                &at("t1", Phase::End),
                "sub",
                Whitespace::Shown,
            )
            .await
            .expect("patch");
        assert!(!marker.exists(), "the submodule's textconv ran");
        git_in(&["diff", "--output=/dev/null", "HEAD~1", "HEAD"]);
        assert!(marker.exists(), "the textconv runs where Git uses it");
    });
}

#[test]
fn captures_that_wait_for_each_other_count_only_their_own_git_against_the_limit() {
    run(async {
        let repo = Repo::new("queued");
        let limit = Duration::from_secs(2);
        let checkpoints = std::sync::Arc::new(Checkpoints::with_timeout(repo.path(), limit));
        let _ = checkpoints.store().await;
        // Another capture holds the turn for longer than one capture's limit.
        let held = checkpoints.hold().await;
        let waiting = tokio::spawn({
            let checkpoints = checkpoints.clone();
            async move { checkpoints.capture(&at("t1", Phase::Start)).await }
        });
        tokio::time::sleep(limit + Duration::from_millis(500)).await;
        assert!(!waiting.is_finished(), "the capture waits for its turn");
        drop(held);
        assert!(
            matches!(waiting.await.expect("capture ends"), Ok(())),
            "waiting did not count against the capture's limit"
        );
    });
}

#[test]
fn a_project_outside_git_says_so_instead_of_showing_no_changes() {
    run(async {
        let dir = std::env::temp_dir().join(format!(
            "watershed-checkpoint-no-git-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("directory");
        let checkpoints = Checkpoints::new(&dir);
        assert!(matches!(checkpoints.in_repository().await, Ok(false)));
        assert!(matches!(
            checkpoints.capture(&at("t1", Phase::Start)).await,
            Err(DiffError::NotARepository)
        ));
        assert!(matches!(
            changes_of(&checkpoints, "t1").await,
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
        let checkpoints = Checkpoints::new(repo.path());
        let error = checkpoints
            .capture(&at("t1", Phase::Start))
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
        let checkpoints = Checkpoints::new(repo.path());
        let error = checkpoints
            .capture(&at("t1", Phase::Start))
            .await
            .expect_err("Git cannot read the file");
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).expect("mode");
        assert!(matches!(error, DiffError::Git(_)), "{error}");
        assert_eq!(repo.git(&["for-each-ref", "refs/watershed"]), "");
    });
}

#[test]
fn a_stalled_git_gives_up_and_leaves_later_captures_working() {
    run(async {
        let repo = Repo::new("stalled");
        // A clean filter runs during `git add`; this one takes far longer than the limit.
        repo.git(&["config", "filter.slow.clean", "sleep 5; cat"]);
        repo.write(".gitattributes", "*.txt filter=slow\n");
        let checkpoints = Checkpoints::with_timeout(repo.path(), Duration::from_millis(300));
        repo.write("a.txt", "new\n");

        let capture = tokio::time::timeout(
            Duration::from_secs(4),
            checkpoints.capture(&at("t0", Phase::Start)),
        )
        .await
        .expect("the capture gives up within its limit");
        assert!(
            matches!(capture, Err(DiffError::TimedOut(_))),
            "{capture:?}"
        );

        repo.git(&["config", "--unset", "filter.slow.clean"]);
        turn(&checkpoints, "t1", || repo.write("a.txt", "newer\n")).await;
        assert_eq!(
            changes_of(&checkpoints, "t1")
                .await
                .expect("a later capture works"),
            [modified("a.txt")]
        );
    });
}

/// Captures until the project's clean filter wrote a program's id to `pid`, then tells whether
/// that program still runs after Git was stopped at its limit; one that does is killed.
async fn outlives_its_git(checkpoints: &Checkpoints, pid: &Path) -> bool {
    let started = std::time::Instant::now();
    while !pid.exists() && started.elapsed() < Duration::from_secs(4) {
        let _ = checkpoints.capture(&at("t1", Phase::Start)).await;
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
        let checkpoints = Checkpoints::with_timeout(repo.path(), Duration::from_millis(300));
        assert!(
            !outlives_its_git(&checkpoints, &pid).await,
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
        let checkpoints = Checkpoints::with_timeout(repo.path(), Duration::from_millis(300));
        assert!(
            !outlives_its_git(&checkpoints, &repo.path().join(".git/holder")).await,
            "the program holding Git's errors outlived it"
        );
    });
}

#[test]
fn closing_removes_the_private_index() {
    run(async {
        let repo = Repo::new("close");
        let checkpoints = Checkpoints::new(repo.path());
        turn(&checkpoints, "t1", || repo.write("a.txt", "new\n")).await;
        let store = checkpoints
            .store()
            .await
            .expect("a Git project has a store");
        assert!(store.exists());

        checkpoints.close().await;
        assert!(!store.exists(), "{}", store.display());
        assert!(changes_of(&checkpoints, "t1").await.is_err(), "closed");
    });
}

#[test]
fn closing_stops_a_capture_in_progress_and_removes_its_index() {
    run(async {
        let repo = Repo::new("close-slow");
        let checkpoints = std::sync::Arc::new(Checkpoints::new(repo.path()));
        let store = checkpoints
            .store()
            .await
            .expect("a Git project has a store");
        // The project's clean filter says when it runs and then waits, so the capture is held
        // inside Git when closing starts.
        repo.git(&[
            "config",
            "filter.gate.clean",
            "touch .git/waiting; sleep 30; cat",
        ]);
        repo.write(".gitattributes", "*.txt filter=gate\n");
        repo.write("a.txt", "new\n");
        let capturing = tokio::spawn({
            let checkpoints = checkpoints.clone();
            async move { checkpoints.capture(&at("t1", Phase::Start)).await }
        });
        let waiting = repo.path().join(".git/waiting");
        tokio::time::timeout(Duration::from_secs(10), async {
            while !waiting.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the capture reaches its Git");

        tokio::time::timeout(Duration::from_secs(2), checkpoints.close())
            .await
            .expect("closing does not wait for the slow capture");
        assert!(!store.exists(), "{}", store.display());
        assert!(matches!(
            capturing.await.expect("the capture ends"),
            Err(DiffError::Closed)
        ));
    });
}

/// The paths the plan reverts and those it leaves alone near a repository.
fn split(plan: &RevertPlan) -> (Vec<&str>, Vec<&str>) {
    let paths = |kept: bool| {
        plan.files
            .iter()
            .filter(|file| file.kept == kept)
            .map(|file| file.path.as_str())
            .collect()
    };
    (paths(false), paths(true))
}

/// The file `path` of the plan, with how the revert changes it and whether it changed since the
/// last turn ended.
fn planned(plan: &RevertPlan, path: &str) -> (ChangeStatus, bool) {
    let file = plan
        .files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("{path} in {plan:?}"));
    (file.status, file.edited)
}

#[test]
fn reverting_to_before_a_turn_restores_its_files_and_leaves_git_alone() {
    run(async {
        let repo = Repo::new("revert");
        repo.write(".gitignore", "*.log\n");
        repo.write("gone.txt", "kept by the turn's start\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "more"]);
        repo.write("draft.txt", "untracked\n");
        let checkpoints = Checkpoints::new(repo.path());
        turn(&checkpoints, "t1", || {
            repo.write("a.txt", "new\n");
            repo.write("b.txt", "new file\n");
            std::fs::remove_file(repo.path().join("gone.txt")).expect("removed");
            std::fs::remove_file(repo.path().join("draft.txt")).expect("removed");
            repo.write("run.log", "ignored\n");
        })
        .await;
        // The user edits a file after the turn.
        repo.write("a.txt", "newer\n");
        repo.git(&["add", "a.txt"]);
        let (index, head) = (
            std::fs::read(repo.path().join(".git/index")).expect("index"),
            repo.git(&["rev-parse", "HEAD"]),
        );

        let plan = checkpoints
            .revert_plan(&at("t1", Phase::Start), Some(&at("t1", Phase::End)))
            .await
            .expect("plan");
        assert_eq!(plan.files.len(), 4, "{plan:?}");
        assert_eq!(planned(&plan, "a.txt"), (ChangeStatus::Modified, true));
        assert_eq!(planned(&plan, "b.txt"), (ChangeStatus::Deleted, false));
        assert_eq!(planned(&plan, "gone.txt"), (ChangeStatus::Added, false));
        assert_eq!(planned(&plan, "draft.txt"), (ChangeStatus::Added, false));
        assert!(plan.running.is_empty());
        assert_eq!(
            plan.branch.as_deref(),
            Some(repo.git(&["branch", "--show-current"]).trim())
        );

        checkpoints
            .revert(&at("t1", Phase::Start))
            .await
            .expect("reverted");
        let read = |path: &str| std::fs::read_to_string(repo.path().join(path)).ok();
        assert_eq!(read("a.txt").as_deref(), Some("old\n"));
        assert_eq!(read("b.txt"), None, "new since the turn started");
        assert_eq!(
            read("gone.txt").as_deref(),
            Some("kept by the turn's start\n")
        );
        assert_eq!(read("draft.txt").as_deref(), Some("untracked\n"));
        assert_eq!(
            read("run.log").as_deref(),
            Some("ignored\n"),
            "Git ignores it"
        );
        assert_eq!(
            std::fs::read(repo.path().join(".git/index")).expect("index"),
            index,
            "the user's staging stays theirs"
        );
        assert_eq!(repo.git(&["rev-parse", "HEAD"]), head);
        assert!(
            checkpoints
                .revert_plan(&at("t1", Phase::Start), None)
                .await
                .expect("plan")
                .files
                .is_empty(),
            "nothing differs any more"
        );
    });
}

#[test]
fn a_revert_changes_only_files_below_the_working_folder() {
    run(async {
        let repo = Repo::new("revert-folder");
        std::fs::create_dir_all(repo.path().join("app")).expect("folder");
        repo.write("app/main.txt", "old\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "app"]);
        let checkpoints = Checkpoints::new(&repo.path().join("app"));
        turn(&checkpoints, "t1", || {
            repo.write("app/main.txt", "new\n");
            repo.write("app/extra.txt", "new\n");
            repo.write("a.txt", "outside\n");
        })
        .await;

        let plan = checkpoints
            .revert_plan(&at("t1", Phase::Start), None)
            .await
            .expect("plan");
        let paths: Vec<_> = plan.files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(paths, ["app/extra.txt", "app/main.txt"]);
        checkpoints
            .revert(&at("t1", Phase::Start))
            .await
            .expect("reverted");
        let read = |path: &str| std::fs::read_to_string(repo.path().join(path)).ok();
        assert_eq!(read("app/main.txt").as_deref(), Some("old\n"));
        assert_eq!(read("app/extra.txt"), None);
        assert_eq!(read("a.txt").as_deref(), Some("outside\n"));
    });
}

#[test]
fn a_revert_is_refused_while_an_agent_runs_in_the_checkout() {
    run(async {
        let repo = Repo::new("revert-running");
        let mine = Checkpoints::new(repo.path());
        turn(&mine, "t1", || repo.write("a.txt", "new\n")).await;
        // Another window's session starts a turn in the same checkout.
        let other = Checkpoints::new(repo.path());
        let theirs = |phase| Checkpoint {
            session: "s2".to_owned(),
            turn: "u1".to_owned(),
            phase,
        };
        other.capture(&theirs(Phase::Start)).await.expect("started");

        let plan = mine
            .revert_plan(&at("t1", Phase::Start), None)
            .await
            .expect("plan");
        assert_eq!(plan.running, ["s2"]);
        assert_eq!(plan.files.len(), 1, "the files still show");
        assert!(matches!(
            mine.revert(&at("t1", Phase::Start)).await,
            Err(DiffError::AgentRunning(running)) if running == ["s2"]
        ));
        assert_eq!(
            std::fs::read_to_string(repo.path().join("a.txt")).expect("a"),
            "new\n"
        );

        other.capture(&theirs(Phase::End)).await.expect("ended");
        // This session's own running turn holds it up too.
        mine.capture(&at("t2", Phase::Start))
            .await
            .expect("started");
        assert!(matches!(
            mine.revert(&at("t1", Phase::Start)).await,
            Err(DiffError::AgentRunning(running)) if running == ["s1"]
        ));
        // A session that ends, even without its turn's end, no longer runs there.
        mine.close().await;
        let later = Checkpoints::new(repo.path());
        later
            .revert(&at("t1", Phase::Start))
            .await
            .expect("reverted once nothing runs");
        assert_eq!(
            std::fs::read_to_string(repo.path().join("a.txt")).expect("a"),
            "old\n"
        );
        other.close().await;
    });
}

#[test]
fn a_revert_to_a_removed_checkpoint_is_reported() {
    run(async {
        let repo = Repo::new("revert-removed");
        let checkpoints = Checkpoints::new(repo.path());
        assert!(matches!(
            checkpoints.revert_plan(&at("t1", Phase::Start), None).await,
            Err(DiffError::NoCheckpoint)
        ));
        assert!(matches!(
            checkpoints.revert(&at("t1", Phase::Start)).await,
            Err(DiffError::NoCheckpoint)
        ));
    });
}

#[test]
fn a_turn_that_starts_during_a_revert_waits_until_the_files_are_back() {
    run(async {
        let repo = Repo::new("revert-wait");
        let checkpoints = std::sync::Arc::new(Checkpoints::new(repo.path()));
        assert!(matches!(checkpoints.in_repository().await, Ok(true)));
        // Another window reverts the checkout.
        let reverting = crate::checkout::Checkout::new(
            repo.path().join(".git/watershed"),
            repo.path().join(".git/watershed"),
        )
        .alone()
        .await
        .expect("nothing runs");
        let starting = tokio::spawn({
            let checkpoints = checkpoints.clone();
            async move { checkpoints.capture(&at("t1", Phase::Start)).await }
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!starting.is_finished(), "the turn waits for the revert");
        drop(reverting);
        tokio::time::timeout(Duration::from_secs(5), starting)
            .await
            .expect("the turn starts once the revert is done")
            .expect("joined")
            .expect("captured");
    });
}

#[test]
fn a_revert_leaves_submodules_and_their_work_alone() {
    run(async {
        let sub = Repo::new("revert-submodule-inner");
        let repo = Repo::new("revert-submodule-outer");
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
        // A common setting that makes `git restore` check submodules out too.
        repo.git(&["config", "submodule.recurse", "true"]);
        let inner = repo.path().join("sub");
        let git_in = |args: &[&str]| {
            let output = Command::new("git")
                .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
                .args(args)
                .current_dir(&inner)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .output()
                .expect("git runs");
            assert!(output.status.success(), "{args:?}");
            String::from_utf8(output.stdout).expect("UTF-8")
        };
        let checkpoints = Checkpoints::new(repo.path());
        turn(&checkpoints, "t1", || {
            std::fs::write(inner.join("a.txt"), "committed\n").expect("file");
            git_in(&["commit", "-q", "-am", "inner change"]);
            repo.write("a.txt", "new\n");
        })
        .await;
        std::fs::write(inner.join("a.txt"), "uncommitted\n").expect("file");
        let head = git_in(&["rev-parse", "HEAD"]);

        let plan = checkpoints
            .revert_plan(&at("t1", Phase::Start), None)
            .await
            .expect("plan");
        assert_eq!(
            split(&plan),
            (vec!["a.txt"], vec!["sub"]),
            "a submodule is left alone"
        );
        checkpoints
            .revert(&at("t1", Phase::Start))
            .await
            .expect("reverted");
        assert_eq!(
            std::fs::read_to_string(repo.path().join("a.txt")).expect("a"),
            "old\n"
        );
        assert_eq!(git_in(&["rev-parse", "HEAD"]), head);
        assert_eq!(
            std::fs::read_to_string(inner.join("a.txt")).expect("inner"),
            "uncommitted\n"
        );
    });
}

#[test]
fn a_revert_that_git_could_not_finish_names_the_files_left() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        let repo = Repo::new("revert-stuck");
        std::fs::create_dir_all(repo.path().join("ro")).expect("folder");
        repo.write("ro/kept.txt", "kept\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "folder"]);
        let checkpoints = Checkpoints::new(repo.path());
        turn(&checkpoints, "t1", || {
            repo.write("a.txt", "new\n");
            repo.write("ro/new.txt", "new\n");
        })
        .await;
        // Git cannot delete a file in a folder it may not write, but still exits with 0.
        let folder = repo.path().join("ro");
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).expect("mode");
        let reverted = checkpoints.revert(&at("t1", Phase::Start)).await;
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).expect("mode");
        assert!(
            matches!(&reverted, Err(DiffError::NotReverted(left)) if left == &["ro/new.txt"]),
            "{reverted:?}"
        );
        assert_eq!(
            std::fs::read_to_string(repo.path().join("a.txt")).expect("a"),
            "old\n"
        );
    });
}

#[test]
fn a_revert_lists_ignored_files_in_the_way_of_restored_ones() {
    run(async {
        let repo = Repo::new("revert-ignored");
        repo.write(".gitignore", "data/\n");
        repo.write("data", "a file\n");
        repo.git(&["add", ".gitignore", "data"]);
        repo.git(&["commit", "-q", "-m", "files"]);
        let checkpoints = Checkpoints::new(repo.path());
        turn(&checkpoints, "t1", || {
            // The file `data` becomes an ignored folder.
            std::fs::remove_file(repo.path().join("data")).expect("removed");
            std::fs::create_dir_all(repo.path().join("data")).expect("folder");
            repo.write("data/db.sqlite", "ignored\n");
        })
        .await;
        let plan = checkpoints
            .revert_plan(&at("t1", Phase::Start), None)
            .await
            .expect("plan");
        let listed: Vec<_> = plan
            .files
            .iter()
            .map(|file| (file.path.as_str(), file.status, file.ignored))
            .collect();
        assert_eq!(
            listed,
            [
                ("data", ChangeStatus::Added, false),
                ("data/", ChangeStatus::Deleted, true),
            ]
        );
    });
}

#[test]
fn a_pipe_among_the_locks_holds_up_no_revert() {
    run(async {
        let repo = Repo::new("revert-fifo");
        let checkpoints = Checkpoints::new(repo.path());
        turn(&checkpoints, "t1", || repo.write("a.txt", "new\n")).await;
        let running = repo.path().join(".git/watershed/running");
        std::fs::create_dir_all(&running).expect("folder");
        let made = Command::new("mkfifo")
            .arg(running.join("pipe"))
            .status()
            .expect("mkfifo runs");
        assert!(made.success());
        let plan = tokio::time::timeout(
            Duration::from_secs(5),
            checkpoints.revert_plan(&at("t1", Phase::Start), None),
        )
        .await
        .expect("the plan does not open the pipe")
        .expect("plan");
        assert!(plan.running.is_empty());
    });
}

#[test]
fn a_session_that_starts_turns_without_ends_still_counts_as_running() {
    run(async {
        let repo = Repo::new("revert-restart");
        let checkpoints = Checkpoints::new(repo.path());
        checkpoints
            .capture(&at("t1", Phase::Start))
            .await
            .expect("started");
        // pi never settled the first run; the next one starts.
        checkpoints
            .capture(&at("t2", Phase::Start))
            .await
            .expect("started");
        let plan = checkpoints
            .revert_plan(&at("t1", Phase::Start), None)
            .await
            .expect("plan");
        assert_eq!(plan.running, ["s1"]);
    });
}

#[test]
fn a_turn_waits_for_a_revert_past_the_time_limit_of_one_git() {
    run(async {
        let repo = Repo::new("revert-long");
        // Long enough for one capture's Git under a busy test run, short beside the wait.
        let checkpoints = std::sync::Arc::new(Checkpoints::with_timeout(
            repo.path(),
            Duration::from_secs(1),
        ));
        assert!(matches!(checkpoints.in_repository().await, Ok(true)));
        let reverting = crate::checkout::Checkout::new(
            repo.path().join(".git/watershed"),
            repo.path().join(".git/watershed"),
        )
        .alone()
        .await
        .expect("nothing runs");
        let starting = tokio::spawn({
            let checkpoints = checkpoints.clone();
            async move { checkpoints.capture(&at("t1", Phase::Start)).await }
        });
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(!starting.is_finished(), "the turn still waits");
        drop(reverting);
        tokio::time::timeout(Duration::from_secs(5), starting)
            .await
            .expect("the turn starts")
            .expect("joined")
            .expect("captured");
        let plan = checkpoints
            .revert_plan(&at("t1", Phase::Start), None)
            .await
            .expect("plan");
        assert_eq!(plan.running, ["s1"], "and it counts as running");
    });
}

/// Runs `git` in `dir` as the tests' user.
fn git_at(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_revert_never_writes_into_a_repository_a_turn_made() {
    run(async {
        let repo = Repo::new("revert-repositories");
        repo.write("foo", "a file\n");
        repo.git(&["add", "foo"]);
        repo.git(&["commit", "-q", "-m", "foo"]);
        std::fs::create_dir_all(repo.path().join("lib")).expect("folder");
        repo.write("lib/x.txt", "draft\n");
        let checkpoints = Checkpoints::new(repo.path());
        turn(&checkpoints, "t1", || {
            // The file `foo` becomes a repository with a commit, and the untracked folder `lib`
            // a repository without one.
            std::fs::remove_file(repo.path().join("foo")).expect("removed");
            std::fs::create_dir_all(repo.path().join("foo")).expect("folder");
            git_at(&repo.path().join("foo"), &["init", "-q"]);
            repo.write("foo/inner.txt", "inner\n");
            git_at(&repo.path().join("foo"), &["add", "."]);
            git_at(&repo.path().join("foo"), &["commit", "-q", "-m", "inner"]);
            git_at(&repo.path().join("lib"), &["init", "-q"]);
            repo.write("a.txt", "new\n");
        })
        .await;
        repo.write("lib/x.txt", "my work\n");

        let plan = checkpoints
            .revert_plan(&at("t1", Phase::Start), None)
            .await
            .expect("plan");
        assert_eq!(
            split(&plan),
            (vec!["a.txt"], vec!["foo", "lib/x.txt"]),
            "{plan:?}"
        );
        checkpoints
            .revert(&at("t1", Phase::Start))
            .await
            .expect("reverted");
        let read = |path: &str| std::fs::read_to_string(repo.path().join(path)).ok();
        assert_eq!(read("a.txt").as_deref(), Some("old\n"));
        assert_eq!(read("foo/inner.txt").as_deref(), Some("inner\n"));
        assert!(repo.path().join("foo/.git").exists(), "its history stays");
        assert_eq!(read("lib/x.txt").as_deref(), Some("my work\n"));
    });
}

#[test]
fn a_restored_file_git_now_ignores_counts_as_reverted() {
    run(async {
        let repo = Repo::new("revert-newly-ignored");
        repo.write("notes.txt", "notes\n");
        let checkpoints = Checkpoints::new(repo.path());
        turn(&checkpoints, "t1", || {
            std::fs::remove_file(repo.path().join("notes.txt")).expect("removed");
            std::fs::write(repo.path().join(".git/info/exclude"), "notes.txt\n").expect("exclude");
        })
        .await;
        checkpoints
            .revert(&at("t1", Phase::Start))
            .await
            .expect("reverted");
        assert_eq!(
            std::fs::read_to_string(repo.path().join("notes.txt")).expect("notes"),
            "notes\n"
        );
    });
}

#[test]
fn a_revert_lists_what_git_ignores_where_it_restores_a_file() {
    run(async {
        let repo = Repo::new("revert-ignored-in-place");
        repo.write("config.local", "draft\n");
        repo.write("data", "a file\n");
        repo.git(&["add", "data"]);
        repo.git(&["commit", "-q", "-m", "data"]);
        let checkpoints = Checkpoints::new(repo.path());
        turn(&checkpoints, "t1", || {
            // Now ignored and changed, so it is in no checkpoint since.
            repo.write(".gitignore", "config.local\n");
            repo.write("config.local", "secret\n");
            // A folder of files Git does not ignore, which the plan lists themselves.
            std::fs::remove_file(repo.path().join("data")).expect("removed");
            std::fs::create_dir_all(repo.path().join("data")).expect("folder");
            repo.write("data/x.txt", "tracked later\n");
        })
        .await;
        let plan = checkpoints
            .revert_plan(&at("t1", Phase::Start), None)
            .await
            .expect("plan");
        let listed: Vec<_> = plan
            .files
            .iter()
            .map(|file| (file.path.as_str(), file.status, file.ignored))
            .collect();
        assert_eq!(
            listed,
            [
                (".gitignore", ChangeStatus::Deleted, false),
                ("config.local", ChangeStatus::Added, true),
                ("data", ChangeStatus::Added, false),
                ("data/x.txt", ChangeStatus::Deleted, false),
            ]
        );
    });
}

#[test]
fn a_turn_whose_name_cannot_be_written_still_holds_reverts_off() {
    run(async {
        let repo = Repo::new("revert-unnamed");
        let mine = Checkpoints::new(repo.path());
        turn(&mine, "t1", || repo.write("a.txt", "new\n")).await;
        let running = repo.path().join(".git/watershed/running");
        std::fs::create_dir_all(&running).expect("folder");
        let made = Command::new("mkfifo")
            .arg(running.join("s2"))
            .status()
            .expect("mkfifo runs");
        assert!(made.success());
        let other = Checkpoints::new(repo.path());
        other
            .capture(&Checkpoint {
                session: "s2".to_owned(),
                turn: "u1".to_owned(),
                phase: Phase::Start,
            })
            .await
            .expect("started");
        assert!(matches!(
            mine.revert(&at("t1", Phase::Start)).await,
            Err(DiffError::AgentRunning(running)) if running.is_empty()
        ));
    });
}

#[test]
fn a_revert_that_git_stopped_midway_says_the_files_may_be_partly_reverted() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        let repo = Repo::new("revert-partly");
        std::fs::create_dir_all(repo.path().join("ro")).expect("folder");
        repo.write("ro/kept.txt", "kept\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "folder"]);
        let checkpoints = Checkpoints::new(repo.path());
        turn(&checkpoints, "t1", || {
            repo.write("ro/kept.txt", "changed\n")
        })
        .await;
        // Git cannot replace a file in a folder it may not write, and fails.
        let folder = repo.path().join("ro");
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).expect("mode");
        let reverted = checkpoints.revert(&at("t1", Phase::Start)).await;
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).expect("mode");
        assert!(
            matches!(reverted, Err(DiffError::PartlyReverted(_))),
            "{reverted:?}"
        );
    });
}

#[test]
fn a_returning_file_deletes_no_repository_in_its_place() {
    run(async {
        let repo = Repo::new("revert-repository-in-place");
        repo.write(".gitignore", "kept/\n");
        repo.git(&["add", ".gitignore"]);
        repo.git(&["commit", "-q", "-m", "ignore"]);
        for name in ["bare", "outer", "kept"] {
            repo.write(name, "a file\n");
        }
        let checkpoints = Checkpoints::new(repo.path());
        turn(&checkpoints, "t1", || {
            let folder = |name: &str| {
                std::fs::remove_file(repo.path().join(name)).expect("removed");
                std::fs::create_dir_all(repo.path().join(name)).expect("folder");
            };
            // A repository without a commit where the file was.
            folder("bare");
            git_at(&repo.path().join("bare"), &["init", "-q"]);
            repo.write("bare/work.txt", "work\n");
            // A folder holding a repository with a commit.
            folder("outer");
            std::fs::create_dir_all(repo.path().join("outer/sub")).expect("folder");
            git_at(&repo.path().join("outer/sub"), &["init", "-q"]);
            repo.write("outer/sub/inner.txt", "inner\n");
            git_at(&repo.path().join("outer/sub"), &["add", "."]);
            git_at(
                &repo.path().join("outer/sub"),
                &["commit", "-q", "-m", "inner"],
            );
            repo.write("outer/README.md", "beside it\n");
            // A repository with a commit that Git ignores.
            folder("kept");
            git_at(&repo.path().join("kept"), &["init", "-q"]);
            repo.write("kept/k.txt", "k\n");
            git_at(&repo.path().join("kept"), &["add", "."]);
            git_at(&repo.path().join("kept"), &["commit", "-q", "-m", "k"]);
            repo.write("a.txt", "new\n");
        })
        .await;
        let plan = checkpoints
            .revert_plan(&at("t1", Phase::Start), None)
            .await
            .expect("plan");
        assert_eq!(
            split(&plan),
            (
                vec!["a.txt"],
                vec!["bare", "kept", "outer", "outer/README.md", "outer/sub"]
            ),
            "{plan:?}"
        );
        checkpoints
            .revert(&at("t1", Phase::Start))
            .await
            .expect("reverted");
        for kept in [
            "bare/.git",
            "bare/work.txt",
            "outer/sub/.git",
            "outer/README.md",
            "kept/.git",
        ] {
            assert!(repo.path().join(kept).exists(), "{kept} was deleted");
        }
        assert_eq!(
            std::fs::read_to_string(repo.path().join("a.txt")).expect("a"),
            "old\n"
        );
    });
}

#[test]
fn a_repository_is_found_as_git_finds_it_and_links_are_not_followed() {
    use std::os::unix::fs::symlink;
    run(async {
        let repo = Repo::new("revert-find-repositories");
        repo.write("link", "a file\n");
        repo.write("caps", "a file\n");
        repo.git(&["add", "link", "caps"]);
        repo.git(&["commit", "-q", "-m", "files"]);
        let elsewhere = Repo::new("revert-find-repositories-elsewhere");
        // Git finds `.git` by asking the file system, which may not tell capitals apart.
        let blind = std::fs::symlink_metadata(repo.path().join("A.TXT")).is_ok();
        let checkpoints = Checkpoints::new(repo.path());
        turn(&checkpoints, "t1", || {
            std::fs::remove_file(repo.path().join("link")).expect("removed");
            symlink(elsewhere.path(), repo.path().join("link")).expect("link");
            // Where capitals count, Git refuses a `.GIT` path and nothing is captured.
            if blind {
                std::fs::remove_file(repo.path().join("caps")).expect("removed");
                std::fs::create_dir_all(repo.path().join("caps/sub")).expect("folder");
                git_at(&repo.path().join("caps/sub"), &["init", "-q"]);
                std::fs::rename(
                    repo.path().join("caps/sub/.git"),
                    repo.path().join("caps/sub/.GIT"),
                )
                .expect("renamed");
            }
        })
        .await;
        let plan = checkpoints
            .revert_plan(&at("t1", Phase::Start), None)
            .await
            .expect("plan");
        let (reverted, kept) = split(&plan);
        assert!(
            reverted.contains(&"link"),
            "a link is no repository: {plan:?}"
        );
        assert_eq!(kept.contains(&"caps"), blind, "{plan:?}");
        checkpoints
            .revert(&at("t1", Phase::Start))
            .await
            .expect("reverted");
        assert_eq!(
            std::fs::read_to_string(repo.path().join("link")).expect("file"),
            "a file\n"
        );
        assert_eq!(repo.path().join("caps/sub/.GIT").exists(), blind);
        drop(elsewhere);
    });
}

#[test]
fn a_revert_reads_paths_as_git_writes_them_whatever_their_order_or_links() {
    use std::os::unix::fs::symlink;
    run(async {
        let repo = Repo::new("revert-order-links");
        repo.write("p", "a file\n");
        std::fs::create_dir_all(repo.path().join("d")).expect("folder");
        repo.write("d/f.txt", "in a folder\n");
        repo.git(&["add", "p", "d"]);
        repo.git(&["commit", "-q", "-m", "files"]);
        // A project setting that lists `p/README.md` before `p`.
        repo.write(".git/order", "p/*\n*\n");
        repo.git(&["config", "diff.orderFile", ".git/order"]);
        let elsewhere = Repo::new("revert-order-links-elsewhere");
        let checkpoints = Checkpoints::new(repo.path());
        turn(&checkpoints, "t1", || {
            std::fs::remove_file(repo.path().join("p")).expect("removed");
            std::fs::create_dir_all(repo.path().join("p/sub")).expect("folder");
            git_at(&repo.path().join("p/sub"), &["init", "-q"]);
            repo.write("p/README.md", "beside it\n");
            // The folder `d` becomes a link to another repository.
            std::fs::remove_dir_all(repo.path().join("d")).expect("removed");
            symlink(elsewhere.path(), repo.path().join("d")).expect("link");
        })
        .await;
        let plan = checkpoints
            .revert_plan(&at("t1", Phase::Start), None)
            .await
            .expect("plan");
        let (reverted, kept) = split(&plan);
        assert_eq!(reverted, ["d", "d/f.txt"], "{plan:?}");
        assert_eq!(kept, ["p", "p/README.md"], "{plan:?}");
        checkpoints
            .revert(&at("t1", Phase::Start))
            .await
            .expect("reverted");
        assert_eq!(
            std::fs::read_to_string(repo.path().join("d/f.txt")).expect("file"),
            "in a folder\n"
        );
        assert!(repo.path().join("p/sub/.git").exists());
        drop(elsewhere);
    });
}

#[test]
fn a_folder_holding_a_submodule_stays_whole_when_a_turn_replaces_it() {
    run(async {
        let sub = Repo::new("revert-replaced-holder-inner");
        let repo = Repo::new("revert-replaced-holder");
        std::fs::create_dir_all(repo.path().join("d")).expect("folder");
        repo.write("d/x", "beside the submodule\n");
        repo.git(&[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            &sub.path().display().to_string(),
            "d/sub",
        ]);
        repo.git(&["add", "d/x"]);
        repo.git(&["commit", "-q", "-m", "holder"]);
        let checkpoints = Checkpoints::new(repo.path());
        turn(&checkpoints, "t1", || {
            std::fs::remove_dir_all(repo.path().join("d")).expect("removed");
            repo.write("d", "now a file\n");
        })
        .await;
        let plan = checkpoints
            .revert_plan(&at("t1", Phase::Start), None)
            .await
            .expect("plan");
        let (reverted, kept) = split(&plan);
        assert!(reverted.is_empty(), "{plan:?}");
        assert!(kept.contains(&"d/x"), "{plan:?}");
        checkpoints
            .revert(&at("t1", Phase::Start))
            .await
            .expect("nothing promised is skipped");
    });
}

#[test]
fn a_revert_leaves_bare_repositories_alone_as_git_finds_them() {
    run(async {
        let repo = Repo::new("revert-bare");
        repo.write(".gitignore", "clone\n");
        repo.write("clone", "a file\n");
        repo.git(&["add", ".gitignore"]);
        repo.git(&["add", "-f", "clone"]);
        repo.git(&["commit", "-q", "-m", "files"]);
        let source = Repo::new("revert-bare-source");
        let checkpoints = Checkpoints::new(repo.path());
        turn(&checkpoints, "t1", || {
            // An ignored bare clone takes the file's place: no `.git`, only Git's own folders.
            std::fs::remove_file(repo.path().join("clone")).expect("removed");
            git_at(
                repo.path(),
                &[
                    "clone",
                    "-q",
                    "--bare",
                    &source.path().display().to_string(),
                    "clone",
                ],
            );
            repo.write("a.txt", "new\n");
        })
        .await;
        let plan = checkpoints
            .revert_plan(&at("t1", Phase::Start), None)
            .await
            .expect("plan");
        let (reverted, kept) = split(&plan);
        assert_eq!(reverted, ["a.txt"], "{plan:?}");
        assert_eq!(kept, ["clone"], "{plan:?}");
        checkpoints
            .revert(&at("t1", Phase::Start))
            .await
            .expect("reverted");
        assert!(repo.path().join("clone/HEAD").exists(), "its history stays");
        drop(source);
    });
}

#[test]
fn a_turns_end_releases_only_its_own_turn() {
    run(async {
        let repo = Repo::new("revert-overlap");
        let checkpoints = std::sync::Arc::new(Checkpoints::new(repo.path()));
        turn(&checkpoints, "t0", || repo.write("a.txt", "new\n")).await;
        checkpoints
            .capture(&at("t1", Phase::Start))
            .await
            .expect("started");
        // Turn t1's end waits for the capture in progress while turn t2 starts.
        let held = checkpoints.hold().await;
        let ending = tokio::spawn({
            let checkpoints = checkpoints.clone();
            async move { checkpoints.capture(&at("t1", Phase::End)).await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        let starting = tokio::spawn({
            let checkpoints = checkpoints.clone();
            async move { checkpoints.capture(&at("t2", Phase::Start)).await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(held);
        ending.await.expect("joined").expect("ended");
        starting.await.expect("joined").expect("started");
        let other = Checkpoints::new(repo.path());
        assert!(
            matches!(
                other.revert(&at("t0", Phase::Start)).await,
                Err(DiffError::AgentRunning(_))
            ),
            "turn t2 still runs"
        );
    });
}

#[test]
fn checkpoints_are_listed_and_removed_by_session_or_all() {
    run(async {
        let repo = Repo::new("remove");
        let mine = Checkpoints::new(repo.path());
        turn(&mine, "t1", || repo.write("a.txt", "one\n")).await;
        turn(&mine, "t2", || repo.write("a.txt", "two\n")).await;
        let other = Checkpoints::new(repo.path());
        let theirs = |turn: &str, phase| Checkpoint {
            session: "s2".to_owned(),
            turn: turn.to_owned(),
            phase,
        };
        other
            .capture(&theirs("u1", Phase::Start))
            .await
            .expect("started");
        other
            .capture(&theirs("u1", Phase::End))
            .await
            .expect("ended");
        assert_eq!(
            mine.sessions().await.expect("listed"),
            [("s1".to_owned(), 2), ("s2".to_owned(), 1)]
        );

        // While an agent runs, no session goes, so none of its turns can start meanwhile.
        let file = repo.path().join(".git/s1.jsonl");
        std::fs::write(&file, "{}\n").expect("session file");
        other
            .capture(&theirs("u2", Phase::Start))
            .await
            .expect("started");
        assert!(matches!(
            mine.delete_session("s1", &file).await,
            Err(DiffError::AgentRunning(running)) if running == ["s2"]
        ));
        assert!(file.exists(), "kept");
        assert!(matches!(
            mine.remove_all().await,
            Err(DiffError::AgentRunning(_))
        ));
        other
            .capture(&theirs("u2", Phase::End))
            .await
            .expect("ended");
        mine.delete_session("s1", &file).await.expect("deleted");
        assert!(!file.exists());
        assert_eq!(
            mine.sessions().await.expect("listed"),
            [("s2".to_owned(), 2)]
        );
        assert!(matches!(
            changes_of(&mine, "t1").await,
            Err(DiffError::NoCheckpoint)
        ));
        mine.remove_all().await.expect("removed");
        assert!(mine.sessions().await.expect("listed").is_empty());
        assert_eq!(repo.git(&["for-each-ref", "refs/watershed"]), "");
        assert!(matches!(
            mine.delete_session("../x", &file).await,
            Err(DiffError::InvalidCheckpoint(_))
        ));
    });
}

#[test]
fn an_agent_running_in_another_worktree_keeps_the_shared_checkpoints() {
    run(async {
        let repo = Repo::new("remove-worktrees");
        let linked = repo.path().with_file_name(format!(
            "watershed-checkpoint-remove-worktrees-linked-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&linked);
        repo.git(&["worktree", "add", "-q", &linked.display().to_string()]);
        let here = Checkpoints::new(repo.path());
        turn(&here, "t1", || repo.write("a.txt", "new\n")).await;
        let there = Checkpoints::new(&linked);
        let theirs = |phase| Checkpoint {
            session: "s2".to_owned(),
            turn: "u1".to_owned(),
            phase,
        };
        there.capture(&theirs(Phase::Start)).await.expect("started");

        assert!(matches!(
            here.remove_all().await,
            Err(DiffError::AgentRunning(running)) if running == ["s2"]
        ));
        let file = repo.path().join(".git/s2.jsonl");
        std::fs::write(&file, "{}\n").expect("session file");
        assert!(matches!(
            here.delete_session("s2", &file).await,
            Err(DiffError::AgentRunning(_))
        ));
        assert!(file.exists());
        there.capture(&theirs(Phase::End)).await.expect("ended");
        here.remove_all().await.expect("removed once nothing runs");
        assert!(here.sessions().await.expect("listed").is_empty());
        there.close().await;
        let _ = std::fs::remove_dir_all(&linked);
    });
}

#[test]
fn worktrees_that_cannot_be_listed_keep_the_checkpoints() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        let repo = Repo::new("remove-unlisted");
        let checkpoints = Checkpoints::new(repo.path());
        turn(&checkpoints, "t1", || repo.write("a.txt", "new\n")).await;
        let worktrees = repo.path().join(".git/worktrees");
        std::fs::create_dir_all(&worktrees).expect("folder");
        std::fs::set_permissions(&worktrees, std::fs::Permissions::from_mode(0o000)).expect("mode");
        let removed = checkpoints.remove_all().await;
        std::fs::set_permissions(&worktrees, std::fs::Permissions::from_mode(0o755)).expect("mode");
        assert!(matches!(removed, Err(DiffError::Io(_))), "{removed:?}");
        assert_eq!(checkpoints.sessions().await.expect("listed").len(), 1);
    });
}

#[test]
fn a_turn_in_a_worktree_the_cleanup_cannot_see_still_holds_it_off() {
    run(async {
        let repo = Repo::new("remove-unseen");
        let linked = repo.path().with_file_name(format!(
            "watershed-checkpoint-remove-unseen-linked-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&linked);
        repo.git(&["worktree", "add", "-q", &linked.display().to_string()]);
        let here = Checkpoints::new(repo.path());
        turn(&here, "t1", || repo.write("a.txt", "new\n")).await;
        let there = Checkpoints::new(&linked);
        there
            .capture(&Checkpoint {
                session: "s2".to_owned(),
                turn: "u1".to_owned(),
                phase: Phase::Start,
            })
            .await
            .expect("started");
        // The worktree's entry is gone from the list, as one made after the cleanup listed them.
        let entries = repo.path().join(".git/worktrees");
        let hidden = repo.path().join(".git/worktrees-hidden");
        std::fs::rename(&entries, &hidden).expect("hidden");
        let removed = here.remove_all().await;
        std::fs::rename(&hidden, &entries).expect("back");
        assert!(
            matches!(removed, Err(DiffError::AgentRunning(_))),
            "{removed:?}"
        );
        there.close().await;
        let _ = std::fs::remove_dir_all(&linked);
    });
}

#[test]
fn a_turns_end_releases_only_the_same_sessions_turn() {
    run(async {
        let repo = Repo::new("revert-same-turn-id");
        let checkpoints = Checkpoints::new(repo.path());
        turn(&checkpoints, "t0", || repo.write("a.txt", "new\n")).await;
        let of = |session: &str, phase| Checkpoint {
            session: session.to_owned(),
            turn: "t1".to_owned(),
            phase,
        };
        checkpoints
            .capture(&of("s1", Phase::Start))
            .await
            .expect("started");
        // The agent switched sessions; the new session's turn has the same id.
        checkpoints
            .capture(&of("s2", Phase::Start))
            .await
            .expect("started");
        checkpoints
            .capture(&of("s1", Phase::End))
            .await
            .expect("ended");
        let other = Checkpoints::new(repo.path());
        assert!(
            matches!(
                other.revert(&at("t0", Phase::Start)).await,
                Err(DiffError::AgentRunning(_))
            ),
            "session s2's turn still runs"
        );
    });
}
