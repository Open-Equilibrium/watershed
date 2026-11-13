use super::checkpoint::{Repo, at, run};
use crate::{ChangeStatus, Checkpoints, DiffError, Phase, Whitespace};

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
        assert_eq!(result.subject.as_deref(), Some("Subject"));
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
fn skip_marks_hide_only_files_absent_from_the_work() {
    run(async {
        // A file on disk that the project marks as skipped still shows its edit.
        let marked = Repo::new("skip-on-disk");
        marked.git(&["update-index", "--skip-worktree", "a.txt"]);
        marked.write("a.txt", "marked edit\n");
        let (_, changes) = Checkpoints::new(marked.path())
            .uncommitted_changes(Whitespace::Shown)
            .await
            .unwrap();
        assert_eq!(
            changes
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            ["a.txt"]
        );
        // dir/b.txt is outside the sparse checkout: absent from disk and not an edit.
        let repo = Repo::new("sparse-checkout");
        identity(&repo);
        std::fs::create_dir(repo.path().join("dir")).unwrap();
        repo.write("dir/b.txt", "b\n");
        repo.git(&["add", "dir/b.txt"]);
        repo.git(&["commit", "-m", "Add b"]);
        repo.git(&["sparse-checkout", "set", "--no-cone", "/a.txt"]);
        repo.write("a.txt", "edited\n");
        let checkpoints = Checkpoints::new(repo.path());
        let (_, changes) = checkpoints
            .uncommitted_changes(Whitespace::Shown)
            .await
            .unwrap();
        assert_eq!(
            changes
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            ["a.txt"]
        );
        assert_eq!(checkpoints.status().await.unwrap().changed, 1);
        let plan = checkpoints.commit_plan(&[]).await.unwrap();
        let paths: Vec<String> = plan
            .files
            .iter()
            .map(|file| file.change.path.clone())
            .collect();
        assert_eq!(paths, ["a.txt"]);
        checkpoints
            .commit(&plan.snapshot, &paths, "Commit a", None)
            .await
            .unwrap();
        assert!(
            repo.git(&["ls-tree", "-r", "--name-only", "HEAD"])
                .lines()
                .any(|path| path == "dir/b.txt")
        );
    });
}

#[test]
fn skip_mark_without_sparse_checkout_keeps_a_deletion_visible() {
    run(async {
        // Without sparse checkout no file is left out of the work, so a deleted file is a deletion.
        let repo = Repo::new("skip-deleted-without-sparse");
        identity(&repo);
        repo.write("b.txt", "b\n");
        repo.git(&["add", "b.txt"]);
        repo.git(&["commit", "-m", "Add b"]);
        repo.git(&["update-index", "--skip-worktree", "b.txt"]);
        std::fs::remove_file(repo.path().join("b.txt")).unwrap();
        let checkpoints = Checkpoints::new(repo.path());
        let (_, changes) = checkpoints
            .uncommitted_changes(Whitespace::Shown)
            .await
            .unwrap();
        assert_eq!(
            changes
                .iter()
                .map(|file| (file.path.as_str(), file.status))
                .collect::<Vec<_>>(),
            [("b.txt", ChangeStatus::Deleted)]
        );
        let plan = checkpoints.commit_plan(&[]).await.unwrap();
        assert_eq!(
            plan.files
                .iter()
                .map(|file| (file.change.path.as_str(), file.change.status))
                .collect::<Vec<_>>(),
            [("b.txt", ChangeStatus::Deleted)]
        );
        checkpoints
            .commit(&plan.snapshot, &["b.txt".into()], "Delete b", None)
            .await
            .unwrap();
        assert!(
            !repo
                .git(&["ls-tree", "-r", "--name-only", "HEAD"])
                .lines()
                .any(|path| path == "b.txt")
        );
    });
}

