use super::checkpoint::{Repo, at, run};
use crate::{Checkpoints, DiffError, Phase, Whitespace};

fn identity(repo: &Repo) {
    repo.git(&["config", "user.name", "Test"]);
    repo.git(&["config", "user.email", "test@example.com"]);
    repo.git(&["config", "commit.gpgsign", "false"]);
}

#[test]
fn commit_whole_selected_files_preserves_excluded_staging_and_checkpoints() {
    run(async {
        let repo = Repo::new("commit-selection");
        identity(&repo);
        let checkpoints = Checkpoints::new(repo.path());
        checkpoints.capture(&at("t1", Phase::Start)).await.unwrap();
        repo.write("a.txt", "staged\n");
        repo.git(&["add", "a.txt"]);
        repo.write("a.txt", "whole file\n");
        repo.write("excluded.txt", "staged excluded\n");
        repo.git(&["add", "excluded.txt"]);
        repo.write("excluded.txt", "unstaged excluded\n");
        repo.write("new[*].txt", "new\n");
        checkpoints.capture(&at("t1", Phase::End)).await.unwrap();
        let refs = repo.git(&["show-ref"]);
        let result = checkpoints
            .commit_current(
                &["a.txt".into(), "new[*].txt".into()],
                "Subject\n\nDetails",
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            repo.git(&["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"])
                .lines()
                .count(),
            2
        );
        assert_eq!(result.subject, "Subject");
        assert_eq!(repo.git(&["show", "HEAD:a.txt"]), "whole file\n");
        assert_eq!(repo.git(&["show", "HEAD:new[*].txt"]), "new\n");
        assert_eq!(repo.git(&["show", ":excluded.txt"]), "staged excluded\n");
        assert_eq!(
            std::fs::read_to_string(repo.path().join("excluded.txt")).unwrap(),
            "unstaged excluded\n"
        );
        for reference in refs.lines().filter(|line| line.contains("refs/watershed/")) {
            assert!(repo.git(&["show-ref"]).contains(reference));
        }
    });
}

#[test]
fn commit_includes_selected_files_hidden_by_index_marks() {
    run(async {
        for mark in ["--skip-worktree", "--assume-unchanged"] {
            let repo = Repo::new("commit-marked-files");
            identity(&repo);
            repo.write("ordinary.txt", "old\n");
            repo.write("excluded.txt", "old excluded\n");
            repo.git(&["add", "."]);
            repo.git(&["commit", "-m", "Initial files"]);
            repo.write("excluded.txt", "staged excluded\n");
            repo.git(&["add", "excluded.txt"]);
            repo.git(&["update-index", mark, "a.txt", "excluded.txt"]);
            repo.write("a.txt", "selected hidden edit\n");
            repo.write("ordinary.txt", "ordinary edit\n");
            repo.write("excluded.txt", "unstaged excluded\n");
            let excluded_mark = repo.git(&["ls-files", "-v", "excluded.txt"]);
            Checkpoints::new(repo.path())
                .commit_current(
                    &["a.txt".into(), "ordinary.txt".into()],
                    "Selected files",
                    None,
                )
                .await
                .unwrap();
            assert_eq!(
                repo.git(&["show", "HEAD:a.txt"]),
                "selected hidden edit\n",
                "{mark}"
            );
            assert_eq!(repo.git(&["show", "HEAD:ordinary.txt"]), "ordinary edit\n");
            assert_eq!(repo.git(&["show", "HEAD:excluded.txt"]), "old excluded\n");
            assert_eq!(repo.git(&["show", ":excluded.txt"]), "staged excluded\n");
            assert_eq!(repo.git(&["ls-files", "-v", "excluded.txt"]), excluded_mark);
            assert_eq!(
                std::fs::read_to_string(repo.path().join("excluded.txt")).unwrap(),
                "unstaged excluded\n"
            );
        }
    });
}

#[test]
fn commit_creates_and_switches_a_branch_and_commits_deletions() {
    run(async {
        let repo = Repo::new("commit-new-branch");
        identity(&repo);
        repo.git(&["branch", "-M", "main"]);
        let old = repo.git(&["rev-parse", "main"]);
        std::fs::remove_file(repo.path().join("a.txt")).unwrap();
        let result = Checkpoints::new(repo.path())
            .commit_current(&["a.txt".into()], "Remove file", Some("feature/remove"))
            .await
            .unwrap();
        assert_eq!(result.branch, "feature/remove");
        assert_eq!(repo.git(&["rev-parse", "main"]), old);
        assert_eq!(repo.git(&["ls-tree", "--name-only", "HEAD"]), "");
    });
}

