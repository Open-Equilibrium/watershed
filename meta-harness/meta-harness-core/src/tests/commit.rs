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
            .commit(
                &["a.txt".into(), "new[*].txt".into()],
                "Subject\n\nDetails",
                None,
            )
            .await
            .unwrap();
        assert_eq!(result.files, 2);
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
fn commit_creates_and_switches_a_branch_and_commits_deletions() {
    run(async {
        let repo = Repo::new("commit-new-branch");
        identity(&repo);
        repo.git(&["branch", "-M", "main"]);
        let old = repo.git(&["rev-parse", "main"]);
        std::fs::remove_file(repo.path().join("a.txt")).unwrap();
        let result = Checkpoints::new(repo.path())
            .commit(&["a.txt".into()], "Remove file", Some("feature/remove"))
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
            assert!(checkpoints.commit(paths, message, None).await.is_err());
        }
        checkpoints.capture(&at("t1", Phase::Start)).await.unwrap();
        assert!(matches!(
            checkpoints.commit(&files, "Message", None).await,
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
        assert!(checkpoints.commit(&files, "Message", None).await.is_err());
        repo.git(&["merge", "--abort"]);
        repo.write("a.txt", "changed\n");
        repo.git(&["checkout", "--detach", "-q"]);
        assert!(matches!(
            checkpoints.commit(&files, "Message", None).await,
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
        let result = checkpoints.commit(&["a.txt".into()], "Message", None).await;
        assert!(
            matches!(result, Err(DiffError::CommitFailed(ref output)) if output.contains("hook-output") && output.contains("hook-error")),
            "{result:?}"
        );
        assert_eq!(repo.git(&["rev-parse", "HEAD"]), head);
        std::fs::write(&hook, "#!/bin/sh\nprintf 'formatted\n' > a.txt\n").unwrap();
        let result = checkpoints
            .commit(&["a.txt".into()], "Message", None)
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
            .commit(&["a.txt".into()], "Formatted", None)
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
        let files = checkpoints
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
            .commit(&["a.txt".into()], "First commit", None)
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
            .commit(&["only-turn.txt".into()], "Commit turn three", None)
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