#[test]
fn commit_plan_detects_partial_staging_without_changing_index_marks() {
    run(async {
        for marks in [
            &[][..],
            &["--assume-unchanged"][..],
            &["--skip-worktree"][..],
            &["--assume-unchanged", "--skip-worktree"][..],
        ] {
            let repo = Repo::new("commit-plan-marked-staging");
            repo.write("a.txt", "staged edit\n");
            repo.write("excluded.txt", "staged only\n");
            repo.git(&["add", "a.txt", "excluded.txt"]);
            for mark in marks {
                repo.git(&["update-index", mark, "a.txt"]);
            }
            repo.write("a.txt", "whole file with later edit\n");
            let index = std::fs::read(repo.path().join(".git/index")).unwrap();
            let flags = repo.git(&["ls-files", "-v", "a.txt"]);
            let plan = Checkpoints::new(repo.path())
                .commit_plan(&[])
                .await
                .unwrap();
            assert!(
                plan.files
                    .iter()
                    .find(|file| file.change.path == "a.txt")
                    .unwrap()
                    .staged_partly,
                "{marks:?}"
            );
            assert!(
                !plan
                    .files
                    .iter()
                    .find(|file| file.change.path == "excluded.txt")
                    .unwrap()
                    .staged_partly
            );
            assert_eq!(
                std::fs::read(repo.path().join(".git/index")).unwrap(),
                index
            );
            assert_eq!(repo.git(&["ls-files", "-v", "a.txt"]), flags);
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
fn commit_staged_deletions_and_renames_preserve_excluded_staging() {
    run(async {
        for rename in [false, true] {
            let repo = Repo::new("commit-staged-deletion");
            identity(&repo);
            repo.write("excluded.txt", "staged excluded\n");
            repo.git(&["add", "excluded.txt"]);
            repo.write("excluded.txt", "unstaged excluded\n");
            let mut selected = vec!["a.txt".into()];
            if rename {
                repo.git(&["mv", "a.txt", "renamed.txt"]);
                selected.push("renamed.txt".into());
            } else {
                repo.git(&["rm", "a.txt"]);
            }
            Checkpoints::new(repo.path())
                .commit_current(&selected, "Remove or rename file", None)
                .await
                .unwrap();
            assert_eq!(
                repo.git(&["ls-tree", "--name-only", "HEAD"]),
                if rename { "renamed.txt\n" } else { "" }
            );
            if rename {
                assert_eq!(repo.git(&["show", "HEAD:renamed.txt"]), "old\n");
            }
            assert_eq!(repo.git(&["show", ":excluded.txt"]), "staged excluded\n");
            assert_eq!(
                repo.git(&["diff", "--cached", "--name-only"]),
                "excluded.txt\n"
            );
            assert_eq!(
                std::fs::read_to_string(repo.path().join("excluded.txt")).unwrap(),
                "unstaged excluded\n"
            );
        }
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
        let merge = super::fixture_git()
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
        // Only Git's record of a merge counts, not a ref that happens to share its name.
        repo.git(&["tag", "MERGE_HEAD"]);
        assert!(!checkpoints.status().await.unwrap().merging);
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
                stop_reason: None,
                stop_error: None,
            });
        }
        repo.git(&["add", "a.txt"]);
        repo.write("a.txt", "turn three and you\n");
        repo.write("outside.txt", "outside\n");
        let plan = checkpoints.commit_plan(&turns).await.unwrap();
        assert!(plan.files.iter().all(|file| file.attribution_available));
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
fn status_resolves_origin_defaults_without_overriding_local_upstreams() {
    run(async {
        let repo = Repo::new("local-upstream");
        repo.git(&["branch", "-M", "main"]);
        repo.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        repo.git(&[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ]);
        let checkpoints = Checkpoints::new(repo.path());
        assert_eq!(
            checkpoints
                .status()
                .await
                .unwrap()
                .default_branch
                .as_deref(),
            Some("main")
        );
        repo.git(&["checkout", "--detach"]);
        assert_eq!(
            checkpoints
                .status()
                .await
                .unwrap()
                .default_branch
                .as_deref(),
            Some("main")
        );
        repo.git(&["switch", "-c", "feat", "--track", "main"]);
        let status = checkpoints.status().await.unwrap();
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
fn a_missing_first_start_leaves_its_stretch_unattributed() {
    use crate::TurnCheckpoints;
    run(async {
        let repo = Repo::new("attribution-missing-first");
        let checkpoints = Checkpoints::new(repo.path());
        repo.write("a.txt", "an edit by turn one, whose start is missing\n");
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
                stop_reason: None,
                stop_error: None,
            })
            .collect();
        let plan = checkpoints.commit_plan(&turns).await.unwrap();
        // The edit may be turn one's, which has no checkpoint to tell it apart from the user's.
        assert!(!plan.files[0].attribution_available);
        assert!(plan.files[0].turns.is_empty());
        assert!(!plan.files[0].edited);
    });
}

#[test]
fn a_turn_without_its_start_leaves_only_its_stretch_unattributed() {
    use crate::TurnCheckpoints;
    run(async {
        let repo = Repo::new("attribution-missing-gap");
        let checkpoints = Checkpoints::new(repo.path());
        checkpoints.capture(&at("t1", Phase::Start)).await.unwrap();
        repo.write("a.txt", "turn one\n");
        checkpoints.capture(&at("t1", Phase::End)).await.unwrap();
        // Turn two's checkpoints are missing, so its edit lies in no checkpointed stretch.
        repo.write("b.txt", "turn two\n");
        checkpoints.capture(&at("t3", Phase::Start)).await.unwrap();
        repo.write("c.txt", "turn three\n");
        checkpoints.capture(&at("t3", Phase::End)).await.unwrap();
        let turns: Vec<_> = (1..=3)
            .map(|number| TurnCheckpoints {
                session: "s1".into(),
                turn: if number == 2 {
                    "t2".into()
                } else {
                    format!("t{number}")
                },
                number,
                start_entry: "start".into(),
                end_entry: Some("end".into()),
                prompt_entry: None,
                prompt: String::new(),
                started_ms: None,
                stop_reason: None,
                stop_error: None,
                ended_ms: None,
                ended: true,
                failure: None,
                changed: None,
            })
            .collect();
        let plan = checkpoints.commit_plan(&turns).await.unwrap();
        let attributed: Vec<_> = plan
            .files
            .iter()
            .map(|file| {
                (
                    file.change.path.as_str(),
                    file.attribution_available,
                    file.turns.clone(),
                )
            })
            .collect();
        assert_eq!(
            attributed,
            [
                ("a.txt", true, vec![1]),
                ("b.txt", false, vec![]),
                ("c.txt", true, vec![3]),
            ]
        );
    });
}

#[test]
fn a_turn_without_its_end_checkpoint_keeps_its_changes() {
    use crate::TurnCheckpoints;
    run(async {
        let repo = Repo::new("attribution-missing-end");
        let checkpoints = Checkpoints::new(repo.path());
        checkpoints.capture(&at("t1", Phase::Start)).await.unwrap();
        repo.write("a.txt", "turn one\n");
        checkpoints.capture(&at("t2", Phase::Start)).await.unwrap();
        repo.write("b.txt", "turn two\n");
        let turns: Vec<_> = (1..=2)
            .map(|number| TurnCheckpoints {
                session: "s1".into(),
                turn: format!("t{number}"),
                number,
                start_entry: "start".into(),
                end_entry: None,
                prompt_entry: None,
                prompt: String::new(),
                started_ms: None,
                ended_ms: None,
                ended: false,
                failure: None,
                changed: None,
                stop_reason: None,
                stop_error: None,
            })
            .collect();
        let plan = checkpoints.commit_plan(&turns).await.unwrap();
        let attributed: Vec<_> = plan
            .files
            .iter()
            .map(|file| (file.change.path.as_str(), file.turns.clone(), file.edited))
            .collect();
        assert_eq!(
            attributed,
            [("a.txt", vec![1], false), ("b.txt", vec![2], false)]
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
fn commit_refuses_a_mode_change_git_would_skip() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        let repo = Repo::new("commit-mode-skipped");
        identity(&repo);
        repo.write("script.sh", "run\n");
        repo.git(&["add", "script.sh"]);
        repo.git(&["commit", "-qm", "script"]);
        // Git ignores an executable bit when this is false, so a commit would leave it out.
        repo.git(&["config", "core.fileMode", "false"]);
        std::fs::set_permissions(
            repo.path().join("script.sh"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        repo.write("a.txt", "edited\n");
        let checkpoints = Checkpoints::new(repo.path());
        let plan = checkpoints.commit_plan(&[]).await.unwrap();
        let head = repo.git(&["rev-parse", "HEAD"]);

        let result = checkpoints
            .commit(
                &plan.snapshot,
                &["a.txt".into(), "script.sh".into()],
                "Message",
                None,
            )
            .await;

        assert!(matches!(result, Err(DiffError::Git(_))), "{result:?}");
        assert_eq!(repo.git(&["rev-parse", "HEAD"]), head);
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
fn attribution_bounds_keep_files_without_partial_labels() {
    use crate::TurnCheckpoints;
    run(async {
        let repo = Repo::new("attribution-timeout");
        let mut checkpoints = Checkpoints::new(repo.path());
        checkpoints.capture(&at("t1", Phase::Start)).await.unwrap();
        repo.write("a.txt", "turn change\n");
        checkpoints.capture(&at("t1", Phase::End)).await.unwrap();
        repo.write("outside.txt", "your change\n");
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
            stop_reason: None,
            stop_error: None,
        };
        let complete = checkpoints
            .commit_plan(std::slice::from_ref(&turn))
            .await
            .unwrap();
        assert!(complete.files[0].attribution_available);
        assert_eq!(complete.files[0].turns, [1]);
        assert!(checkpoints.commit_plan(&[]).await.unwrap().files[0].attribution_available);
        // Reusing captured tree pairs reaches the batch's 4 MiB bound without
        // thousands of separate checkpoint writes.
        for (deadline, turns) in [
            (std::time::Duration::ZERO, vec![turn.clone()]),
            (std::time::Duration::from_secs(60), vec![turn; 26_000]),
        ] {
            checkpoints.set_attribution_timeout(deadline);
            let plan = checkpoints.commit_plan(&turns).await.unwrap();
            assert_eq!(plan.files.len(), 2);
            for (file, original) in plan.files.iter().zip(&complete.files) {
                assert_eq!(file.change, original.change);
                assert!(!file.attribution_available);
                assert!(file.turns.is_empty());
                assert!(!file.edited);
            }
        }
        identity(&repo);
        checkpoints
            .commit(
                &complete.snapshot,
                &["a.txt".into()],
                "Commit without attribution",
                None,
            )
            .await
            .unwrap();
        assert_eq!(repo.git(&["show", "HEAD:a.txt"]), "turn change\n");
    });
}

#[test]
fn malformed_checkpoint_ids_list_files_without_attribution() {
    use crate::TurnCheckpoints;
    run(async {
        let repo = Repo::new("malformed-checkpoint-id");
        repo.write("a.txt", "changed\n");
        // A session id outside letters, digits, `-` and `_` names no checkpoint ref.
        let turn = TurnCheckpoints {
            session: "a.b".into(),
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
            stop_reason: None,
            stop_error: None,
        };
        let plan = Checkpoints::new(repo.path())
            .commit_plan(&[turn])
            .await
            .unwrap();
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].change.path, "a.txt");
        assert!(!plan.files[0].attribution_available);
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
        for (reverse, staged) in [(false, false), (false, true), (true, false), (true, true)] {
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
            if staged {
                repo.git(&["add", "--all", "--", "foo"]);
            }
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

#[test]
fn a_successful_commit_survives_failed_post_commit_inspection() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        for target in ["index", "config"] {
            let repo = Repo::new("commit-inspection-failure");
            identity(&repo);
            repo.write("a.txt", "committed content\n");
            let hook = repo.path().join(".git/hooks/post-commit");
            std::fs::write(
                &hook,
                format!("#!/bin/sh\nprintf 'bad {target}' > .git/{target}\n"),
            )
            .unwrap();
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
            let result = Checkpoints::new(repo.path())
                .commit_current(&["a.txt".into()], "Committed", None)
                .await;
            assert!(result.is_ok(), "{result:?}");
            let result = result.unwrap();
            assert_eq!(
                result.subject.as_deref(),
                if target == "index" {
                    Some("Committed")
                } else {
                    None
                }
            );
            assert!(result.inspection_error.is_some());
            if target == "config" {
                // Repair only this fixture's deliberately malformed configuration to inspect HEAD.
                std::fs::write(
                    repo.path().join(".git/config"),
                    "[core]\nrepositoryformatversion = 0\nbare = false\n",
                )
                .unwrap();
            }
            assert_eq!(repo.git(&["show", "HEAD:a.txt"]), "committed content\n");
        }
    });
}