#[test]
fn commits_require_message_files_branch_and_no_merge_or_running_agent() {
    run(async {
        let repo = Repo::new("commit-refusals");
        identity(&repo);
        repo.git(&["branch", "-M", "main"]);
        let checkpoints = Checkpoints::new(repo.path());
        repo.write("a.txt", "changed\n");
        let files = ["a.txt".into()];
        for (paths, message) in [(&files[..], "  "), (&[][..], "Message")] {
            assert!(
                checkpoints
                    .commit_current(paths, message, None)
                    .await
                    .is_err()
            );
        }
        checkpoints.capture(&at("t1", Phase::Start)).await.unwrap();
        assert!(matches!(
            checkpoints.commit_current(&files, "Message", None).await,
            Err(DiffError::AgentRunning(_))
        ));
        checkpoints.capture(&at("t1", Phase::End)).await.unwrap();
        repo.git(&["switch", "-c", "other"]);
        repo.git(&["commit", "-am", "Other edit"]);
        repo.git(&["switch", "main"]);
        repo.write("a.txt", "main edit\n");
        repo.git(&["commit", "-am", "Main edit"]);
        let merge = std::process::Command::new("git")
            .args(["merge", "other"])
            .current_dir(repo.path())
            .output()
            .unwrap();
        assert!(!merge.status.success());
        let status = checkpoints.status().await.unwrap();
        assert!(status.merging);
        assert_eq!(status.conflicts, ["a.txt"]);
        assert!(
            checkpoints
                .commit_current(&files, "Message", None)
                .await
                .is_err()
        );
        repo.git(&["merge", "--abort"]);
        repo.write("a.txt", "changed\n");
        repo.git(&["checkout", "--detach", "-q"]);
        assert!(matches!(
            checkpoints.commit_current(&files, "Message", None).await,
            Err(DiffError::NoBranch)
        ));
    });
}

#[test]
fn commit_hooks_report_failure_output_and_changes_left_uncommitted() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        let repo = Repo::new("commit-hooks");
        identity(&repo);
        let checkpoints = Checkpoints::new(repo.path());
        let hook = repo.path().join(".git/hooks/pre-commit");
        std::fs::write(
            &hook,
            "#!/bin/sh\necho hook-output\necho hook-error >&2\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        repo.write("a.txt", "changed\n");
        let head = repo.git(&["rev-parse", "HEAD"]);
        let result = checkpoints
            .commit_current(&["a.txt".into()], "Message", None)
            .await;
        assert!(
            matches!(result, Err(DiffError::CommitFailed(ref output)) if output.contains("hook-output") && output.contains("hook-error")),
            "{result:?}"
        );
        assert_eq!(repo.git(&["rev-parse", "HEAD"]), head);
        std::fs::write(&hook, "#!/bin/sh\nprintf 'formatted\n' > a.txt\n").unwrap();
        let result = checkpoints
            .commit_current(&["a.txt".into()], "Message", None)
            .await
            .unwrap();
        assert_eq!(result.hook_changed, ["a.txt"]);
        assert_eq!(repo.git(&["show", "HEAD:a.txt"]), "changed\n");
        assert_eq!(
            std::fs::read_to_string(repo.path().join("a.txt")).unwrap(),
            "formatted\n"
        );
        repo.write("a.txt", "next change\n");
        std::fs::write(
            &hook,
            "#!/bin/sh\nprintf 'staged format\n' > a.txt\ngit add a.txt\n",
        )
        .unwrap();
        let result = checkpoints
            .commit_current(&["a.txt".into()], "Formatted", None)
            .await
            .unwrap();
        assert!(
            result.hook_changed.is_empty(),
            "a change committed by the hook is not left uncommitted"
        );
        assert_eq!(repo.git(&["show", "HEAD:a.txt"]), "staged format\n");
    });
}

