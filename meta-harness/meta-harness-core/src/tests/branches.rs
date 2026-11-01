use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use super::checkpoint::{Repo, at, run};
use crate::{BranchTarget, Changes, Checkpoints, DiffError, GitStatus, Phase};

/// A clone of a repository with `a.txt` on `main`, which it tracks as `origin/main`.
struct Clone {
    origin: Repo,
    repo: Repo,
}

impl Clone {
    fn new(name: &str) -> Self {
        let origin = Repo::new(&format!("{name}-origin"));
        origin.git(&["branch", "-M", "main"]);
        let dir = PathBuf::from(format!("{}-clone", origin.path().display()));
        let _ = std::fs::remove_dir_all(&dir);
        let cloned = Command::new("git")
            .args(["clone", "-q"])
            .arg(origin.path())
            .arg(&dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .expect("git runs");
        assert!(cloned.success(), "cloned");
        Self {
            origin,
            repo: Repo(dir),
        }
    }

    /// Commits `text` as `a.txt` on the origin's `main`.
    fn commit_upstream(&self, text: &str) {
        self.origin.write("a.txt", text);
        self.origin.git(&["commit", "-q", "-am", text]);
    }
}

/// The id of the stash of the branch checked out; one naming nothing when it has none.
async fn stash_of(checkpoints: &Checkpoints) -> String {
    match checkpoints.stash().await.expect("stash read") {
        Some(stash) => stash.id,
        None => "0000000000000000000000000000000000000000".to_owned(),
    }
}

/// Makes the hook `name` of `repo` exit with `status`.
fn hook(repo: &Repo, name: &str, status: u8) {
    use std::os::unix::fs::PermissionsExt;
    let path = repo.path().join(".git/hooks").join(name);
    std::fs::write(
        &path,
        format!("#!/bin/sh\necho 'hook failed' >&2\nexit {status}\n"),
    )
    .expect("hook");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("executable");
}

fn local(name: &str) -> BranchTarget {
    BranchTarget::Local(name.to_owned())
}

fn read(repo: &Repo, path: &str) -> String {
    std::fs::read_to_string(repo.path().join(path)).expect("file read")
}

#[test]
fn the_status_counts_commits_against_the_upstream_and_uncommitted_files() {
    run(async {
        let clone = Clone::new("status");
        let repo = &clone.repo;
        repo.write("a.txt", "mine\n");
        repo.git(&["commit", "-q", "-am", "mine"]);
        clone.commit_upstream("theirs one\n");
        clone.commit_upstream("theirs two\n");
        repo.git(&["fetch", "-q"]);
        repo.write("a.txt", "edited\n");
        repo.write("new.txt", "new\n");
        std::fs::create_dir(repo.path().join("dir")).expect("dir");
        repo.write("dir/b.txt", "b\n");
        let index = std::fs::read(repo.path().join(".git/index")).expect("index");
        let checkpoints = Checkpoints::new(repo.path());

        let status = checkpoints.status().await.expect("status");

        assert_eq!(
            status,
            GitStatus {
                branch: Some("main".to_owned()),
                commit: status.commit.clone(),
                upstream: Some("origin/main".to_owned()),
                ahead: 1,
                behind: 2,
                changed: 3,
            }
        );
        assert_eq!(
            std::fs::read(repo.path().join(".git/index")).expect("index"),
            index,
            "reading the status leaves the project's index alone"
        );
    });
}

#[test]
fn a_detached_head_or_a_branch_without_upstream_has_no_counts() {
    run(async {
        let repo = Repo::new("status-detached");
        let checkpoints = Checkpoints::new(repo.path());
        let head = repo.git(&["rev-parse", "HEAD"]).trim().to_owned();

        let status = checkpoints.status().await.expect("status");
        assert!(status.branch.is_some());
        assert_eq!(status.upstream, None);
        assert_eq!((status.ahead, status.behind, status.changed), (0, 0, 0));

        repo.git(&["switch", "-q", "--detach"]);
        let status = checkpoints.status().await.expect("status");
        assert_eq!(status.branch, None);
        assert_eq!(status.commit, Some(head));

        let outside = std::env::temp_dir().join(format!("watershed-no-git-{}", std::process::id()));
        std::fs::create_dir_all(&outside).expect("folder");
        assert!(matches!(
            Checkpoints::new(&outside).status().await,
            Err(DiffError::NotARepository)
        ));
        let _ = std::fs::remove_dir_all(outside);
    });
}

#[test]
fn branches_list_local_ones_then_remote_ones_no_local_branch_tracks() {
    run(async {
        let clone = Clone::new("branches");
        clone.origin.git(&["branch", "release"]);
        let repo = &clone.repo;
        repo.git(&["fetch", "-q"]);
        repo.git(&["branch", "feature"]);
        let checkpoints = Checkpoints::new(repo.path());

        let branches = checkpoints.branches().await.expect("branches");

        let names: Vec<(&str, bool, bool)> = branches
            .branches
            .iter()
            .map(|branch| (branch.name.as_str(), branch.remote, branch.current))
            .collect();
        assert_eq!(
            names,
            [
                ("feature", false, false),
                ("main", false, true),
                ("origin/release", true, false),
            ],
            "origin/main is main's upstream and origin/HEAD no branch"
        );
        assert!(
            branches
                .branches
                .iter()
                .all(|branch| branch.committed_ms > 0)
        );
        assert!(branches.running.is_empty());
    });
}

#[test]
fn switching_carries_uncommitted_changes_and_a_remote_branch_gets_a_local_one_tracking_it() {
    run(async {
        let clone = Clone::new("switch");
        clone.origin.git(&["branch", "release"]);
        let repo = &clone.repo;
        repo.git(&["fetch", "-q"]);
        repo.git(&["branch", "feature"]);
        repo.write("a.txt", "edited\n");
        let checkpoints = Checkpoints::new(repo.path());

        checkpoints
            .switch_branch(&local("feature"), Changes::Bring)
            .await
            .expect("switched");
        assert_eq!(repo.git(&["branch", "--show-current"]), "feature\n");
        assert_eq!(read(repo, "a.txt"), "edited\n");

        checkpoints
            .switch_branch(
                &BranchTarget::Remote("origin/release".to_owned()),
                Changes::Bring,
            )
            .await
            .expect("switched");
        assert_eq!(repo.git(&["branch", "--show-current"]), "release\n");
        assert_eq!(
            repo.git(&["rev-parse", "--abbrev-ref", "release@{upstream}"]),
            "origin/release\n"
        );
    });
}

#[test]
fn git_refusing_a_switch_over_uncommitted_changes_names_the_files_and_changes_nothing() {
    run(async {
        let repo = Repo::new("switch-refused");
        repo.git(&["branch", "other"]);
        repo.git(&["switch", "-q", "other"]);
        repo.write("a.txt", "other\n");
        repo.git(&["commit", "-q", "-am", "other"]);
        repo.git(&["switch", "-q", "-"]);
        repo.write("a.txt", "edited\n");
        let current = repo.git(&["branch", "--show-current"]);
        let checkpoints = Checkpoints::new(repo.path());

        assert!(matches!(
            checkpoints.switch_branch(&local("other"), Changes::Bring).await,
            Err(DiffError::WouldOverwrite(paths)) if paths == ["a.txt"]
        ));
        assert_eq!(repo.git(&["branch", "--show-current"]), current);
        assert_eq!(read(&repo, "a.txt"), "edited\n");
    });
}

#[test]
fn no_branch_is_switched_or_created_while_an_agent_runs_in_the_checkout() {
    run(async {
        let repo = Repo::new("switch-running");
        repo.git(&["branch", "other"]);
        let current = repo.git(&["branch", "--show-current"]);
        let agent = Checkpoints::new(repo.path());
        agent
            .capture(&at("t1", Phase::Start))
            .await
            .expect("started");
        let checkpoints = Checkpoints::new(repo.path());

        let branches = checkpoints.branches().await.expect("branches");
        assert_eq!(branches.running, ["s1"]);
        assert!(matches!(
            checkpoints.switch_branch(&local("other"), Changes::Bring).await,
            Err(DiffError::AgentRunning(running)) if running == ["s1"]
        ));
        assert!(matches!(
            checkpoints.create_branch("new").await,
            Err(DiffError::AgentRunning(running)) if running == ["s1"]
        ));
        repo.write("a.txt", "edited\n");
        assert!(matches!(
            checkpoints.switch_branch(&local("other"), Changes::Leave).await,
            Err(DiffError::AgentRunning(running)) if running == ["s1"]
        ));
        assert!(matches!(
            checkpoints.restore_stash(&stash_of(&checkpoints).await).await,
            Err(DiffError::AgentRunning(running)) if running == ["s1"]
        ));
        assert!(matches!(
            checkpoints.discard_stash(&stash_of(&checkpoints).await).await,
            Err(DiffError::AgentRunning(running)) if running == ["s1"]
        ));
        assert_eq!(repo.git(&["stash", "list"]), "");
        assert_eq!(repo.git(&["branch", "--show-current"]), current);
        assert_eq!(repo.git(&["branch", "--list", "new"]), "");

        agent.capture(&at("t1", Phase::End)).await.expect("ended");
        checkpoints
            .switch_branch(&local("other"), Changes::Bring)
            .await
            .expect("switched once the turn ended");
        assert_eq!(read(&repo, "a.txt"), "edited\n");
    });
}

#[test]
fn a_new_branch_starts_at_the_current_commit_unless_git_refuses_its_name() {
    run(async {
        let repo = Repo::new("create");
        repo.write("a.txt", "edited\n");
        let head = repo.git(&["rev-parse", "HEAD"]);
        let checkpoints = Checkpoints::new(repo.path());

        for refused in ["fix..login", "-x", "a b", ""] {
            assert!(
                matches!(
                    checkpoints.create_branch(refused).await,
                    Err(DiffError::InvalidBranch(_))
                ),
                "{refused:?} is refused"
            );
        }
        assert_eq!(repo.git(&["branch", "--list"]).lines().count(), 1);

        checkpoints
            .create_branch("feature/retry-after")
            .await
            .expect("created");
        assert_eq!(
            repo.git(&["branch", "--show-current"]),
            "feature/retry-after\n"
        );
        assert_eq!(repo.git(&["rev-parse", "HEAD"]), head);
        assert_eq!(read(&repo, "a.txt"), "edited\n");
    });
}

#[test]
fn fetching_updates_only_the_upstreams_remote_tracking_branches() {
    run(async {
        let clone = Clone::new("fetch");
        let repo = &clone.repo;
        let checkpoints = Checkpoints::new(repo.path());
        clone.commit_upstream("theirs\n");
        clone.origin.git(&["tag", "v1"]);
        let _ = std::fs::remove_file(repo.path().join(".git/FETCH_HEAD"));

        assert!(checkpoints.fetch().await.expect("fetched"));

        assert_eq!(checkpoints.status().await.expect("status").behind, 1);
        assert_eq!(read(repo, "a.txt"), "old\n", "the files stay");
        assert_eq!(repo.git(&["tag", "--list"]), "", "no tags arrive");
        assert!(!repo.path().join(".git/FETCH_HEAD").exists());

        repo.git(&["switch", "-q", "-c", "local"]);
        assert!(
            !checkpoints.fetch().await.expect("nothing to fetch"),
            "a branch without upstream fetches nothing"
        );
    });
}

#[test]
fn a_background_fetch_leaves_tags_alone_despite_pruning_settings() {
    run(async {
        for scope in ["fetch", "remote.origin"] {
            let clone = Clone::new(&format!("fetch-prune-{scope}"));
            let repo = &clone.repo;
            repo.git(&["tag", "local-only"]);
            let tags = repo.git(&["show-ref", "--tags"]);
            clone.commit_upstream("theirs\n");
            clone.origin.git(&["tag", "remote-only"]);
            repo.git(&["config", &format!("{scope}.prune"), "true"]);
            repo.git(&["config", &format!("{scope}.pruneTags"), "true"]);
            let checkpoints = Checkpoints::new(repo.path());

            assert!(checkpoints.fetch().await.expect("fetched"));

            assert_eq!(repo.git(&["show-ref", "--tags"]), tags, "{scope}");
            assert_eq!(checkpoints.status().await.expect("status").behind, 1);
        }
    });
}

#[test]
fn a_failed_fetch_keeps_credentials_out_of_its_error() {
    run(async {
        let repo = Repo::new("fetch-secret");
        repo.git(&[
            "remote",
            "add",
            "origin",
            "https://someone:hunter2@127.0.0.1:9/repository.git",
        ]);
        repo.git(&["config", "branch.master.remote", "origin"]);
        repo.git(&["config", "branch.master.merge", "refs/heads/master"]);
        repo.git(&["config", "branch.main.remote", "origin"]);
        repo.git(&["config", "branch.main.merge", "refs/heads/main"]);
        let checkpoints = Checkpoints::new(repo.path());

        let error = checkpoints
            .fetch()
            .await
            .expect_err("nothing listens there");

        assert!(
            !error.to_string().contains("hunter2"),
            "the password stays out: {error}"
        );
    });
}

#[test]
fn leaving_changes_stashes_them_on_the_branch_and_restoring_brings_them_back() {
    run(async {
        let repo = Repo::new("stash");
        let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
        repo.git(&["branch", "other"]);
        repo.write("a.txt", "edited\n");
        repo.write("new.txt", "new\n");
        let checkpoints = Checkpoints::new(repo.path());

        checkpoints
            .switch_branch(&local("other"), Changes::Leave)
            .await
            .expect("switched");

        assert_eq!(repo.git(&["branch", "--show-current"]), "other\n");
        assert_eq!(read(&repo, "a.txt"), "old\n");
        assert!(
            !repo.path().join("new.txt").exists(),
            "untracked files go too"
        );
        assert_eq!(
            checkpoints.stash().await.expect("read"),
            None,
            "other has none"
        );
        let branches = checkpoints.branches().await.expect("branches");
        let stashed: Vec<&str> = branches
            .branches
            .iter()
            .filter(|branch| branch.stashed)
            .map(|branch| branch.name.as_str())
            .collect();
        assert_eq!(stashed, [main.as_str()]);
        assert_eq!(
            repo.git(&["stash", "list", "--format=%s"]),
            format!("On {main}: Liquid: changes left on {main}\n")
        );

        checkpoints
            .switch_branch(&local(&main), Changes::Bring)
            .await
            .expect("back");
        let stash = checkpoints
            .stash()
            .await
            .expect("read")
            .expect("main's stash");
        let mut files: Vec<&str> = stash.files.iter().map(|file| file.path.as_str()).collect();
        files.sort();
        assert_eq!(files, ["a.txt", "new.txt"]);
        assert!(stash.created_ms > 0);

        checkpoints
            .restore_stash(&stash_of(&checkpoints).await)
            .await
            .expect("restored");
        assert_eq!(read(&repo, "a.txt"), "edited\n");
        assert_eq!(read(&repo, "new.txt"), "new\n");
        assert_eq!(checkpoints.stash().await.expect("read"), None);
        assert_eq!(repo.git(&["stash", "list"]), "");
    });
}

#[test]
fn a_branch_keeps_one_stash_and_other_stashes_stay_untouched() {
    run(async {
        let repo = Repo::new("stash-one");
        let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
        repo.git(&["branch", "other"]);
        repo.write("a.txt", "theirs\n");
        repo.git(&["stash", "push", "-q", "-m", "the user's own"]);
        let checkpoints = Checkpoints::new(repo.path());

        for text in ["first\n", "second\n"] {
            repo.write("a.txt", text);
            checkpoints
                .switch_branch(&local("other"), Changes::Leave)
                .await
                .expect("left");
            checkpoints
                .switch_branch(&local(&main), Changes::Bring)
                .await
                .expect("back");
        }

        let subjects = repo.git(&["stash", "list", "--format=%s"]);
        assert_eq!(
            subjects,
            format!("On {main}: Liquid: changes left on {main}\nOn {main}: the user's own\n")
        );
        checkpoints
            .restore_stash(&stash_of(&checkpoints).await)
            .await
            .expect("restored");
        assert_eq!(read(&repo, "a.txt"), "second\n");

        repo.write("a.txt", "third\n");
        checkpoints
            .switch_branch(&local("other"), Changes::Leave)
            .await
            .expect("left");
        checkpoints
            .switch_branch(&local(&main), Changes::Bring)
            .await
            .expect("back");
        checkpoints
            .discard_stash(&stash_of(&checkpoints).await)
            .await
            .expect("discarded");
        assert_eq!(read(&repo, "a.txt"), "old\n");
        assert_eq!(
            repo.git(&["stash", "list", "--format=%s"]),
            format!("On {main}: the user's own\n")
        );
    });
}

#[test]
fn a_restore_git_refuses_or_that_conflicts_keeps_the_stash() {
    run(async {
        let repo = Repo::new("stash-refused");
        let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
        repo.git(&["branch", "other"]);
        repo.write("a.txt", "stashed\n");
        let checkpoints = Checkpoints::new(repo.path());
        checkpoints
            .switch_branch(&local("other"), Changes::Leave)
            .await
            .expect("left");
        checkpoints
            .switch_branch(&local(&main), Changes::Bring)
            .await
            .expect("back");

        repo.write("a.txt", "edited meanwhile\n");
        assert!(matches!(
            checkpoints.restore_stash(&stash_of(&checkpoints).await).await,
            Err(DiffError::WouldOverwrite(paths)) if paths == ["a.txt"]
        ));
        assert_eq!(read(&repo, "a.txt"), "edited meanwhile\n");

        repo.git(&["commit", "-q", "-am", "committed meanwhile"]);
        assert!(matches!(
            checkpoints.restore_stash(&stash_of(&checkpoints).await).await,
            Err(DiffError::RestoredWithConflicts(paths)) if paths == ["a.txt"]
        ));
        assert!(
            checkpoints.stash().await.expect("read").is_some(),
            "Git keeps the stash"
        );
    });
}

#[test]
fn a_detached_head_cannot_leave_changes_on_a_branch() {
    run(async {
        let repo = Repo::new("stash-detached");
        let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
        repo.git(&["switch", "-q", "--detach"]);
        repo.write("a.txt", "edited\n");
        let checkpoints = Checkpoints::new(repo.path());

        assert!(matches!(
            checkpoints
                .switch_branch(&local(&main), Changes::Leave)
                .await,
            Err(DiffError::NoBranch)
        ));
        assert_eq!(repo.git(&["stash", "list"]), "");
        assert_eq!(read(&repo, "a.txt"), "edited\n");
    });
}

#[test]
fn a_hook_failing_after_git_switched_keeps_the_changes_left_on_the_branch() {
    run(async {
        let repo = Repo::new("switch-hook");
        let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
        repo.git(&["branch", "other"]);
        repo.write("a.txt", "edited\n");
        hook(&repo, "post-checkout", 1);
        let checkpoints = Checkpoints::new(repo.path());

        let switched = checkpoints
            .switch_branch(&local("other"), Changes::Leave)
            .await;

        assert!(
            matches!(&switched, Err(DiffError::HookFailed(reason)) if reason.contains("hook failed")),
            "{switched:?}"
        );
        assert_eq!(repo.git(&["branch", "--show-current"]), "other\n");
        assert_eq!(read(&repo, "a.txt"), "old\n", "the changes stay on {main}");
        assert_eq!(
            repo.git(&["stash", "list", "--format=%s"]),
            format!("On {main}: Liquid: changes left on {main}\n")
        );
        assert!(matches!(
            checkpoints.create_branch("new").await,
            Err(DiffError::HookFailed(_))
        ));
        assert_eq!(repo.git(&["branch", "--show-current"]), "new\n");
    });
}

#[test]
fn no_restore_runs_over_unresolved_conflicts_or_untracked_files_in_the_way() {
    run(async {
        let repo = Repo::new("stash-unmerged");
        let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
        repo.git(&["branch", "other"]);
        repo.write("a.txt", "stashed\n");
        repo.write("u.txt", "untracked\n");
        let checkpoints = Checkpoints::new(repo.path());
        checkpoints
            .switch_branch(&local("other"), Changes::Leave)
            .await
            .expect("left");
        checkpoints
            .switch_branch(&local(&main), Changes::Bring)
            .await
            .expect("back");

        repo.write("u.txt", "in the way\n");
        assert!(matches!(
            checkpoints.restore_stash(&stash_of(&checkpoints).await).await,
            Err(DiffError::WouldOverwrite(paths)) if paths == ["u.txt"]
        ));
        assert_eq!(read(&repo, "a.txt"), "old\n", "nothing was restored");
        std::fs::remove_file(repo.path().join("u.txt")).expect("removed");

        // A merge the user stopped with a conflict.
        repo.git(&["switch", "-q", "other"]);
        repo.write("a.txt", "theirs\n");
        repo.git(&["commit", "-q", "-am", "theirs"]);
        repo.git(&["switch", "-q", &main]);
        repo.write("a.txt", "ours\n");
        repo.git(&["commit", "-q", "-am", "ours"]);
        let merged = Command::new("git")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "merge",
                "-q",
                "other",
            ])
            .current_dir(repo.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .expect("git runs");
        assert!(!merged.status.success(), "the merge stops at its conflict");

        assert!(matches!(
            checkpoints.restore_stash(&stash_of(&checkpoints).await).await,
            Err(DiffError::Unresolved(paths)) if paths == ["a.txt"]
        ));
        assert!(checkpoints.stash().await.expect("read").is_some());
    });
}

