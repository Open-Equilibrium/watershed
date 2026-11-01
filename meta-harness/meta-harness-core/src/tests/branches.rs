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

async fn leave_on(checkpoints: &Checkpoints) -> Changes {
    Changes::Leave {
        branch: checkpoints
            .status()
            .await
            .expect("status")
            .branch
            .unwrap_or_default(),
        stash: checkpoints
            .stash()
            .await
            .expect("stash")
            .map(|stash| stash.id),
    }
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
fn tracking_a_local_branch_keeps_both_local_branches_in_the_list() {
    run(async {
        let clone = Clone::new("branches-local-upstream");
        let repo = &clone.repo;
        repo.git(&["branch", "--track", "feature", "main"]);
        let checkpoints = Checkpoints::new(repo.path());

        let branches = checkpoints.branches().await.expect("branches");

        let names: Vec<(&str, bool, bool)> = branches
            .branches
            .iter()
            .map(|branch| (branch.name.as_str(), branch.remote, branch.current))
            .collect();
        assert_eq!(names, [("feature", false, false), ("main", false, true)]);
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
            checkpoints.switch_branch(&local("other"), leave_on(&checkpoints).await).await,
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
fn fetching_never_waits_for_an_ssh_passphrase_on_the_terminal() {
    const CHILD: &str = "WATERSHED_SSH_PROMPT_TEST";
    if std::env::var_os(CHILD).is_none() {
        // Run this case with a controlling terminal, as Liquid launched by flutter run has.
        let executable = std::env::current_exe().expect("test executable");
        let name = "tests::branches::fetching_never_waits_for_an_ssh_passphrase_on_the_terminal";
        let mut terminal = Command::new("script");
        terminal.arg("-q");
        #[cfg(target_os = "macos")]
        terminal
            .arg("/dev/null")
            .arg(&executable)
            .arg("--exact")
            .arg(name);
        #[cfg(not(target_os = "macos"))]
        terminal
            .args(["-e", "-c"])
            .arg(format!(
                "{} --exact {name}",
                super::shell_word(executable.to_str().expect("test path"))
            ))
            .arg("/dev/null");
        let output = terminal.env(CHILD, "1").output().expect("terminal starts");
        let output = String::from_utf8_lossy(&output.stdout);
        assert!(output.contains("1 passed; 0 failed"), "{output}");
        return;
    }
    run(async {
        let clone = Clone::new("fetch-ssh-prompt");
        let repo = &clone.repo;
        // An ephemeral test fixture, never an identity used to authenticate anywhere.
        let key = repo.path().join(".git/prompt-fixture");
        assert!(
            Command::new("ssh-keygen")
                .args(["-q", "-t", "ed25519", "-N", "test-passphrase", "-f"])
                .arg(&key)
                .status()
                .expect("fixture generated")
                .success()
        );
        repo.git(&[
            "remote",
            "set-url",
            "origin",
            "ssh://example.invalid/repository",
        ]);
        // OpenSSH's key reader uses the same terminal/askpass policy as its SSH client.
        repo.git(&[
            "config",
            "core.sshCommand",
            &format!(
                "sh -c {}",
                super::shell_word(format!(
                    "exec ssh-keygen -y -f {}",
                    super::shell_word(key.to_str().unwrap())
                ))
            ),
        ]);
        let checkpoints = Checkpoints::with_timeout(repo.path(), Duration::from_secs(2));

        let fetched = checkpoints.fetch().await;

        assert!(matches!(fetched, Err(DiffError::Git(_))), "{fetched:?}");
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
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
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
fn restoring_a_stash_requires_its_original_branch() {
    run(async {
        let repo = Repo::new("stash-original-branch");
        let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
        repo.git(&["branch", "other"]);
        repo.write("a.txt", "stashed\n");
        let checkpoints = Checkpoints::new(repo.path());
        checkpoints
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
            .await
            .expect("left");
        let stash = repo.git(&["rev-parse", "refs/stash"]).trim().to_owned();
        let index = std::fs::read(repo.path().join(".git/index")).expect("index");

        let restored = checkpoints.restore_stash(&stash).await;

        assert!(matches!(restored, Err(DiffError::NoStash)), "{restored:?}");
        assert_eq!(read(&repo, "a.txt"), "old\n");
        assert_eq!(repo.git(&["rev-parse", "refs/stash"]).trim(), stash);
        assert_eq!(
            std::fs::read(repo.path().join(".git/index")).expect("index"),
            index
        );
        checkpoints
            .switch_branch(&local(&main), Changes::Bring)
            .await
            .expect("back");
        checkpoints.restore_stash(&stash).await.expect("restored");
        assert_eq!(read(&repo, "a.txt"), "stashed\n");
        assert_eq!(repo.git(&["stash", "list"]), "");
    });
}

#[test]
fn restoring_or_rolling_back_a_stash_preserves_the_saved_index() {
    run(async {
        for target in ["other", "missing"] {
            let repo = Repo::new(&format!("stash-index-{target}"));
            let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
            repo.git(&["branch", "other"]);
            repo.write("a.txt", "staged\n");
            repo.write("new.txt", "only in the index\n");
            repo.git(&["add", "."]);
            let index = repo.git(&["write-tree"]);
            repo.write("a.txt", "old\n");
            let checkpoints = Checkpoints::new(repo.path());

            let switched = checkpoints
                .switch_branch(&local(target), leave_on(&checkpoints).await)
                .await;
            if target == "other" {
                switched.expect("switched");
                checkpoints
                    .switch_branch(&local(&main), Changes::Bring)
                    .await
                    .expect("back");
                checkpoints
                    .restore_stash(&stash_of(&checkpoints).await)
                    .await
                    .expect("restored");
            } else {
                assert!(switched.is_err(), "the missing branch refuses the switch");
            }

            assert_eq!(repo.git(&["write-tree"]), index, "{target}");
            assert_eq!(read(&repo, "a.txt"), "old\n");
            assert_eq!(read(&repo, "new.txt"), "only in the index\n");
            assert_eq!(repo.git(&["stash", "list"]), "");
            assert_eq!(repo.git(&["branch", "--show-current"]), format!("{main}\n"));
        }
    });
}

#[test]
fn a_stale_switch_confirmation_never_replaces_unconfirmed_changes() {
    run(async {
        for changed in ["absent", "replaced", "branch"] {
            let repo = Repo::new(&format!("switch-confirmed-{changed}"));
            let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
            repo.git(&["branch", "other"]);
            repo.git(&["branch", "next"]);
            let checkpoints = Checkpoints::new(repo.path());
            let another = Checkpoints::new(repo.path());
            if changed == "replaced" {
                repo.write("a.txt", "approved old stash\n");
                checkpoints
                    .switch_branch(&local("other"), leave_on(&checkpoints).await)
                    .await
                    .expect("left");
                checkpoints
                    .switch_branch(&local(&main), Changes::Bring)
                    .await
                    .expect("back");
            }
            let confirmed = leave_on(&checkpoints).await;
            repo.write("a.txt", "unconfirmed stash\n");
            another
                .switch_branch(&local("other"), leave_on(&another).await)
                .await
                .expect("another leaves");
            if changed != "branch" {
                another
                    .switch_branch(&local(&main), Changes::Bring)
                    .await
                    .expect("another returns");
            }
            repo.write("a.txt", "current edits\n");
            let before = repo.git(&["stash", "list", "--format=%H"]);
            let branch = repo.git(&["branch", "--show-current"]);
            let index = repo.git(&["write-tree"]);

            let switched = checkpoints.switch_branch(&local("next"), confirmed).await;

            assert!(switched.is_err(), "{changed}: {switched:?}");
            assert_eq!(repo.git(&["stash", "list", "--format=%H"]), before);
            assert_eq!(repo.git(&["branch", "--show-current"]), branch);
            assert_eq!(repo.git(&["write-tree"]), index);
            assert_eq!(read(&repo, "a.txt"), "current edits\n");
        }
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
                .switch_branch(&local("other"), leave_on(&checkpoints).await)
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
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
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
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
            .await
            .expect("left");
        checkpoints
            .switch_branch(&local(&main), Changes::Bring)
            .await
            .expect("back");

        repo.write("a.txt", "edited meanwhile\n");
        assert!(matches!(
            checkpoints
                .restore_stash(&stash_of(&checkpoints).await)
                .await,
            Err(DiffError::Uncommitted(1))
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
                .switch_branch(&local(&main), leave_on(&checkpoints).await)
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
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
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
fn no_restore_runs_over_uncommitted_changes_or_ignored_files_in_the_way() {
    run(async {
        let repo = Repo::new("stash-unmerged");
        let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
        repo.git(&["branch", "other"]);
        repo.write("a.txt", "stashed\n");
        repo.write("u.txt", "untracked\n");
        let checkpoints = Checkpoints::new(repo.path());
        checkpoints
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
            .await
            .expect("left");
        checkpoints
            .switch_branch(&local(&main), Changes::Bring)
            .await
            .expect("back");

        // As GitHub Desktop, a restore needs a checkout without changes; Git would restore part
        // of a stash whose tracked file was edited meanwhile and keep it.
        repo.write("a.txt", "edited meanwhile\n");
        assert!(matches!(
            checkpoints
                .restore_stash(&stash_of(&checkpoints).await)
                .await,
            Err(DiffError::Uncommitted(1))
        ));
        assert!(!repo.path().join("u.txt").exists(), "nothing was restored");
        repo.git(&["checkout", "--", "a.txt"]);

        repo.write("u.txt", "in the way\n");
        assert!(matches!(
            checkpoints
                .restore_stash(&stash_of(&checkpoints).await)
                .await,
            Err(DiffError::Uncommitted(1))
        ));
        // An ignored file counts as no change, but is in the way.
        std::fs::write(repo.path().join(".git/info/exclude"), "u.txt\n").expect("excluded");
        assert!(matches!(
            checkpoints.restore_stash(&stash_of(&checkpoints).await).await,
            Err(DiffError::WouldOverwrite(paths)) if paths == ["u.txt"]
        ));
        assert_eq!(read(&repo, "a.txt"), "old\n", "nothing was restored");
        std::fs::remove_file(repo.path().join("u.txt")).expect("removed");
        std::fs::remove_file(repo.path().join(".git/info/exclude")).expect("included");

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
            checkpoints
                .restore_stash(&stash_of(&checkpoints).await)
                .await,
            Err(DiffError::Uncommitted(_))
        ));
        assert!(checkpoints.stash().await.expect("read").is_some());
    });
}

#[test]
fn leaving_changes_never_cleans_up_unsaved_ignored_contents() {
    run(async {
        for placement in ["at-file", "at-parent"] {
            let repo = Repo::new(&format!("leave-ignored-{placement}"));
            let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
            if placement == "at-parent" {
                repo.git(&["mv", "a.txt", "saved.txt"]);
                std::fs::create_dir(repo.path().join("a.txt")).expect("folder");
                repo.git(&["mv", "saved.txt", "a.txt/child"]);
                repo.git(&["commit", "-qm", "nested file"]);
                std::fs::remove_file(repo.path().join("a.txt/child")).expect("removed");
                std::fs::remove_dir(repo.path().join("a.txt")).expect("removed folder");
                repo.write("a.txt", "private\n");
            } else {
                std::fs::remove_file(repo.path().join("a.txt")).expect("removed");
                std::fs::create_dir(repo.path().join("a.txt")).expect("folder");
                repo.write("a.txt/private", "private\n");
            }
            let protected = if placement == "at-file" {
                "a.txt/private"
            } else {
                "a.txt"
            };
            std::fs::write(repo.path().join(".git/info/exclude"), "a.txt\n").expect("excluded");
            repo.git(&["branch", "other"]);
            let index = std::fs::read(repo.path().join(".git/index")).expect("index");
            let checkpoints = Checkpoints::new(repo.path());

            let switched = checkpoints
                .switch_branch(&local("other"), leave_on(&checkpoints).await)
                .await;

            assert_eq!(
                read(&repo, protected),
                "private\n",
                "{placement}: {switched:?}"
            );
            assert!(matches!(switched, Err(DiffError::WouldOverwrite(_))));
            assert_eq!(repo.git(&["branch", "--show-current"]).trim(), main);
            assert!(repo.git(&["stash", "list"]).is_empty());
            assert_eq!(
                std::fs::read(repo.path().join(".git/index")).expect("index"),
                index
            );
        }
    });
}

#[test]
fn no_restore_exempts_a_tracked_parent_committed_after_the_stash() {
    run(async {
        let repo = Repo::new("stash-later-parent");
        let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
        repo.git(&["branch", "other"]);
        repo.write("a.txt", "stashed\n");
        std::fs::create_dir(repo.path().join("dir")).expect("folder");
        repo.write("dir/new.txt", "untracked\n");
        let checkpoints = Checkpoints::new(repo.path());
        checkpoints
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
            .await
            .expect("left");
        checkpoints
            .switch_branch(&local(&main), Changes::Bring)
            .await
            .expect("back");
        repo.write("dir", "later commit\n");
        repo.git(&["add", "dir"]);
        repo.git(&["commit", "-qm", "add parent"]);
        let stash = stash_of(&checkpoints).await;
        let index = std::fs::read(repo.path().join(".git/index")).expect("index");

        let restored = checkpoints.restore_stash(&stash).await;

        assert_eq!(read(&repo, "a.txt"), "old\n", "{restored:?}");
        assert!(
            matches!(restored, Err(DiffError::WouldOverwrite(paths)) if paths == ["dir/new.txt"])
        );
        assert_eq!(read(&repo, "dir"), "later commit\n");
        assert_eq!(stash_of(&checkpoints).await, stash);
        assert_eq!(
            std::fs::read(repo.path().join(".git/index")).expect("index"),
            index
        );
    });
}

#[test]
fn a_stashed_file_can_return_as_a_directory() {
    run(async {
        for staged in [false, true] {
            let repo = Repo::new(&format!("stash-file-directory-{staged}"));
            let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
            repo.git(&["branch", "other"]);
            if staged {
                repo.git(&["rm", "-q", "a.txt"]);
            } else {
                std::fs::remove_file(repo.path().join("a.txt")).expect("removed");
            }
            std::fs::create_dir(repo.path().join("a.txt")).expect("folder");
            repo.write("a.txt/new.txt", "untracked\n");
            let checkpoints = Checkpoints::new(repo.path());
            checkpoints
                .switch_branch(&local("other"), leave_on(&checkpoints).await)
                .await
                .expect("left");
            checkpoints
                .switch_branch(&local(&main), Changes::Bring)
                .await
                .expect("back");

            let stash = stash_of(&checkpoints).await;
            let restored = checkpoints.restore_stash(&stash).await;
            if !staged {
                // Git saved the tracked file unchanged, so its untracked child really is
                // obstructed. Native pop would fail after starting the restoration.
                assert!(matches!(restored, Err(DiffError::WouldOverwrite(_))));
                assert_eq!(read(&repo, "a.txt"), "old\n");
                assert_eq!(stash_of(&checkpoints).await, stash);
                continue;
            }
            restored.expect("restored after tracked deletion");

            assert_eq!(read(&repo, "a.txt/new.txt"), "untracked\n");
            assert!(repo.git(&["ls-files", "--", "a.txt"]).is_empty());
            assert!(checkpoints.stash().await.expect("stash").is_none());
        }
    });
}

#[test]
fn no_restore_partly_applies_a_stash_with_undecodable_paths() {
    use std::io::Write;
    use std::process::Stdio;

    run(async {
        let repo = Repo::new("stash-undecodable");
        let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
        repo.git(&["branch", "other"]);
        repo.write("a.txt", "stashed\n");
        let checkpoints = Checkpoints::new(repo.path());
        checkpoints
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
            .await
            .expect("left");
        checkpoints
            .switch_branch(&local(&main), Changes::Bring)
            .await
            .expect("back");
        let original = stash_of(&checkpoints).await;
        let blob = repo.git(&["hash-object", "-w", "a.txt"]);
        // Git objects can contain these pathname bytes even on a host whose filesystem cannot.
        let mut make_tree = Command::new("git")
            .args(["mktree", "-z"])
            .current_dir(repo.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("mktree runs");
        let mut input = make_tree.stdin.take().expect("stdin");
        input
            .write_all(format!("100644 blob {}\t", blob.trim()).as_bytes())
            .expect("tree entry");
        input.write_all(b"\xff\0").expect("pathname");
        drop(input);
        let tree = make_tree.wait_with_output().expect("tree");
        assert!(tree.status.success());
        let tree = String::from_utf8(tree.stdout).expect("tree id");
        let untracked = repo.git(&["commit-tree", tree.trim(), "-m", "untracked"]);
        let tracked = repo.git(&["rev-parse", &format!("{original}^{{tree}}")]);
        let base = repo.git(&["rev-parse", &format!("{original}^1")]);
        let index = repo.git(&["rev-parse", &format!("{original}^2")]);
        let message = format!("On {main}: Liquid: changes left on {main}");
        let stash = repo.git(&[
            "commit-tree",
            tracked.trim(),
            "-p",
            base.trim(),
            "-p",
            index.trim(),
            "-p",
            untracked.trim(),
            "-m",
            &message,
        ]);
        repo.git(&["stash", "store", "-m", &message, stash.trim()]);
        let index_before = std::fs::read(repo.path().join(".git/index")).expect("index");

        let restored = checkpoints.restore_stash(stash.trim()).await;

        assert_eq!(read(&repo, "a.txt"), "old\n", "{restored:?}");
        assert!(matches!(restored, Err(DiffError::Git(reason)) if reason.contains("UTF-8")));
        assert_eq!(stash_of(&checkpoints).await, stash.trim());
        assert_eq!(
            std::fs::read(repo.path().join(".git/index")).expect("index"),
            index_before
        );
    });
}

#[test]
fn leaving_changes_never_loses_edits_hidden_from_native_git() {
    use std::os::unix::fs::PermissionsExt;

    run(async {
        for mark in ["--assume-unchanged", "--skip-worktree", "file-mode"] {
            let repo = Repo::new(&format!("leave-hidden-{mark}"));
            repo.write("b.txt", "old b\n");
            repo.git(&["add", "b.txt"]);
            repo.git(&["commit", "-q", "-m", "add b"]);
            let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
            repo.git(&["branch", "other"]);
            let expected = if mark == "file-mode" {
                repo.git(&["config", "core.fileMode", "false"]);
                std::fs::set_permissions(
                    repo.path().join("a.txt"),
                    std::fs::Permissions::from_mode(0o755),
                )
                .expect("executable");
                "old\n"
            } else {
                repo.git(&["update-index", mark, "a.txt"]);
                repo.write("a.txt", "hidden edit\n");
                "hidden edit\n"
            };
            repo.write("b.txt", "visible edit\n");
            let mode = std::fs::metadata(repo.path().join("a.txt"))
                .expect("a metadata")
                .permissions()
                .mode();
            let index = std::fs::read(repo.path().join(".git/index")).expect("index");
            let checkpoints = Checkpoints::new(repo.path());

            let switched = checkpoints
                .switch_branch(&local("other"), leave_on(&checkpoints).await)
                .await;

            assert_eq!(read(&repo, "a.txt"), expected, "{mark}: {switched:?}");
            assert_eq!(
                std::fs::metadata(repo.path().join("a.txt"))
                    .expect("a metadata")
                    .permissions()
                    .mode(),
                mode,
                "{mark}: {switched:?}"
            );
            assert_eq!(read(&repo, "b.txt"), "visible edit\n");
            assert!(
                matches!(switched, Err(DiffError::Git(reason)) if reason.contains("Git would skip")),
                "{mark}: leaving must refuse before stashing"
            );
            assert_eq!(repo.git(&["branch", "--show-current"]).trim(), main);
            assert!(repo.git(&["stash", "list"]).is_empty());
            assert_eq!(
                std::fs::read(repo.path().join(".git/index")).expect("index"),
                index
            );
        }
    });
}

#[test]
fn no_restore_partly_applies_a_stash_over_hidden_tracked_edits() {
    run(async {
        for mark in ["--assume-unchanged", "--skip-worktree"] {
            let repo = Repo::new(&format!("stash-hidden-{mark}"));
            let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
            repo.git(&["branch", "other"]);
            repo.write("a.txt", "stashed\n");
            repo.write("new.txt", "untracked\n");
            let checkpoints = Checkpoints::new(repo.path());
            checkpoints
                .switch_branch(&local("other"), leave_on(&checkpoints).await)
                .await
                .expect("left");
            checkpoints
                .switch_branch(&local(&main), Changes::Bring)
                .await
                .expect("back");
            repo.git(&["update-index", mark, "a.txt"]);
            repo.write("a.txt", "hidden edit\n");
            let index = std::fs::read(repo.path().join(".git/index")).expect("index");
            let stash = stash_of(&checkpoints).await;

            let restored = checkpoints.restore_stash(&stash).await;

            assert!(
                !repo.path().join("new.txt").exists(),
                "{mark}: nothing should be restored, got {restored:?}"
            );
            assert!(
                matches!(restored, Err(DiffError::Uncommitted(1))),
                "{mark}: {restored:?}"
            );
            assert_eq!(read(&repo, "a.txt"), "hidden edit\n");
            assert_eq!(checkpoints.status().await.expect("status").changed, 1);
            assert_eq!(
                std::fs::read(repo.path().join(".git/index")).expect("index"),
                index
            );
            assert_eq!(stash_of(&checkpoints).await, stash);
        }
    });
}

#[test]
fn a_stashed_case_only_rename_restores_without_an_ignored_obstruction() {
    run(async {
        let repo = Repo::new("stash-case-only");
        let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
        repo.git(&["branch", "other"]);
        repo.git(&["mv", "a.txt", "A.txt"]);
        let checkpoints = Checkpoints::new(repo.path());
        checkpoints
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
            .await
            .expect("left");
        checkpoints
            .switch_branch(&local(&main), Changes::Bring)
            .await
            .expect("back");

        checkpoints
            .restore_stash(&stash_of(&checkpoints).await)
            .await
            .expect("the tracked file's other spelling is no ignored obstruction");

        assert_eq!(read(&repo, "A.txt"), "old\n");
        assert_eq!(repo.git(&["ls-files"]), "A.txt\n");
        assert!(checkpoints.stash().await.expect("stash").is_none());
    });
}

#[test]
fn no_restore_overwrites_ignored_paths_from_the_stashed_index_or_working_tree() {
    run(async {
        for saved in ["staged", "index-only", "deleted-since-stash"] {
            for obstruction in ["file", "parent"] {
                let repo = Repo::new(&format!("stash-ignored-{saved}-{obstruction}"));
                let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
                std::fs::create_dir(repo.path().join("dir")).expect("folder");
                repo.write("dir/saved.txt", "saved\n");
                repo.git(&["add", "dir/saved.txt"]);
                if saved == "deleted-since-stash" {
                    repo.git(&["commit", "-q", "-m", "tracked"]);
                    repo.write("dir/saved.txt", "changed\n");
                } else if saved == "index-only" {
                    std::fs::remove_file(repo.path().join("dir/saved.txt")).expect("removed");
                }
                repo.git(&["branch", "other"]);
                repo.write("a.txt", "stashed\n");
                let checkpoints = Checkpoints::new(repo.path());
                checkpoints
                    .switch_branch(&local("other"), leave_on(&checkpoints).await)
                    .await
                    .expect("left");
                checkpoints
                    .switch_branch(&local(&main), Changes::Bring)
                    .await
                    .expect("back");
                if saved == "deleted-since-stash" {
                    repo.git(&["rm", "-q", "dir/saved.txt"]);
                    repo.git(&["commit", "-q", "-m", "removed"]);
                }
                let path = if obstruction == "file" {
                    std::fs::create_dir_all(repo.path().join("dir")).expect("folder");
                    "dir/saved.txt"
                } else {
                    let _ = std::fs::remove_dir(repo.path().join("dir"));
                    "dir"
                };
                std::fs::write(repo.path().join(".git/info/exclude"), "dir\n").expect("excluded");
                repo.write(path, "keep ignored contents\n");
                assert_eq!(checkpoints.status().await.expect("status").changed, 0);
                let index = repo.git(&["write-tree"]);
                let stash = stash_of(&checkpoints).await;

                let restored = checkpoints.restore_stash(&stash).await;

                assert!(
                    matches!(&restored, Err(DiffError::WouldOverwrite(paths))
                        if paths == &[path]),
                    "{saved}/{obstruction}: {restored:?}"
                );
                assert_eq!(read(&repo, path), "keep ignored contents\n");
                assert_eq!(read(&repo, "a.txt"), "old\n", "nothing was restored");
                assert_eq!(repo.git(&["write-tree"]), index);
                assert_eq!(stash_of(&checkpoints).await, stash);
            }
        }
    });
}

#[test]
fn no_restore_applies_tracked_changes_when_an_untracked_parent_is_obstructed() {
    run(async {
        for obstruction in ["file", "symlink"] {
            let repo = Repo::new(&format!("stash-parent-{obstruction}"));
            let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
            repo.git(&["branch", "other"]);
            repo.write("a.txt", "stashed\n");
            std::fs::create_dir_all(repo.path().join("dir/nested")).expect("folders");
            repo.write("dir/nested/new.txt", "untracked\n");
            let checkpoints = Checkpoints::new(repo.path());
            checkpoints
                .switch_branch(&local("other"), leave_on(&checkpoints).await)
                .await
                .expect("left");
            checkpoints
                .switch_branch(&local(&main), Changes::Bring)
                .await
                .expect("back");
            // Ignored, so the checkout has no changes and only the obstruction stops it.
            std::fs::write(repo.path().join(".git/info/exclude"), "dir\n").expect("excluded");
            match obstruction {
                "file" => repo.write("dir", "in the way\n"),
                _ => std::os::unix::fs::symlink("absent", repo.path().join("dir"))
                    .expect("dangling link"),
            }
            let index = repo.git(&["write-tree"]);
            let saved = stash_of(&checkpoints).await;

            let restored = checkpoints.restore_stash(&saved).await;

            assert_eq!(read(&repo, "a.txt"), "old\n", "{obstruction}: {restored:?}");
            assert_eq!(repo.git(&["write-tree"]), index);
            assert!(matches!(restored, Err(DiffError::WouldOverwrite(paths))
                if paths == ["dir/nested/new.txt"]));
            assert_eq!(stash_of(&checkpoints).await, saved);
        }
    });
}

#[test]
fn a_stash_preview_includes_changes_saved_only_in_the_index() {
    run(async {
        let repo = Repo::new("stash-index-preview");
        let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
        repo.git(&["branch", "other"]);
        repo.write("a.txt", "staged\n");
        repo.git(&["add", "a.txt"]);
        repo.write("a.txt", "old\n");
        let checkpoints = Checkpoints::new(repo.path());
        checkpoints
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
            .await
            .expect("left");
        checkpoints
            .switch_branch(&local(&main), Changes::Bring)
            .await
            .expect("back");

        let stash = checkpoints.stash().await.expect("read").expect("saved");
        let paths: Vec<&str> = stash.files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(
            paths,
            ["a.txt"],
            "discard counts include index-only changes"
        );
        let patch = checkpoints
            .stash_patch(&stash.id, "a.txt", crate::Whitespace::Shown)
            .await
            .expect("patch");
        assert!(patch.contains("-old\n+staged\n"), "{patch}");
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
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
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
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
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
fn worktrees_serialize_leaving_restoring_and_discarding_stashes() {
    run(async {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::Arc;

        for action in ["restore", "discard", "leave"] {
            let repo = Repo::new(&format!("stash-worktrees-{action}"));
            let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
            let other = Repo(repo.path().with_extension("worktree"));
            let _ = std::fs::remove_dir_all(other.path());
            repo.git(&[
                "worktree",
                "add",
                "-q",
                "-b",
                "side",
                other.path().to_str().unwrap(),
            ]);
            repo.git(&["branch", "next"]);
            repo.git(&["branch", "away"]);
            let side = Arc::new(Checkpoints::new(other.path()));
            other.write("a.txt", "side's saved changes\n");
            side.switch_branch(&local("away"), leave_on(&side).await)
                .await
                .expect("left on side");
            side.switch_branch(&local("side"), Changes::Bring)
                .await
                .expect("back on side");
            let saved = stash_of(&side).await;
            if action == "leave" {
                other.write("a.txt", "side's replacement\n");
            }

            let entered = repo.path().join(".git/stash-switching");
            let release = repo.path().join(".git/release-stash");
            let hook = repo.path().join(".git/hooks/post-checkout");
            std::fs::write(
                &hook,
                format!(
                    "#!/bin/sh\n: > {}\nwhile ! test -e {}; do sleep 0.02; done\n",
                    super::shell_word(entered.display()),
                    super::shell_word(release.display()),
                ),
            )
            .expect("hook");
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755))
                .expect("executable");
            repo.write("a.txt", "main's saved changes\n");
            let mine = Arc::new(Checkpoints::new(repo.path()));
            let switching = tokio::spawn({
                let mine = mine.clone();
                async move {
                    mine.switch_branch(&local("next"), leave_on(&mine).await)
                        .await
                }
            });
            tokio::time::timeout(Duration::from_secs(5), async {
                while !entered.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("the switch reaches its hook");

            let before = repo.git(&["stash", "list", "--format=%H"]);
            let mut writing = tokio::spawn({
                let side = side.clone();
                async move {
                    match action {
                        "restore" => side.restore_stash(&saved).await,
                        "discard" => side.discard_stash(&saved).await,
                        _ => {
                            side.switch_branch(&local("away"), leave_on(&side).await)
                                .await
                        }
                    }
                }
            });
            let early = tokio::time::timeout(Duration::from_millis(200), &mut writing).await;
            let waited = early.is_err();
            let during = repo.git(&["stash", "list", "--format=%H"]);
            std::fs::write(&release, "").expect("released");
            switching.await.expect("joined switch").expect("switched");
            match early {
                Ok(result) => result,
                Err(_) => tokio::time::timeout(Duration::from_secs(5), writing)
                    .await
                    .expect("the stash write resumes"),
            }
            .expect("joined stash write")
            .expect("stash written");

            assert!(
                waited,
                "{action} waits for the other worktree's stash write"
            );
            assert_eq!(during, before, "{action} changes no stash while waiting");
            assert!(
                repo.git(&["stash", "list", "--format=%gs"])
                    .contains(&format!("Liquid: changes left on {main}"))
            );
            if action == "restore" {
                assert_eq!(read(&other, "a.txt"), "side's saved changes\n");
            }
        }
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
            checkpoints.switch_branch(&local("other"), leave_on(&checkpoints).await),
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