#[test]
fn uncommitted_scope_includes_untracked_honors_whitespace_and_keeps_index() {
    run(async {
        let repo = Repo::new("uncommitted");
        let checkpoints = Checkpoints::new(repo.path());
        repo.write("a.txt", " old  \n");
        repo.write("new.txt", "new\n");
        repo.write(".git/info/exclude", "ignored.txt\n");
        repo.write("ignored.txt", "ignored\n");
        let index = std::fs::read(repo.path().join(".git/index")).unwrap();
        let (_, files) = checkpoints
            .uncommitted_changes(Whitespace::Shown)
            .await
            .unwrap();
        assert_eq!(
            files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            ["a.txt", "new.txt"]
        );
        assert_eq!(
            checkpoints
                .uncommitted_changes(Whitespace::Hidden)
                .await
                .unwrap()
                .1
                .len(),
            1
        );
        assert!(
            checkpoints
                .uncommitted_patch("new.txt", Whitespace::Shown)
                .await
                .unwrap()
                .contains("+new")
        );
        assert_eq!(
            std::fs::read(repo.path().join(".git/index")).unwrap(),
            index
        );
    });
}

#[test]
fn uncommitted_identity_tracks_content_and_the_committed_baseline() {
    run(async {
        let repo = Repo::new("uncommitted-identity");
        identity(&repo);
        let checkpoints = Checkpoints::new(repo.path());
        repo.write("a.txt", "first edit\n");
        let first = checkpoints
            .uncommitted_changes(Whitespace::Shown)
            .await
            .unwrap();
        assert_eq!(
            checkpoints
                .uncommitted_changes(Whitespace::Shown)
                .await
                .unwrap(),
            first
        );
        repo.write("a.txt", "other edit\n");
        let other = checkpoints
            .uncommitted_changes(Whitespace::Shown)
            .await
            .unwrap();
        assert_eq!(
            other.1, first.1,
            "the same files and counts do not identify their contents"
        );
        assert_ne!(other.0, first.0);
        repo.git(&["add", "a.txt"]);
        repo.git(&["commit", "-m", "Updated baseline"]);
        let committed = checkpoints
            .uncommitted_changes(Whitespace::Shown)
            .await
            .unwrap();
        assert_ne!(
            committed.0, other.0,
            "the unchanged checkout now has a different baseline"
        );
        assert!(committed.1.is_empty());
    });
}

#[test]
fn initialize_git_reopens_checkpoint_store_and_starts_with_next_turn() {
    run(async {
        let repo = Repo::new("initialize");
        std::fs::remove_dir_all(repo.path().join(".git")).unwrap();
        let checkpoints = Checkpoints::new(repo.path());
        let other = Checkpoints::new(repo.path());
        assert!(!checkpoints.in_repository().await.unwrap());
        assert!(!other.in_repository().await.unwrap());
        checkpoints.initialize_git().await.unwrap();
        assert!(checkpoints.in_repository().await.unwrap());
        assert!(other.in_repository().await.unwrap());
        assert!(checkpoints.sessions().await.unwrap().is_empty());
        assert_eq!(
            checkpoints
                .uncommitted_changes(Whitespace::Shown)
                .await
                .unwrap()
                .1
                .len(),
            1
        );
        checkpoints
            .capture(&at("next", Phase::Start))
            .await
            .unwrap();
        checkpoints.capture(&at("next", Phase::End)).await.unwrap();
        assert_eq!(checkpoints.sessions().await.unwrap(), [("s1".into(), 1)]);
        identity(&repo);
        repo.write("excluded.txt", "excluded\n");
        repo.git(&["add", "excluded.txt"]);
        checkpoints
            .commit_current(&["a.txt".into()], "First commit", None)
            .await
            .unwrap();
        assert_eq!(repo.git(&["ls-tree", "--name-only", "HEAD"]), "a.txt\n");
        assert_eq!(repo.git(&["show", ":excluded.txt"]), "excluded\n");
    });
}