#[test]
fn a_stashed_files_patch_shows_tracked_and_untracked_changes() {
    run(async {
        let repo = Repo::new("stash-patch");
        let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
        repo.git(&["branch", "other"]);
        repo.write("a.txt", "stashed\n");
        repo.write("u.txt", "untracked\n");
        let checkpoints = Checkpoints::new(repo.path());
        checkpoints
            .switch_branch(&local("other"), Changes::Leave)
            .await
            .expect("left");
        checkpoints
            .switch_branch(&local(&main), Changes::Bring)
            .await
            .expect("back");
        let stash = stash_of(&checkpoints).await;

        let tracked = checkpoints
            .stash_patch(&stash, "a.txt", crate::Whitespace::Shown)
            .await
            .expect("patch");
        let untracked = checkpoints
            .stash_patch(&stash, "u.txt", crate::Whitespace::Shown)
            .await
            .expect("patch");

        assert!(tracked.contains("-old\n+stashed\n"), "{tracked}");
        assert!(untracked.contains("+untracked\n"), "{untracked}");
    });
}

#[test]
fn a_tag_named_like_the_branch_changes_nothing() {
    run(async {
        let clone = Clone::new("tag-name");
        let repo = &clone.repo;
        repo.git(&["branch", "other"]);
        repo.git(&["tag", "main"]);
        clone.commit_upstream("theirs\n");
        let checkpoints = Checkpoints::new(repo.path());

        assert!(checkpoints.fetch().await.expect("fetched"));
        repo.write("a.txt", "edited\n");
        checkpoints
            .switch_branch(&local("other"), Changes::Leave)
            .await
            .expect("left");
        let branches = checkpoints.branches().await.expect("branches");
        assert!(
            branches
                .branches
                .iter()
                .any(|branch| branch.name == "main" && branch.stashed)
        );
    });
}