#[test]
fn commit_plan_attributes_turns_outside_edits_and_partial_staging() {
    use crate::TurnCheckpoints;
    run(async {
        let repo = Repo::new("commit-attribution");
        let checkpoints = Checkpoints::new(repo.path());
        repo.write("before.txt", "your earlier edit\n");
        let mut turns = Vec::new();
        for number in 1..=3 {
            let id = format!("t{number}");
            if number == 3 {
                repo.write("a.txt", "your edit between turns\n");
            }
            checkpoints.capture(&at(&id, Phase::Start)).await.unwrap();
            if number == 1 {
                repo.write("before.txt", "turn on earlier edit\n");
            }
            if number != 2 {
                repo.write("a.txt", &format!("turn {number}\n"));
            }
            if number == 3 {
                repo.write("only-turn.txt", "turn three\n");
            }
            checkpoints.capture(&at(&id, Phase::End)).await.unwrap();
            turns.push(TurnCheckpoints {
                session: "s1".into(),
                turn: id,
                number,
                start_entry: "start".into(),
                end_entry: Some("end".into()),
                prompt_entry: None,
                prompt: String::new(),
                started_ms: None,
                ended_ms: None,
                ended: true,
                failure: None,
                changed: None,
            });
        }
        repo.git(&["add", "a.txt"]);
        repo.write("a.txt", "turn three and you\n");
        repo.write("outside.txt", "outside\n");
        let plan = checkpoints.commit_plan(&turns).await.unwrap();
        let a = plan
            .files
            .iter()
            .find(|f| f.change.path == "a.txt")
            .unwrap();
        let before = plan
            .files
            .iter()
            .find(|f| f.change.path == "before.txt")
            .unwrap();
        assert_eq!(before.turns, [1]);
        assert!(
            before.edited,
            "an edit before the first turn is outside that turn"
        );
        assert_eq!(a.turns, [1, 3]);
        assert!(a.edited);
        assert!(a.staged_partly);
        let only = plan
            .files
            .iter()
            .find(|f| f.change.path == "only-turn.txt")
            .unwrap();
        assert_eq!(only.turns, [3]);
        assert!(!only.edited);
        assert!(
            plan.files
                .iter()
                .find(|f| f.change.path == "outside.txt")
                .unwrap()
                .turns
                .is_empty()
        );
        identity(&repo);
        checkpoints
            .commit_current(&["only-turn.txt".into()], "Commit turn three", None)
            .await
            .unwrap();
        checkpoints.capture(&at("t4", Phase::Start)).await.unwrap();
        repo.write("only-turn.txt", "turn four\n");
        checkpoints.capture(&at("t4", Phase::End)).await.unwrap();
        turns.push(TurnCheckpoints {
            turn: "t4".into(),
            number: 4,
            ..turns.last().unwrap().clone()
        });
        let next = checkpoints.commit_plan(&turns).await.unwrap();
        let only = next
            .files
            .iter()
            .find(|f| f.change.path == "only-turn.txt")
            .unwrap();
        assert_eq!(only.turns, [3, 4]);
        assert!(
            !only.edited,
            "committing a turn is not an outside file edit"
        );
        repo.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        repo.git(&[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ]);
        repo.git(&["config", "remote.origin.url", "."]);
        repo.git(&[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ]);
        let branch = repo.git(&["branch", "--show-current"]);
        repo.git(&[
            "config",
            &format!("branch.{}.remote", branch.trim()),
            "origin",
        ]);
        repo.git(&[
            "config",
            &format!("branch.{}.merge", branch.trim()),
            "refs/heads/main",
        ]);
        assert_eq!(
            checkpoints
                .status()
                .await
                .unwrap()
                .default_branch
                .as_deref(),
            Some("main")
        );
    });
}

#[test]
fn status_supports_a_branch_tracking_a_local_branch() {
    run(async {
        let repo = Repo::new("local-upstream");
        repo.git(&["branch", "-M", "main"]);
        repo.git(&["switch", "-c", "feat", "--track", "main"]);
        let status = Checkpoints::new(repo.path()).status().await.unwrap();
        assert_eq!(status.upstream.as_deref(), Some("main"));
        assert_eq!(status.default_branch, None);
    });
}

#[test]
fn initialize_refuses_a_turn_in_another_non_git_session() {
    run(async {
        let repo = Repo::new("initialize-running");
        std::fs::remove_dir_all(repo.path().join(".git")).unwrap();
        let first = Checkpoints::new(repo.path());
        let second = Checkpoints::new(repo.path());
        assert!(matches!(
            first.capture(&at("t1", Phase::Start)).await,
            Err(DiffError::NotARepository)
        ));
        assert!(matches!(second.initialize_git().await,
            Err(DiffError::AgentRunning(ref sessions)) if sessions == &["s1"]));
        assert!(!repo.path().join(".git").exists());
        assert!(matches!(
            first.capture(&at("t1", Phase::End)).await,
            Err(DiffError::NotARepository)
        ));
        second.initialize_git().await.unwrap();
        first.capture(&at("t2", Phase::Start)).await.unwrap();
        assert!(matches!(second.create_branch("blocked").await,
            Err(DiffError::AgentRunning(ref sessions)) if sessions == &["s1"]));
        first.capture(&at("t2", Phase::End)).await.unwrap();
        second.create_branch("allowed").await.unwrap();
    });
}

#[test]
fn commit_refuses_partial_directory_file_replacements_before_index_changes() {
    run(async {
        for reverse in [false, true] {
            let repo = Repo::new("commit-overlap");
            identity(&repo);
            if reverse {
                repo.write("foo", "file\n");
            } else {
                std::fs::create_dir(repo.path().join("foo")).unwrap();
                repo.write("foo/bar", "nested\n");
            }
            repo.git(&["add", "."]);
            repo.git(&["commit", "-m", "Before replacement"]);
            if reverse {
                std::fs::remove_file(repo.path().join("foo")).unwrap();
                std::fs::create_dir(repo.path().join("foo")).unwrap();
                repo.write("foo/bar", "nested\n");
            } else {
                std::fs::remove_dir_all(repo.path().join("foo")).unwrap();
                repo.write("foo", "file\n");
            }
            let checkpoints = Checkpoints::new(repo.path());
            let index = std::fs::read(repo.path().join(".git/index")).unwrap();
            let head = repo.git(&["rev-parse", "HEAD"]);
            for selected in ["foo", "foo/bar"] {
                let result = checkpoints
                    .commit_current(&[selected.into()], "Replacement", None)
                    .await;
                assert!(result.is_err(), "partial replacement was committed");
                assert_eq!(repo.git(&["rev-parse", "HEAD"]), head);
                assert_eq!(
                    std::fs::read(repo.path().join(".git/index")).unwrap(),
                    index
                );
            }
        }
    });
}

#[test]
fn commit_includes_an_untracked_nested_repository() {
    run(async {
        let repo = Repo::new("commit-nested");
        identity(&repo);
        let nested = Repo::at(repo.path().join("nested"));
        let checkpoints = Checkpoints::new(repo.path());
        checkpoints
            .commit_current(&["nested".into()], "Add nested", None)
            .await
            .unwrap();
        assert!(
            repo.git(&["ls-tree", "HEAD", "nested"])
                .starts_with("160000 commit")
        );
        drop(nested);
    });
}

#[test]
fn commit_plan_uses_the_first_available_start_as_its_baseline() {
    use crate::TurnCheckpoints;
    run(async {
        let repo = Repo::new("attribution-missing-first");
        let checkpoints = Checkpoints::new(repo.path());
        repo.write("a.txt", "your edit\n");
        checkpoints.capture(&at("t2", Phase::Start)).await.unwrap();
        repo.write("a.txt", "turn edit\n");
        checkpoints.capture(&at("t2", Phase::End)).await.unwrap();
        let turns: Vec<_> = (1..=2)
            .map(|number| TurnCheckpoints {
                session: "s1".into(),
                turn: format!("t{number}"),
                number,
                start_entry: "start".into(),
                end_entry: Some("end".into()),
                prompt_entry: None,
                prompt: String::new(),
                started_ms: None,
                ended_ms: None,
                ended: true,
                failure: None,
                changed: None,
            })
            .collect();
        let plan = checkpoints.commit_plan(&turns).await.unwrap();
        assert_eq!(plan.files[0].turns, [2]);
        assert!(
            plan.files[0].edited,
            "the edit before the first available start is yours"
        );
    });
}

#[test]
fn commit_refuses_selected_content_changed_since_the_plan() {
    run(async {
        let repo = Repo::new("commit-stale");
        identity(&repo);
        let checkpoints = Checkpoints::new(repo.path());
        repo.write("a.txt", "shown content\n");
        let plan = checkpoints.commit_plan(&[]).await.unwrap();
        repo.write("a.txt", "same count but unseen\n");
        let index = std::fs::read(repo.path().join(".git/index")).unwrap();
        let head = repo.git(&["rev-parse", "HEAD"]);
        let result = checkpoints
            .commit(&plan.snapshot, &["a.txt".into()], "Message", None)
            .await;
        assert!(
            matches!(result, Err(DiffError::Git(ref text)) if text ==
            "The selected files changed. Open Commit changes again."),
            "{result:?}"
        );
        assert_eq!(repo.git(&["rev-parse", "HEAD"]), head);
        assert_eq!(
            std::fs::read(repo.path().join(".git/index")).unwrap(),
            index
        );
    });
}