#[test]
fn no_background_fetch_writes_outside_remote_tracking_branches() {
    run(async {
        let clone = Clone::new("fetch-refspec");
        let repo = &clone.repo;
        clone.origin.git(&["branch", "feature"]);
        repo.git(&["fetch", "-q"]);
        repo.git(&["branch", "feature", "origin/feature"]);
        repo.git(&[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/heads/feature:refs/heads/feature",
        ]);
        let before = repo.git(&["rev-parse", "feature"]);
        clone.origin.git(&["switch", "-q", "feature"]);
        clone.commit_upstream("theirs\n");
        let checkpoints = Checkpoints::new(repo.path());

        assert!(matches!(
            checkpoints.fetch().await,
            Err(DiffError::FetchOutsideRemotes(_))
        ));
        assert_eq!(repo.git(&["rev-parse", "feature"]), before);
    });
}

#[test]
fn a_switch_whose_caller_gives_up_still_finishes_and_leaves_no_lock() {
    run(async {
        let repo = Repo::new("switch-slow");
        let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
        repo.git(&["config", "filter.slow.smudge", "sleep 1; cat"]);
        repo.git(&["config", "filter.slow.clean", "cat"]);
        repo.git(&["switch", "-q", "-c", "other"]);
        repo.write(".gitattributes", "*.slow filter=slow\n");
        repo.write("f.slow", "slow\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "slow"]);
        repo.git(&["switch", "-q", &main]);
        repo.write("a.txt", "edited\n");
        let checkpoints = Checkpoints::new(repo.path());

        let given_up = tokio::time::timeout(
            Duration::from_millis(300),
            checkpoints.switch_branch(&local("other"), Changes::Leave),
        )
        .await;
        assert!(given_up.is_err(), "the switch outlasts its caller");
        checkpoints.close().await;

        let waited = std::time::Instant::now();
        while repo.git(&["branch", "--show-current"]) != "other\n" {
            assert!(
                waited.elapsed() < Duration::from_secs(20),
                "the switch finishes"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        while repo.path().join(".git/index.lock").exists() {
            assert!(
                waited.elapsed() < Duration::from_secs(20),
                "Git removes its lock"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(read(&repo, "f.slow"), "slow\n");
        assert_eq!(
            repo.git(&["stash", "list", "--format=%s"]),
            format!("On {main}: Liquid: changes left on {main}\n")
        );
    });
}