#[test]
fn new_branch_commit_failure_reports_the_branch_after_either_hook() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        for hook_name in ["post-checkout", "pre-commit"] {
            let repo = Repo::new("commit-branch-hook");
            identity(&repo);
            let checkpoints = Checkpoints::new(repo.path());
            repo.write("a.txt", "changed\n");
            let hook = repo.path().join(".git/hooks").join(hook_name);
            std::fs::write(&hook, "#!/bin/sh\necho hook-refused >&2\nexit 1\n").unwrap();
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
            let head = repo.git(&["rev-parse", "HEAD"]);
            let error = checkpoints
                .commit_current(&["a.txt".into()], "Message", Some("feat"))
                .await
                .unwrap_err();
            let DiffError::CommitOnBranchFailed { branch, error } = error else {
                panic!("missing created branch outcome: {error:?}");
            };
            assert_eq!(branch, "feat");
            assert!(matches!(
                error.as_ref(),
                DiffError::HookFailed(_) | DiffError::CommitFailed(_)
            ));
            assert!(error.to_string().contains("hook-refused"), "{error}");
            assert_eq!(repo.git(&["branch", "--show-current"]), "feat\n");
            assert_eq!(repo.git(&["rev-parse", "HEAD"]), head);
        }
    });
}

impl Checkpoints {
    async fn commit_current(
        &self,
        paths: &[String],
        message: &str,
        branch: Option<&str>,
    ) -> Result<crate::diff::Commit, DiffError> {
        let plan = self.commit_plan(&[]).await?;
        self.commit(&plan.snapshot, paths, message, branch).await
    }
}

#[test]
fn attribution_timeout_keeps_files_without_partial_labels() {
    use crate::TurnCheckpoints;
    run(async {
        let repo = Repo::new("attribution-timeout");
        let mut checkpoints = Checkpoints::new(repo.path());
        checkpoints.capture(&at("t1", Phase::Start)).await.unwrap();
        repo.write("a.txt", "turn change\n");
        checkpoints.capture(&at("t1", Phase::End)).await.unwrap();
        let turn = TurnCheckpoints {
            session: "s1".into(),
            turn: "t1".into(),
            number: 1,
            start_entry: "start".into(),
            end_entry: Some("end".into()),
            prompt_entry: None,
            prompt: String::new(),
            started_ms: None,
            ended_ms: None,
            ended: true,
            failure: None,
            changed: None,
        };
        assert_eq!(
            checkpoints
                .commit_plan(std::slice::from_ref(&turn))
                .await
                .unwrap()
                .files[0]
                .turns,
            [1]
        );
        checkpoints.set_attribution_timeout(std::time::Duration::ZERO);
        let plan = checkpoints.commit_plan(&[turn]).await.unwrap();
        assert_eq!(plan.files[0].change.path, "a.txt");
        assert!(plan.files[0].turns.is_empty());
        assert!(!plan.files[0].edited);
    });
}

#[test]
fn a_turn_waits_until_initialization_releases_its_folder_lock() {
    run(async {
        let repo = Repo::new("initialize-waits");
        std::fs::remove_dir_all(repo.path().join(".git")).unwrap();
        let initializing = crate::checkout::Checkout::folder(repo.path())
            .unwrap()
            .alone()
            .await
            .unwrap();
        // Initialize has written the Git directory but has not released its lock yet.
        crate::diff::initialize(repo.path()).await.unwrap();
        let checkpoints = std::sync::Arc::new(Checkpoints::new(repo.path()));
        let mut waiting = checkpoints.waiting();
        let starting = tokio::spawn({
            let checkpoints = checkpoints.clone();
            async move { checkpoints.capture(&at("t1", Phase::Start)).await }
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), waiting.changed())
            .await
            .expect("turn waits for Initialize")
            .unwrap();
        assert!(*waiting.borrow());
        assert!(!starting.is_finished());
        drop(initializing);
        starting.await.unwrap().unwrap();
        assert!(!*waiting.borrow());
        let other = Checkpoints::new(repo.path());
        assert!(matches!(other.create_branch("blocked").await,
            Err(DiffError::AgentRunning(ref sessions)) if sessions == &["s1"]));
        checkpoints.capture(&at("t1", Phase::End)).await.unwrap();
    });
}

#[test]
fn commit_accepts_excluded_edits_after_the_plan() {
    run(async {
        let repo = Repo::new("commit-excluded-snapshot");
        identity(&repo);
        repo.write("a.txt", "selected\n");
        repo.write("excluded.txt", "before\n");
        let checkpoints = Checkpoints::new(repo.path());
        let plan = checkpoints.commit_plan(&[]).await.unwrap();
        repo.write("excluded.txt", "after\n");
        checkpoints
            .commit(&plan.snapshot, &["a.txt".into()], "Subject", None)
            .await
            .unwrap();
        assert_eq!(repo.git(&["show", "HEAD:a.txt"]), "selected\n");
        assert_eq!(
            std::fs::read_to_string(repo.path().join("excluded.txt")).unwrap(),
            "after\n"
        );
        assert!(
            !repo
                .git(&["ls-tree", "--name-only", "HEAD"])
                .contains("excluded.txt")
        );
    });
}

#[test]
fn commit_preserves_staged_only_paths_conflicting_with_the_selection() {
    run(async {
        for reverse in [false, true] {
            let repo = Repo::new("commit-staged-overlap");
            identity(&repo);
            let (staged, selected) = if reverse {
                ("foo/bar", "foo")
            } else {
                ("foo", "foo/bar")
            };
            if reverse {
                std::fs::create_dir(repo.path().join("foo")).unwrap();
            }
            repo.write(staged, "excluded staging\n");
            repo.git(&["add", staged]);
            if reverse {
                std::fs::remove_dir_all(repo.path().join("foo")).unwrap();
            } else {
                std::fs::remove_file(repo.path().join("foo")).unwrap();
                std::fs::create_dir(repo.path().join("foo")).unwrap();
            }
            repo.write(selected, "selected\n");
            let index = std::fs::read(repo.path().join(".git/index")).unwrap();
            let head = repo.git(&["rev-parse", "HEAD"]);
            let result = Checkpoints::new(repo.path())
                .commit_current(&[selected.into()], "Selection", None)
                .await;
            assert!(result.is_err(), "{result:?}");
            assert_eq!(
                std::fs::read(repo.path().join(".git/index")).unwrap(),
                index
            );
            assert_eq!(repo.git(&["rev-parse", "HEAD"]), head);
            assert_eq!(
                repo.git(&["show", &format!(":{staged}")]),
                "excluded staging\n"
            );
        }
    });
}

#[test]
fn commit_accepts_complete_file_directory_replacements() {
    run(async {
        for reverse in [false, true] {
            let repo = Repo::new("commit-complete-overlap");
            identity(&repo);
            let (old, new) = if reverse {
                ("foo/bar", "foo")
            } else {
                ("foo", "foo/bar")
            };
            if reverse {
                std::fs::create_dir(repo.path().join("foo")).unwrap();
            }
            repo.write(old, "old\n");
            repo.git(&["add", old]);
            repo.git(&["commit", "-m", "Before replacement"]);
            repo.write("excluded", "excluded staging\n");
            repo.git(&["add", "excluded"]);
            if reverse {
                std::fs::remove_dir_all(repo.path().join("foo")).unwrap();
            } else {
                std::fs::remove_file(repo.path().join("foo")).unwrap();
                std::fs::create_dir(repo.path().join("foo")).unwrap();
            }
            repo.write(new, "replacement\n");
            Checkpoints::new(repo.path())
                .commit_current(&[old.into(), new.into()], "Replacement", None)
                .await
                .unwrap();
            assert_eq!(repo.git(&["show", &format!("HEAD:{new}")]), "replacement\n");
            assert_eq!(repo.git(&["show", ":excluded"]), "excluded staging\n");
            assert_eq!(repo.git(&["diff", "--name-only"]), "");
            assert_eq!(repo.git(&["diff", "--cached", "--name-only"]), "excluded\n");
        }
    });
}
