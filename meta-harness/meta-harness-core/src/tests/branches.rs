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

/// Readable subjects with each new operation token verified and removed for comparison.
fn stash_subjects(repo: &Repo) -> String {
    repo.git(&["stash", "list", "--format=%s"])
        .lines()
        .map(|subject| {
            if subject.contains("Liquid: changes left on ") {
                let (label, token) = subject.rsplit_once(" · ").expect("operation token");
                assert_eq!(token.len(), 16, "short operation token");
                assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
                format!("{label}\n")
            } else {
                format!("{subject}\n")
            }
        })
        .collect()
}

fn read(repo: &Repo, path: &str) -> String {
    std::fs::read_to_string(repo.path().join(path)).expect("file read")
}

#[test]
fn branch_and_stash_dates_outside_millisecond_range_return_an_error() {
    run(async {
        for reference in ["refs/heads/future", "refs/stash"] {
            let repo = Repo::new("branch-date-overflow");
            let tree = repo.git(&["rev-parse", "HEAD^{tree}"]);
            repo.write(
            ".git/future-commit",
            &format!(
                "tree {}\nauthor Test <test@example.com> 9223372036854776 +0000\ncommitter Test <test@example.com> 9223372036854776 +0000\n\nfuture\n",
                tree.trim()
            ),
        );
            let future = repo.git(&["hash-object", "-t", "commit", "-w", ".git/future-commit"]);
            repo.git(&[
                "update-ref",
                "--create-reflog",
                "-m",
                "On main: Liquid: changes left on main",
                reference,
                future.trim(),
            ]);

            let listed = Checkpoints::new(repo.path()).branches().await;

            assert!(matches!(listed, Err(DiffError::Git(_))), "{listed:?}");
        }
    });
}

#[test]
fn leaving_changes_preserves_a_nested_repository_replacing_a_tracked_file() {
    run(async {
        let repo = Repo::new("stash-nested-repository");
        repo.write("p", "tracked\n");
        repo.git(&["add", "p"]);
        repo.git(&["commit", "-qm", "track p"]);
        repo.git(&["branch", "other"]);
        std::fs::remove_file(repo.path().join("p")).expect("remove tracked file");
        let nested = Repo::at(repo.path().join("p"));
        nested.write("a.txt", "unsaved nested contents\n");
        let nested_head = nested.git(&["rev-parse", "HEAD"]);
        repo.write("a.txt", "unsaved parent contents\n");
        let before = repo.git(&["status", "--porcelain"]);
        let index = std::fs::read(repo.path().join(".git/index")).expect("index");
        let checkpoints = Checkpoints::new(repo.path());

        let switched = checkpoints
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
            .await;

        assert!(
            matches!(switched, Err(DiffError::SubmoduleChanged(ref paths)) if paths == &["p"]),
            "{switched:?}"
        );
        assert_eq!(read(&nested, "a.txt"), "unsaved nested contents\n");
        assert_eq!(nested.git(&["rev-parse", "HEAD"]), nested_head);
        assert_eq!(read(&repo, "a.txt"), "unsaved parent contents\n");
        assert_eq!(repo.git(&["status", "--porcelain"]), before);
        assert_eq!(
            std::fs::read(repo.path().join(".git/index")).expect("index"),
            index
        );
        assert!(repo.git(&["stash", "list"]).is_empty());
    });
}

#[test]
fn a_status_stash_stays_on_its_captured_branch_after_an_external_switch() {
    run(async {
        let repo = Repo::new("status-stash-branch");
        repo.git(&["branch", "-M", "main"]);
        repo.git(&["branch", "other"]);
        let checkpoints = Checkpoints::new(repo.path());
        repo.write("a.txt", "main changes\n");
        checkpoints
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
            .await
            .expect("main left");
        repo.write("a.txt", "other changes\n");
        checkpoints
            .switch_branch(&local("main"), leave_on(&checkpoints).await)
            .await
            .expect("other left");
        let (status, expected) = checkpoints
            .status_with_stash()
            .await
            .expect("status and stash");
        let tree = crate::diff::WorkingTree::open(repo.path())
            .await
            .expect("tree");
        repo.git(&["switch", "-q", "other"]);

        let stashed = tree
            .stash_for(status.branch.as_deref(), status.commit.as_deref())
            .await
            .expect("captured branch stash");

        assert_eq!(stashed, expected);
        assert_ne!(
            stashed.expect("main stash").id,
            stash_of(&checkpoints).await
        );
        assert_eq!(repo.git(&["branch", "--show-current"]), "other\n");
    });
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
                default_branch: Some("main".to_owned()),
                merging: false,
                conflicts: Vec::new(),
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
fn the_bounded_branch_list_keeps_the_current_branch_and_recent_order() {
    run(async {
        let repo = Repo::new("branches-current-limit");
        let current = repo.git(&["branch", "--show-current"]).trim().to_owned();
        let timestamp = repo
            .git(&["show", "-s", "--format=%ct", "HEAD"])
            .trim()
            .parse::<u64>()
            .expect("commit timestamp")
            + 1;
        let newer = Command::new("git")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit-tree",
                "HEAD^{tree}",
                "-p",
                "HEAD",
                "-m",
                "newer",
            ])
            .current_dir(repo.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_COMMITTER_DATE", format!("@{timestamp} +0000"))
            .output()
            .expect("commit created");
        assert!(newer.status.success(), "{newer:?}");
        let newer = String::from_utf8(newer.stdout).expect("commit id");
        let refs: String = (0..1000)
            .map(|i| format!("{} refs/heads/newer/{i:04}\n", newer.trim()))
            .collect();
        repo.write(".git/packed-refs", &refs);
        let checkpoints = Checkpoints::new(repo.path());

        let branches = checkpoints.branches().await.expect("branches").branches;

        assert_eq!(branches.len(), 1000);
        let source = branches
            .iter()
            .find(|branch| branch.current)
            .expect("current branch retained");
        assert_eq!(source.name, current);
        assert_eq!(branches.first().expect("first").name, "newer/0000");
        assert_eq!(branches.last().expect("last").name, current);
        assert!(
            branches
                .windows(2)
                .all(|pair| pair[0].committed_ms >= pair[1].committed_ms)
        );
        repo.write("a.txt", "edited\n");
        checkpoints
            .switch_branch(
                &local("newer/0000"),
                Changes::Leave {
                    branch: source.name.clone(),
                    stash: None,
                },
            )
            .await
            .expect("changes left on the listed source branch");
        assert_eq!(read(&repo, "a.txt"), "old\n");
        assert_eq!(
            stash_subjects(&repo),
            format!("On {current}: Liquid: changes left on {current}\n")
        );
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
        clone.origin.write("release.txt", "release only\n");
        clone.origin.git(&["add", "release.txt"]);
        clone.origin.git(&["commit", "-qm", "release file"]);
        clone.origin.git(&["branch", "release"]);
        let repo = &clone.repo;
        repo.git(&["fetch", "-q"]);
        repo.git(&["branch", "feature"]);
        repo.git(&["branch", "origin/release"]);
        let remote = repo.git(&["rev-parse", "refs/remotes/origin/release"]);
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
        assert_eq!(repo.git(&["rev-parse", "HEAD"]), remote);
        assert_eq!(read(repo, "release.txt"), "release only\n");
        assert_eq!(
            repo.git(&["rev-parse", "--symbolic-full-name", "release@{upstream}"]),
            "refs/remotes/origin/release\n"
        );
    });
}

#[test]
fn switching_never_overwrites_ignored_files_in_the_destination_branch() {
    run(async {
        for remote in [false, true] {
            for leave in [false, true] {
                let clone = Clone::new(&format!("switch-ignored-{remote}-{leave}"));
                clone.origin.git(&["switch", "-q", "-c", "release"]);
                clone.origin.write("private.txt", "committed\n");
                clone.origin.git(&["add", "private.txt"]);
                clone.origin.git(&["commit", "-qm", "add destination file"]);
                let repo = &clone.repo;
                repo.git(&["fetch", "-q"]);
                let target = if remote {
                    BranchTarget::Remote("origin/release".to_owned())
                } else {
                    repo.git(&["branch", "--track", "release", "origin/release"]);
                    local("release")
                };
                repo.write(".git/info/exclude", "private.txt\n");
                repo.write("private.txt", "unsaved ignored contents\n");
                repo.write("a.txt", "edited\n");
                let index = repo.git(&["write-tree"]);
                let checkpoints = Checkpoints::new(repo.path());
                let changes = if leave {
                    leave_on(&checkpoints).await
                } else {
                    Changes::Bring
                };

                let switched = checkpoints.switch_branch(&target, changes).await;

                assert!(
                    matches!(switched, Err(DiffError::WouldOverwrite(ref paths)) if paths == &["private.txt"]),
                    "remote={remote}, leave={leave}: {switched:?}"
                );
                assert_eq!(repo.git(&["branch", "--show-current"]), "main\n");
                assert_eq!(read(repo, "private.txt"), "unsaved ignored contents\n");
                assert_eq!(read(repo, "a.txt"), "edited\n");
                assert_eq!(repo.git(&["write-tree"]), index);
                assert_eq!(repo.git(&["stash", "list"]), "");
                if remote {
                    assert_eq!(repo.git(&["branch", "--list", "release"]), "");
                }
            }
        }
    });
}

#[test]
fn repeated_identical_saves_remain_distinct_and_restore_after_refusal() {
    const CHILD: &str = "WATERSHED_STASH_ID_TEST";
    if std::env::var_os(CHILD).is_none() {
        let output = Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "tests::branches::repeated_identical_saves_remain_distinct_and_restore_after_refusal",
            ])
            .env(CHILD, "1")
            .env("GIT_AUTHOR_DATE", "2001-02-03T04:05:06+0000")
            .env("GIT_COMMITTER_DATE", "2001-02-03T04:05:06+0000")
            .output()
            .expect("test starts");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        return;
    }
    run(async {
        let repo = Repo::new("stash-same-commit");
        repo.git(&["branch", "-M", "main"]);
        repo.git(&["switch", "-qc", "other"]);
        repo.write("private.txt", "committed\n");
        repo.git(&["add", "private.txt"]);
        repo.git(&["commit", "-qm", "add destination file"]);
        repo.git(&["switch", "-q", "main"]);
        repo.write("a.txt", "indexed\n");
        repo.git(&["add", "a.txt"]);
        repo.write("a.txt", "working\n");
        let checkpoints = Checkpoints::new(repo.path());
        checkpoints
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
            .await
            .expect("initial changes left");
        checkpoints
            .switch_branch(&local("main"), Changes::Bring)
            .await
            .expect("back");
        repo.git(&["stash", "apply", "--quiet", "--index"]);
        repo.write(".git/info/exclude", "private.txt\n");
        repo.write("private.txt", "unsaved ignored contents\n");
        let index = repo.git(&["write-tree"]);
        let stashes = repo.git(&["stash", "list", "--format=%H"]);

        let switched = checkpoints
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
            .await;

        assert!(
            matches!(switched, Err(DiffError::WouldOverwrite(_))),
            "{switched:?}"
        );
        assert_eq!(repo.git(&["branch", "--show-current"]), "main\n");
        assert_eq!(read(&repo, "a.txt"), "working\n");
        assert_eq!(read(&repo, "private.txt"), "unsaved ignored contents\n");
        assert_eq!(repo.git(&["write-tree"]), index);
        assert_eq!(repo.git(&["stash", "list", "--format=%H"]), stashes);

        std::fs::remove_file(repo.path().join("private.txt")).expect("remove obstruction");
        checkpoints
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
            .await
            .expect("same changes left after switching");
        assert_eq!(repo.git(&["branch", "--show-current"]), "other\n");
        assert_eq!(read(&repo, "a.txt"), "old\n");
        let replaced = repo.git(&["stash", "list", "--format=%H"]);
        assert_eq!(
            replaced.lines().count(),
            1,
            "the earlier entry was replaced"
        );
        assert_ne!(
            replaced, stashes,
            "the token prevents identical-commit reuse"
        );
        assert_eq!(
            stash_subjects(&repo),
            "On main: Liquid: changes left on main\n"
        );
        checkpoints
            .switch_branch(&local("main"), Changes::Bring)
            .await
            .expect("back again");
        checkpoints
            .restore_stash(&stash_of(&checkpoints).await)
            .await
            .expect("repeated save restored");
        assert_eq!(read(&repo, "a.txt"), "working\n");
        assert_eq!(repo.git(&["write-tree"]), index);
        assert!(repo.git(&["stash", "list"]).is_empty());
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
        repo.git(&["config", "fetch.writeCommitGraph", "true"]);
        let checkpoints = Checkpoints::new(repo.path());
        clone.commit_upstream("theirs\n");
        clone.origin.git(&["tag", "v1"]);
        let _ = std::fs::remove_file(repo.path().join(".git/FETCH_HEAD"));

        assert!(checkpoints.fetch().await.expect("fetched"));

        assert_eq!(checkpoints.status().await.expect("status").behind, 1);
        assert_eq!(read(repo, "a.txt"), "old\n", "the files stay");
        assert_eq!(repo.git(&["tag", "--list"]), "", "no tags arrive");
        assert!(!repo.path().join(".git/FETCH_HEAD").exists());
        assert!(!repo.path().join(".git/objects/info/commit-graph").exists());
        assert!(!repo.path().join(".git/objects/info/commit-graphs").exists());

        repo.git(&["switch", "-q", "-c", "local"]);
        assert!(
            !checkpoints.fetch().await.expect("nothing to fetch"),
            "a branch without upstream fetches nothing"
        );
    });
}

#[test]
fn automatic_fetch_never_imports_configured_bundles() {
    run(async {
        for symbolic in [true, false] {
            let clone = Clone::new(&format!("fetch-bundle-{symbolic}"));
            let repo = &clone.repo;
            clone.commit_upstream("theirs\n");
            let bundle = clone.origin.path().join(".git/fetch.bundle");
            clone.origin.git(&[
                "bundle",
                "create",
                bundle.to_str().expect("bundle path"),
                "main",
            ]);
            repo.git(&[
                "config",
                "fetch.bundleURI",
                bundle.to_str().expect("bundle path"),
            ]);
            if symbolic {
                repo.git(&["symbolic-ref", "refs/bundles/heads/main", "refs/heads/main"]);
            }
            let head = repo.git(&["rev-parse", "HEAD"]);
            let bundles = repo.git(&["for-each-ref", "refs/bundles"]);
            let checkpoints = Checkpoints::new(repo.path());

            assert!(checkpoints.fetch().await.expect("remote fetched"));

            assert_eq!(repo.git(&["rev-parse", "HEAD"]), head, "{symbolic}");
            assert_eq!(repo.git(&["for-each-ref", "refs/bundles"]), bundles);
            assert_eq!(read(repo, "a.txt"), "old\n", "the files stay");
            assert_eq!(checkpoints.status().await.expect("status").behind, 1);
        }
    });
}

#[test]
fn automatic_fetch_never_expands_a_remote_group_into_unchecked_destinations() {
    run(async {
        for (case, members) in [
            ("list", vec!["backup vendor"]),
            ("values", vec!["backup", "vendor"]),
            ("duplicate", vec!["backup backup"]),
        ] {
            let clone = Clone::new(&format!("fetch-group-{case}"));
            let repo = &clone.repo;
            repo.git(&["branch", "release"]);
            let before = repo.git(&["show-ref"]);
            clone.commit_upstream("theirs\n");
            for remote in ["backup", "vendor"] {
                repo.git(&[
                    "remote",
                    "add",
                    remote,
                    clone.origin.path().to_str().expect("origin path"),
                ]);
            }
            repo.git(&[
                "config",
                "remote.backup.fetch",
                "+refs/heads/main:refs/heads/release",
            ]);
            for member in members {
                repo.git(&["config", "--add", "remotes.origin", member]);
            }
            let checkpoints = Checkpoints::new(repo.path());

            let fetched = checkpoints.fetch().await;

            assert_eq!(repo.git(&["show-ref"]), before, "{case}: {fetched:?}");
            assert!(matches!(fetched, Err(DiffError::Git(_))), "{fetched:?}");
            // Git uses the named remote for a one-member group, without expanding it.
            repo.git(&["config", "--replace-all", "remotes.origin", "backup"]);
            assert!(checkpoints.fetch().await.expect("only origin fetched"));
            assert_eq!(
                repo.git(&["rev-parse", "release"]),
                repo.git(&["rev-parse", "HEAD"])
            );
            assert_eq!(checkpoints.status().await.expect("status").behind, 1);
            // Group names use Git's normalized variable spelling, unlike remote subsections.
            repo.git(&["remote", "rename", "origin", "temporary"]);
            repo.git(&["remote", "rename", "temporary", "Origin"]);
            repo.git(&["config", "--replace-all", "remotes.origin", "backup vendor"]);
            assert!(
                checkpoints
                    .fetch()
                    .await
                    .expect("Origin is not the group origin")
            );
            assert_eq!(
                repo.git(&["rev-parse", "release"]),
                repo.git(&["rev-parse", "HEAD"])
            );
        }
    });
}

#[test]
fn automatic_fetch_refuses_legacy_remote_definitions() {
    run(async {
        for (directory, linked) in [("remotes", false), ("branches", false), ("remotes", true)] {
            let clone = Clone::new(&format!("fetch-legacy-{directory}-{linked}"));
            let repo = &clone.repo;
            repo.git(&["branch", "release"]);
            repo.git(&["config", "--unset-all", "remote.origin.url"]);
            let definitions = repo.path().join(".git").join(directory);
            std::fs::create_dir(&definitions).expect("legacy definitions directory");
            let definition = if directory == "remotes" {
                format!(
                    "URL: {}\nPull: +refs/heads/main:refs/remotes/origin/main\nPull: +refs/heads/main:refs/heads/release\n",
                    clone.origin.path().display()
                )
            } else {
                format!("{}#main\n", clone.origin.path().display())
            };
            std::fs::write(definitions.join("origin"), definition).expect("legacy definition");
            let working = if linked {
                let path = repo.path().join("linked");
                repo.git(&[
                    "worktree",
                    "add",
                    "-q",
                    "--track",
                    "-b",
                    "linked",
                    path.to_str().expect("worktree path"),
                    "origin/main",
                ]);
                path
            } else {
                repo.path().to_owned()
            };
            let before = repo.git(&["show-ref"]);
            clone.commit_upstream("theirs\n");
            let checkpoints = Checkpoints::new(&working);

            let fetched = checkpoints.fetch().await;

            assert!(
                matches!(fetched, Err(DiffError::Git(_))),
                "{directory}: {fetched:?}"
            );
            assert_eq!(repo.git(&["show-ref"]), before, "{directory}");
            assert_eq!(read(repo, "a.txt"), "old\n", "{directory}");
        }
    });
}

#[test]
fn automatic_fetch_refuses_incomplete_configuration_reads() {
    use std::io::Write;

    run(async {
        for (case, section, key, prefix) in [
            ("refspecs", "remote \"origin\"", "fetch", "^refs/heads/"),
            ("groups", "remotes", "unrelated", ""),
        ] {
            let clone = Clone::new(&format!("fetch-config-limit-{case}"));
            let repo = &clone.repo;
            let mut config = std::fs::OpenOptions::new()
                .append(true)
                .open(repo.path().join(".git/config"))
                .expect("test configuration opened for append");
            writeln!(
                config,
                "\n[{section}]\n{key} = {prefix}{}",
                "x".repeat(4 * 1024 * 1024)
            )
            .expect("large validation input");
            drop(config);
            let before = repo.git(&["show-ref"]);
            clone.commit_upstream("theirs\n");
            let checkpoints = Checkpoints::new(repo.path());

            let fetched = checkpoints.fetch().await;

            assert!(
                matches!(fetched, Err(DiffError::TooLarge)),
                "{case}: {fetched:?}"
            );
            assert_eq!(repo.git(&["show-ref"]), before, "{case}");
            assert_eq!(read(repo, "a.txt"), "old\n", "{case}");
        }
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
            stash_subjects(&repo),
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

        let subjects = stash_subjects(&repo);
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
            stash_subjects(&repo),
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

        // A commit since moves the branch off the stash's tree, where Git would merge.
        repo.git(&["commit", "-q", "-am", "committed meanwhile"]);
        assert!(matches!(
            checkpoints
                .restore_stash(&stash_of(&checkpoints).await)
                .await,
            Err(DiffError::BranchMoved)
        ));
        assert_eq!(read(&repo, "a.txt"), "edited meanwhile\n");
        assert!(
            checkpoints.stash().await.expect("read").is_some(),
            "the stash stays"
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
fn explicit_writes_finish_hooks_with_oversized_output() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        for exit in [0, 1] {
            let repo = Repo::new(&format!("hook-output-{exit}"));
            repo.git(&["branch", "other"]);
            let path = repo.path().join(".git/hooks/post-checkout");
            std::fs::write(&path, format!(
                "#!/bin/sh\nprintf 'hook diagnostic' >&2\nhead -c 5242880 /dev/zero\ntouch .git/hook-finished\nexit {exit}\n"
            )).expect("hook");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("executable");
            let checkpoints = Checkpoints::new(repo.path());
            let switched = checkpoints
                .switch_branch(&local("other"), Changes::Bring)
                .await;
            assert!(
                repo.path().join(".git/hook-finished").exists(),
                "hook was interrupted: {switched:?}"
            );
            if exit == 0 {
                switched.expect("switched");
            } else {
                assert!(
                    matches!(&switched, Err(DiffError::HookFailed(reason)) if reason.contains("hook diagnostic")),
                    "expected the retained hook diagnostic"
                );
            }
            assert_eq!(repo.git(&["branch", "--show-current"]), "other\n");
            checkpoints
                .capture(&at("after", Phase::Start))
                .await
                .expect("checkout lock released");
        }
    });
}

#[test]
fn a_background_hook_child_cannot_hold_a_completed_writes_checkout_lock() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        let repo = Repo::new("hook-background");
        repo.git(&["branch", "other"]);
        let gate = repo.path().join(".git/hook-gate");
        assert!(
            Command::new("mkfifo")
                .arg(&gate)
                .status()
                .expect("fifo")
                .success()
        );
        let path = repo.path().join(".git/hooks/post-checkout");
        // The background reader inherits both pipes but never completes until released.
        std::fs::write(
            &path,
            "#!/bin/sh\n(read line < .git/hook-gate) &\necho $! > .git/hook-child\nexit 0\n",
        )
        .expect("hook");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("executable");
        let checkpoints = Checkpoints::new(repo.path());
        let switched = tokio::time::timeout(
            Duration::from_secs(3),
            checkpoints.switch_branch(&local("other"), Changes::Bring),
        )
        .await;
        // Always clean up the child, including on a regression failure.
        let pid = std::fs::read_to_string(repo.path().join(".git/hook-child")).expect("child pid");
        let lock = if switched.is_ok() {
            Some(
                tokio::time::timeout(
                    Duration::from_secs(3),
                    Checkpoints::new(repo.path()).capture(&at("after", Phase::Start)),
                )
                .await,
            )
        } else {
            None
        };
        Command::new("kill")
            .arg(pid.trim())
            .status()
            .expect("cleanup");
        switched
            .expect("switch completes while its background child waits")
            .expect("switched");
        lock.expect("lock checked")
            .expect("checkout guard available")
            .expect("guard acquired");
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
            stash_subjects(&repo),
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
fn no_restore_exempts_ignored_paths_after_a_stashed_deletion_is_committed() {
    run(async {
        for ignored in ["p", "p/new.txt"] {
            let repo = Repo::new(&format!(
                "stash-committed-deletion-{}",
                ignored.replace('/', "-")
            ));
            let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
            repo.write("p", "tracked\n");
            repo.git(&["add", "p"]);
            repo.git(&["commit", "-qm", "add tracked parent"]);
            repo.git(&["branch", "other"]);
            repo.git(&["rm", "-q", "p"]);
            std::fs::create_dir(repo.path().join("p")).expect("directory");
            repo.write("p/new.txt", "untracked stash contents\n");
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
            repo.git(&["rm", "-q", "p"]);
            repo.git(&["commit", "-qm", "commit the deletion"]);
            repo.write(".git/info/exclude", "p\n");
            if ignored.contains('/') {
                std::fs::create_dir(repo.path().join("p")).expect("ignored directory");
            }
            repo.write(ignored, "unsaved ignored contents\n");
            let stash = stash_of(&checkpoints).await;
            let index = std::fs::read(repo.path().join(".git/index")).expect("index");

            let restored = checkpoints.restore_stash(&stash).await;

            assert_eq!(read(&repo, "a.txt"), "old\n", "{ignored}: {restored:?}");
            // The committed deletion moved the branch off the stash's tree.
            assert!(
                matches!(restored, Err(DiffError::BranchMoved)),
                "{restored:?}"
            );
            assert_eq!(read(&repo, ignored), "unsaved ignored contents\n");
            assert_eq!(stash_of(&checkpoints).await, stash);
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
        assert!(matches!(restored, Err(DiffError::BranchMoved)));
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
        assert!(
            matches!(checkpoints.stash().await, Err(DiffError::Git(reason)) if reason.contains("UTF-8"))
        );
        assert_eq!(repo.git(&["rev-parse", "refs/stash"]).trim(), stash.trim());
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

                // A deletion committed since moves the branch off the stash's tree.
                assert!(
                    match saved {
                        "deleted-since-stash" => matches!(&restored, Err(DiffError::BranchMoved)),
                        _ => matches!(&restored, Err(DiffError::WouldOverwrite(paths))
                            if paths == &[path]),
                    },
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
fn stashed_changes_filter_whitespace_in_files_and_line_counts() {
    run(async {
        let repo = Repo::new("stash-whitespace");
        let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
        repo.write("space.txt", "spaced\n");
        repo.write("index-only.txt", "base\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-qm", "add preview files"]);
        repo.git(&["branch", "other"]);
        repo.write("a.txt", " old \nsemantic\n");
        repo.write("space.txt", " spaced \n");
        repo.write("index-only.txt", " staged \n");
        repo.git(&["add", "index-only.txt"]);
        repo.write("index-only.txt", "base\n");
        repo.write("new.txt", "new\n");
        let checkpoints = Checkpoints::new(repo.path());
        checkpoints
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
            .await
            .expect("left");
        let stash = repo.git(&["rev-parse", "refs/stash"]).trim().to_owned();

        let files = checkpoints
            .stash_changes(&stash, crate::Whitespace::Hidden)
            .await
            .expect("filtered changes");

        let files: Vec<(&str, u64, u64)> = files
            .iter()
            .map(|file| (file.path.as_str(), file.added, file.removed))
            .collect();
        assert_eq!(
            files,
            [("a.txt", 1, 0), ("index-only.txt", 1, 1), ("new.txt", 1, 0)]
        );
        assert_eq!(
            checkpoints
                .stash_patch(&stash, "space.txt", crate::Whitespace::Hidden)
                .await
                .expect("patch"),
            ""
        );
        checkpoints
            .switch_branch(&local(&main), Changes::Bring)
            .await
            .expect("back");
        let full = checkpoints.stash().await.expect("stash").expect("saved");
        assert_eq!(full.id, stash);
        assert_eq!(
            full.files.len(),
            4,
            "the stash summary counts every saved file"
        );
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
fn automatic_fetch_follows_symbolic_destinations_only_within_remote_refs() {
    run(async {
        for (case, target, indirect, allowed) in [
            ("branch", "refs/heads/release", false, false),
            ("checkout", "refs/heads/main", false, false),
            ("chain", "refs/heads/release", true, false),
            ("remote", "refs/remotes/origin/release", true, true),
        ] {
            let clone = Clone::new(&format!("fetch-symbolic-{case}"));
            let repo = &clone.repo;
            repo.git(&["branch", "release"]);
            repo.git(&["update-ref", "refs/remotes/origin/release", "HEAD"]);
            let destination = "refs/remotes/origin/main";
            if indirect {
                repo.git(&["symbolic-ref", "refs/remotes/origin/alias", target]);
                repo.git(&["symbolic-ref", destination, "refs/remotes/origin/alias"]);
            } else {
                repo.git(&["symbolic-ref", destination, target]);
            }
            repo.git(&[
                "config",
                "remote.origin.fetch",
                "+refs/heads/main:refs/remotes/origin/main",
            ]);
            let before = repo.git(&["show-ref"]);
            let branches = repo.git(&["show-ref", "--heads"]);
            clone.commit_upstream("theirs\n");
            let checkpoints = Checkpoints::new(repo.path());

            let fetched = checkpoints.fetch().await;

            if allowed {
                assert!(fetched.expect("remote target fetched"));
                assert_eq!(
                    repo.git(&["rev-parse", target]),
                    clone.origin.git(&["rev-parse", "HEAD"])
                );
            } else {
                assert!(
                    matches!(fetched, Err(DiffError::FetchOutsideRemotes(_))),
                    "{case}: {fetched:?}"
                );
                assert_eq!(repo.git(&["show-ref"]), before, "{case}");
            }
            assert_eq!(repo.git(&["show-ref", "--heads"]), branches, "{case}");
            assert_eq!(read(repo, "a.txt"), "old\n", "{case}");
        }
    });
}

#[test]
fn automatic_fetch_rejects_non_utf8_refspecs_before_contacting_the_remote() {
    use std::io::Write;

    run(async {
        let clone = Clone::new("fetch-refspec-bytes");
        let repo = &clone.repo;
        let contacted = repo.path().join(".git/remote-contacted");
        repo.git(&[
            "config",
            "remote.origin.uploadpack",
            &format!("touch {}; false", super::shell_word(contacted.display())),
        ]);
        std::fs::OpenOptions::new()
            .append(true)
            .open(repo.path().join(".git/config"))
            .expect("fixture configuration")
            .write_all(
                b"\n[remote \"origin\"]\nfetch = +refs/heads/main:refs/remotes/origin/\xff\n",
            )
            .expect("raw-byte refspec");
        let before = repo.git(&["show-ref"]);
        let checkpoints = Checkpoints::new(repo.path());

        let fetched = checkpoints.fetch().await;

        assert!(
            !contacted.exists(),
            "invalid refspec reached fetch: {fetched:?}"
        );
        assert!(
            matches!(fetched, Err(DiffError::Git(ref reason)) if reason.contains("UTF-8")),
            "{fetched:?}"
        );
        assert_eq!(repo.git(&["show-ref"]), before);
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
            stash_subjects(&repo),
            format!("On {main}: Liquid: changes left on {main}\n")
        );
    });
}

#[test]
fn stashed_changes_restore_only_on_the_tree_they_were_left_on() {
    run(async {
        let repo = Repo::new("stash-moved");
        let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
        repo.git(&["branch", "other"]);
        std::fs::create_dir(repo.path().join("old")).expect("folder");
        repo.write("old/new.txt", "stashed\n");
        repo.git(&["add", "old/new.txt"]);
        let checkpoints = Checkpoints::new(repo.path());
        checkpoints
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
            .await
            .expect("left");
        checkpoints
            .switch_branch(&local(&main), Changes::Bring)
            .await
            .expect("back");
        assert!(
            checkpoints
                .stash()
                .await
                .expect("read")
                .expect("stash")
                .current
        );

        // Git would merge the stash into the moved tree, here through a directory rename onto
        // an ignored file; Liquid restores only without such a merge.
        std::fs::create_dir(repo.path().join("old")).expect("folder");
        repo.write("old/kept.txt", "kept\n");
        repo.git(&["add", "old"]);
        repo.git(&["commit", "-q", "-m", "old"]);
        repo.git(&["mv", "old", "new"]);
        repo.git(&["commit", "-q", "-m", "renamed"]);
        repo.git(&["config", "merge.directoryRenames", "true"]);
        std::fs::write(repo.path().join(".git/info/exclude"), "new/new.txt\n").expect("excluded");
        repo.write("new/new.txt", "ignored\n");
        let saved = stash_of(&checkpoints).await;
        assert!(
            !checkpoints
                .stash()
                .await
                .expect("read")
                .expect("stash")
                .current
        );

        assert!(matches!(
            checkpoints.restore_stash(&saved).await,
            Err(DiffError::BranchMoved)
        ));
        assert_eq!(read(&repo, "new/new.txt"), "ignored\n");
        assert_eq!(stash_of(&checkpoints).await, saved);
    });
}

#[test]
fn leaving_refuses_a_submodule_commit_git_cannot_stash() {
    run(async {
        for state in ["working", "staged", "deleted"] {
            let sub = Repo::new(&format!("stash-submodule-inner-{state}"));
            let repo = Repo::new(&format!("stash-submodule-outer-{state}"));
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
            repo.git(&["branch", "other"]);
            let inner = Repo(repo.path().join("sub"));
            inner.write("a.txt", "moved\n");
            inner.git(&["commit", "-q", "-am", "moved"]);
            if state == "staged" {
                repo.git(&["add", "sub"]);
            } else if state == "deleted" {
                repo.git(&["rm", "--cached", "sub"]);
            }
            let moved = inner.git(&["rev-parse", "HEAD"]);
            let current = repo.git(&["branch", "--show-current"]);
            let index = repo.git(&["ls-files", "--stage"]);
            let checkpoints = Checkpoints::new(repo.path());

            assert!(matches!(
                checkpoints.switch_branch(&local("other"), leave_on(&checkpoints).await).await,
                Err(DiffError::SubmoduleChanged(paths)) if paths == ["sub"]
            ));
            assert_eq!(repo.git(&["branch", "--show-current"]), current);
            assert_eq!(inner.git(&["rev-parse", "HEAD"]), moved);
            assert_eq!(repo.git(&["ls-files", "--stage"]), index);
            assert_eq!(repo.git(&["stash", "list"]), "");
            // The inner repository belongs to the outer one's temporary folder.
            std::mem::forget(inner);
        }
    });
}

/// A stash of `text` in `a.txt` that is in no list, as another Git stores it.
fn external_stash(repo: &Repo, text: &str) -> String {
    repo.write("a.txt", text);
    let id = repo.git(&["stash", "create"]).trim().to_owned();
    repo.git(&["checkout", "--", "a.txt"]);
    id
}

#[test]
fn a_stash_another_git_stores_meanwhile_survives_a_discard_and_a_restore() {
    run(async {
        let repo = Repo::new("stash-external");
        let main = repo.git(&["branch", "--show-current"]).trim().to_owned();
        repo.git(&["branch", "other"]);
        let checkpoints = Checkpoints::new(repo.path());
        let leave = |text: &'static str| {
            let repo = &repo;
            let checkpoints = &checkpoints;
            let main = main.clone();
            async move {
                repo.write("a.txt", text);
                checkpoints
                    .switch_branch(&local("other"), leave_on(checkpoints).await)
                    .await
                    .expect("left");
                checkpoints
                    .switch_branch(&local(&main), Changes::Bring)
                    .await
                    .expect("back");
                stash_of(checkpoints).await
            }
        };

        for restoring in [false, true] {
            let mine = leave("mine\n").await;
            let theirs = external_stash(&repo, "theirs\n");
            let root = repo.path().to_owned();
            let stored = theirs.clone();
            // Between finding its place and dropping it, another Git stores a stash on top.
            crate::diff::before_drop(repo.path(), move || {
                let output = Command::new("git")
                    .args(["stash", "store", "-m", "theirs", &stored])
                    .current_dir(&root)
                    .env("GIT_CONFIG_GLOBAL", "/dev/null")
                    .output()
                    .expect("git runs");
                assert!(output.status.success(), "stored");
            });

            if restoring {
                checkpoints.restore_stash(&mine).await.expect("restored");
                assert_eq!(read(&repo, "a.txt"), "mine\n");
                repo.git(&["checkout", "--", "a.txt"]);
            } else {
                checkpoints.discard_stash(&mine).await.expect("discarded");
                assert_eq!(read(&repo, "a.txt"), "old\n");
            }

            let listed = repo.git(&["stash", "list", "--format=%H"]);
            assert_eq!(
                listed,
                format!("{theirs}\n"),
                "only the confirmed stash went"
            );
            repo.git(&["stash", "drop", "-q"]);
        }
    });
}

#[test]
fn a_concurrent_store_during_save_preserves_the_own_stash_and_index() {
    run(async {
        for refused in [false, true] {
            for replace in [false, true] {
                let repo = Repo::new(&format!("stash-save-identity-{refused}-{replace}"));
                repo.git(&["branch", "-M", "main"]);
                repo.git(&["branch", "other"]);
                let checkpoints = Checkpoints::new(repo.path());
                let earlier = if replace {
                    repo.write("a.txt", "earlier\n");
                    repo.git(&["stash", "push", "-qm", "Liquid: changes left on main"]);
                    Some(repo.git(&["rev-parse", "refs/stash"]).trim().to_owned())
                } else {
                    None
                };
                let theirs = external_stash(&repo, "theirs\n");
                repo.write("a.txt", "indexed\n");
                repo.git(&["add", "a.txt"]);
                repo.write("a.txt", "working\n");
                repo.write("new.txt", "untracked\n");
                let index = repo.git(&["write-tree"]);
                let status = repo.git(&["status", "--porcelain"]);
                let root = repo.path().to_owned();
                let stored = theirs.clone();
                crate::diff::before_stash_identity(repo.path(), move || {
                    let repo = std::mem::ManuallyDrop::new(Repo(root));
                    repo.git(&["stash", "store", "-m", "theirs", &stored]);
                });

                let switched = checkpoints
                    .switch_branch(
                        &local(if refused { "missing" } else { "other" }),
                        leave_on(&checkpoints).await,
                    )
                    .await;

                let mut expected = vec![theirs];
                if refused {
                    assert!(matches!(switched, Err(DiffError::Git(_))), "{switched:?}");
                    assert_eq!(repo.git(&["branch", "--show-current"]), "main\n");
                    if let Some(earlier) = earlier {
                        expected.push(earlier);
                    }
                } else {
                    switched.expect("switched");
                    assert_eq!(repo.git(&["branch", "--show-current"]), "other\n");
                    assert_eq!(read(&repo, "a.txt"), "old\n");
                    assert!(!repo.path().join("new.txt").exists());
                    checkpoints
                        .switch_branch(&local("main"), Changes::Bring)
                        .await
                        .expect("back");
                    let mine = stash_of(&checkpoints).await;
                    assert_ne!(mine, expected[0], "the concurrent save is never ours");
                    assert_eq!(
                        repo.git(&["stash", "list", "--format=%H"]).lines().count(),
                        2
                    );
                    checkpoints
                        .restore_stash(&mine)
                        .await
                        .expect("own save restored");
                }
                assert_eq!(read(&repo, "a.txt"), "working\n");
                assert_eq!(read(&repo, "new.txt"), "untracked\n");
                assert_eq!(repo.git(&["write-tree"]), index);
                assert_eq!(repo.git(&["status", "--porcelain"]), status);
                assert_eq!(
                    repo.git(&["stash", "list", "--format=%H"])
                        .lines()
                        .collect::<Vec<_>>(),
                    expected
                );
            }
        }
    });
}

#[test]
fn an_unidentified_save_preserves_stashes_and_reports_recovery() {
    run(async {
        let repo = Repo::new("stash-unidentified-save");
        repo.git(&["branch", "-M", "main"]);
        repo.git(&["branch", "other"]);
        repo.write("a.txt", "earlier\n");
        repo.git(&["stash", "push", "-qm", "Liquid: changes left on main"]);
        let earlier = repo.git(&["rev-parse", "refs/stash"]).trim().to_owned();
        let theirs = external_stash(&repo, "theirs\n");
        repo.write("a.txt", "indexed\n");
        repo.git(&["add", "a.txt"]);
        repo.write("a.txt", "working\n");
        let index = repo.git(&["write-tree"]);
        let checkpoints = Checkpoints::new(repo.path());
        let root = repo.path().to_owned();
        let stored = theirs.clone();
        crate::diff::before_stash_identity(repo.path(), move || {
            let repo = std::mem::ManuallyDrop::new(Repo(root));
            let mine = repo.git(&["rev-parse", "refs/stash"]);
            repo.git(&["stash", "drop", "-q"]);
            repo.git(&["stash", "store", "-m", "renamed externally", mine.trim()]);
            repo.git(&["stash", "store", "-m", "theirs", &stored]);
        });

        let switched = checkpoints
            .switch_branch(&local("missing"), leave_on(&checkpoints).await)
            .await;

        assert!(
            matches!(switched, Err(DiffError::Git(ref reason)) if reason.contains("identify") && reason.contains("Git")),
            "{switched:?}"
        );
        assert_eq!(repo.git(&["branch", "--show-current"]), "main\n");
        assert_eq!(read(&repo, "a.txt"), "old\n");
        let listed = repo.git(&["stash", "list", "--format=%H"]);
        let ids: Vec<_> = listed.lines().collect();
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0], theirs);
        assert_eq!(ids[2], earlier);
        repo.git(&["stash", "apply", "--quiet", "--index", ids[1]]);
        assert_eq!(read(&repo, "a.txt"), "working\n");
        assert_eq!(repo.git(&["write-tree"]), index);
    });
}

#[test]
fn legacy_branch_stashes_remain_listed_restorable_and_discardable() {
    run(async {
        for restore in [false, true] {
            let repo = Repo::new(&format!("stash-legacy-{restore}"));
            repo.git(&["branch", "-M", "main"]);
            repo.write("a.txt", "indexed\n");
            repo.git(&["add", "a.txt"]);
            repo.write("a.txt", "working\n");
            let index = repo.git(&["write-tree"]);
            repo.git(&["stash", "push", "-qm", "Liquid: changes left on main"]);
            let checkpoints = Checkpoints::new(repo.path());
            let mine = stash_of(&checkpoints).await;
            assert!(
                checkpoints
                    .branches()
                    .await
                    .expect("branches")
                    .branches
                    .iter()
                    .any(|branch| branch.name == "main" && branch.stashed)
            );
            if restore {
                checkpoints
                    .restore_stash(&mine)
                    .await
                    .expect("legacy restored");
                assert_eq!(read(&repo, "a.txt"), "working\n");
                assert_eq!(repo.git(&["write-tree"]), index);
            } else {
                checkpoints
                    .discard_stash(&mine)
                    .await
                    .expect("legacy discarded");
                assert_eq!(read(&repo, "a.txt"), "old\n");
            }
            assert!(repo.git(&["stash", "list"]).is_empty());
        }
    });
}

#[test]
fn non_utf8_refs_never_collide_with_replacement_character_branches() {
    run(async {
        let repo = Repo::new("branch-encoding-identity");
        repo.git(&["branch", "-M", "main"]);
        let id = repo.git(&["rev-parse", "HEAD"]);
        let mut packed = Vec::new();
        for prefix in ["refs/heads/", "refs/remotes/origin/"] {
            for suffix in [b"\xff".as_slice(), "\u{fffd}".as_bytes()] {
                packed.extend_from_slice(format!("{} {prefix}bad-", id.trim()).as_bytes());
                packed.extend_from_slice(suffix);
                packed.push(b'\n');
            }
        }
        std::fs::write(repo.path().join(".git/packed-refs"), packed).expect("byte-distinct refs");
        let checkpoints = Checkpoints::new(repo.path());

        let listed = checkpoints
            .branches()
            .await
            .expect("representable branches still usable");

        let names: Vec<_> = listed
            .branches
            .iter()
            .map(|branch| (branch.name.as_str(), branch.remote))
            .collect();
        assert_eq!(
            names,
            [
                ("bad-\u{fffd}", false),
                ("main", false),
                ("origin/bad-\u{fffd}", true)
            ]
        );
        checkpoints
            .switch_branch(&local("bad-\u{fffd}"), Changes::Bring)
            .await
            .expect("exact branch selected");
        assert_eq!(
            repo.git(&["symbolic-ref", "HEAD"]),
            "refs/heads/bad-\u{fffd}\n"
        );

        std::fs::write(repo.path().join(".git/HEAD"), b"ref: refs/heads/bad-\xff\n")
            .expect("non-UTF-8 current ref");
        let tree = crate::diff::WorkingTree::open(repo.path())
            .await
            .expect("tree");
        for read in [
            tree.branch().await.map(|_| ()),
            checkpoints.status().await.map(|_| ()),
        ] {
            assert!(
                matches!(read, Err(DiffError::Git(ref reason)) if reason.contains("UTF-8")),
                "{read:?}"
            );
        }
        assert_eq!(
            std::fs::read(repo.path().join(".git/HEAD")).expect("HEAD bytes"),
            b"ref: refs/heads/bad-\xff\n"
        );
    });
}

#[test]
fn leaving_changes_never_resets_a_directory_holding_a_repository() {
    run(async {
        let repo = Repo::new("stash-repository-below-reset");
        repo.git(&["branch", "-M", "main"]);
        repo.write("slot", "tracked\n");
        repo.git(&["add", "slot"]);
        repo.git(&["commit", "-qm", "track slot"]);
        repo.git(&["branch", "other"]);
        std::fs::remove_file(repo.path().join("slot")).expect("remove file");
        let nested = Repo::at(repo.path().join("slot/nested"));
        nested.write("a.txt", "nested edits\n");
        let nested_head = nested.git(&["rev-parse", "HEAD"]);
        repo.write("a.txt", "parent edits\n");
        let before = repo.git(&["status", "--porcelain"]);
        let index = repo.git(&["write-tree"]);
        // A branch operation affects the whole checkout, even from a working subfolder.
        std::fs::create_dir(repo.path().join("work")).expect("working folder");
        let checkpoints = Checkpoints::new(&repo.path().join("work"));

        let switched = checkpoints
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
            .await;

        assert!(
            matches!(switched, Err(DiffError::WouldOverwrite(ref paths)) if paths.iter().any(|path| path == "slot")),
            "{switched:?}"
        );
        assert_eq!(repo.git(&["branch", "--show-current"]), "main\n");
        assert_eq!(read(&nested, "a.txt"), "nested edits\n");
        assert_eq!(nested.git(&["rev-parse", "HEAD"]), nested_head);
        nested.git(&["cat-file", "-e", nested_head.trim()]);
        assert_eq!(read(&repo, "a.txt"), "parent edits\n");
        assert_eq!(repo.git(&["status", "--porcelain"]), before);
        assert_eq!(repo.git(&["write-tree"]), index);
        assert!(repo.git(&["stash", "list"]).is_empty());
    });
}

#[test]
fn stash_identity_and_drop_repair_use_utf8_despite_log_settings() {
    run(async {
        let repo = Repo::new("stash-log-encoding");
        repo.git(&["branch", "-M", "main"]);
        repo.git(&["branch", "other"]);
        repo.git(&["config", "i18n.logOutputEncoding", "ISO-8859-1"]);
        repo.write("a.txt", "indexed\n");
        repo.git(&["add", "a.txt"]);
        let index = repo.git(&["write-tree"]);
        repo.write("a.txt", "working\n");
        repo.write("new.txt", "untracked\n");
        let checkpoints = Checkpoints::new(repo.path());
        checkpoints
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
            .await
            .expect("saved and switched");
        checkpoints
            .switch_branch(&local("main"), Changes::Bring)
            .await
            .expect("back");
        let mine = stash_of(&checkpoints).await;
        repo.write("a.txt", "theirs\n");
        let theirs = repo
            .git(&["stash", "create", "théirs · note"])
            .trim()
            .to_owned();
        repo.git(&["checkout", "--", "a.txt"]);
        let subject = repo.git(&["log", "-1", "--encoding=UTF-8", "--format=%s", &theirs]);
        let root = repo.path().to_owned();
        let stored = theirs.clone();
        let label = subject.clone();
        crate::diff::before_drop(repo.path(), move || {
            let external = Repo(root);
            external.git(&["stash", "store", "-m", label.trim_end(), &stored]);
            std::mem::forget(external);
        });

        checkpoints.restore_stash(&mine).await.expect("restored");

        assert_eq!(read(&repo, "a.txt"), "working\n");
        assert_eq!(read(&repo, "new.txt"), "untracked\n");
        assert_eq!(repo.git(&["write-tree"]), index);
        assert_eq!(
            repo.git(&["stash", "list", "--format=%H"]),
            format!("{theirs}\n")
        );
        assert_eq!(
            repo.git(&["stash", "list", "--encoding=UTF-8", "--format=%gs"]),
            subject
        );
    });
}

#[test]
fn an_external_commit_during_a_refused_switch_preserves_both_stashes() {
    run(async {
        let repo = Repo::new("switch-external-advance");
        repo.git(&["branch", "-M", "main"]);
        let other = Repo(repo.path().with_extension("worktree"));
        let _ = std::fs::remove_dir_all(other.path());
        repo.git(&[
            "worktree",
            "add",
            "-q",
            "-b",
            "other",
            other.path().to_str().unwrap(),
        ]);
        repo.write("a.txt", "earlier\n");
        repo.git(&["stash", "push", "-qm", "Liquid: changes left on main"]);
        let earlier = repo.git(&["rev-parse", "refs/stash"]).trim().to_owned();
        let start = repo.git(&["rev-parse", "HEAD"]);
        repo.write("a.txt", "new changes\n");
        let checkpoints = Checkpoints::new(repo.path());
        let root = repo.path().to_owned();
        crate::diff::before_stash_identity(repo.path(), move || {
            let external = Repo(root);
            external.git(&["commit", "--allow-empty", "-qm", "external advance"]);
            std::mem::forget(external);
        });

        let switched = checkpoints
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
            .await;

        assert!(matches!(switched, Err(DiffError::Git(_))), "{switched:?}");
        assert_eq!(repo.git(&["branch", "--show-current"]), "main\n");
        assert_ne!(repo.git(&["rev-parse", "HEAD"]), start);
        let listed = repo.git(&["stash", "list", "--format=%H"]);
        let ids: Vec<&str> = listed.lines().collect();
        assert_eq!(
            ids.len(),
            2,
            "both saves survive uncertain checkout movement"
        );
        assert_eq!(ids[1], earlier);
        assert_eq!(
            repo.git(&["show", &format!("{}:a.txt", ids[0])]),
            "new changes\n"
        );
        assert_eq!(
            repo.git(&["show", &format!("{earlier}:a.txt")]),
            "earlier\n"
        );
    });
}

#[test]
fn restoring_a_stash_never_replaces_a_dirty_submodule() {
    run(async {
        let sub = Repo::new("restore-submodule-inner");
        let repo = Repo::new("restore-submodule-outer");
        repo.git(&["branch", "-M", "main"]);
        repo.git(&[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            &sub.path().display().to_string(),
            "sub",
        ]);
        repo.git(&["commit", "-qm", "submodule"]);
        repo.git(&["branch", "other"]);
        std::fs::remove_dir_all(repo.path().join("sub")).expect("remove checkout");
        repo.write("sub", "saved file instead of submodule\n");
        repo.git(&["add", "sub"]);
        let checkpoints = Checkpoints::new(repo.path());
        checkpoints
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
            .await
            .expect("left");
        checkpoints
            .switch_branch(&local("main"), Changes::Bring)
            .await
            .expect("back");
        repo.git(&[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "update",
            "--init",
            "-q",
        ]);
        let inner = Repo(repo.path().join("sub"));
        inner.write("a.txt", "edited submodule\n");
        inner.write("untracked.txt", "untracked submodule\n");
        let head = inner.git(&["rev-parse", "HEAD"]);
        let index = repo.git(&["write-tree"]);
        let saved = stash_of(&checkpoints).await;
        assert_eq!(checkpoints.status().await.expect("status").changed, 0);

        let restored = checkpoints.restore_stash(&saved).await;

        assert!(
            matches!(restored, Err(DiffError::WouldOverwrite(ref paths)) if paths == &["sub"]),
            "{restored:?}"
        );
        assert_eq!(read(&inner, "a.txt"), "edited submodule\n");
        assert_eq!(read(&inner, "untracked.txt"), "untracked submodule\n");
        assert_eq!(inner.git(&["rev-parse", "HEAD"]), head);
        assert_eq!(repo.git(&["write-tree"]), index);
        assert_eq!(stash_of(&checkpoints).await, saved);
        std::mem::forget(inner);
    });
}

#[test]
fn a_branch_named_detached_keeps_its_identity_and_stash() {
    run(async {
        let repo = Repo::new("branch-detached-name");
        repo.git(&["branch", "-M", "(detached)"]);
        repo.write("a.txt", "stashed\n");
        repo.git(&["stash", "push", "-qm", "Liquid: changes left on (detached)"]);
        let saved = repo.git(&["rev-parse", "refs/stash"]).trim().to_owned();
        let checkpoints = Checkpoints::new(repo.path());

        let (status, stash) = checkpoints.status_with_stash().await.expect("status");

        assert_eq!(status.branch.as_deref(), Some("(detached)"));
        assert_eq!(status.changed, 0);
        let stash = stash.expect("branch stash");
        assert_eq!(stash.id, saved);
        assert!(stash.current);
        repo.git(&["switch", "--detach", "-q", "HEAD"]);
        let (status, stash) = checkpoints
            .status_with_stash()
            .await
            .expect("detached status");
        assert!(status.branch.is_none());
        assert!(stash.is_none());
    });
}

#[test]
fn a_refused_switch_never_restores_its_stash_over_a_dirty_submodule() {
    run(async {
        let sub = Repo::new("rollback-submodule-inner");
        let repo = Repo::new("rollback-submodule-outer");
        repo.git(&["branch", "-M", "main"]);
        repo.git(&[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            &sub.path().display().to_string(),
            "sub",
        ]);
        repo.git(&["commit", "-qm", "submodule"]);
        let head = repo.git(&["rev-parse", "HEAD"]);
        let index = repo.git(&["write-tree"]);
        let other = Repo(repo.path().with_extension("worktree"));
        let _ = std::fs::remove_dir_all(other.path());
        repo.git(&[
            "worktree",
            "add",
            "-q",
            "-b",
            "other",
            other.path().to_str().unwrap(),
        ]);
        std::fs::remove_dir_all(repo.path().join("sub")).expect("remove checkout");
        repo.write("sub", "saved file instead of submodule\n");
        repo.git(&["add", "sub"]);
        let checkpoints = Checkpoints::new(repo.path());
        let root = repo.path().to_owned();
        crate::diff::before_stash_identity(repo.path(), move || {
            let external = Repo(root);
            external.git(&[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "update",
                "--init",
                "-q",
            ]);
            external.write("sub/a.txt", "edited submodule\n");
            external.write("sub/untracked.txt", "untracked submodule\n");
            std::mem::forget(external);
        });

        let switched = checkpoints
            .switch_branch(&local("other"), leave_on(&checkpoints).await)
            .await;

        assert!(
            matches!(switched, Err(DiffError::WouldOverwrite(ref paths)) if paths == &["sub"]),
            "{switched:?}"
        );
        assert_eq!(repo.git(&["branch", "--show-current"]), "main\n");
        assert_eq!(repo.git(&["rev-parse", "HEAD"]), head);
        assert_eq!(repo.git(&["write-tree"]), index);
        assert_eq!(read(&repo, "sub/a.txt"), "edited submodule\n");
        assert_eq!(read(&repo, "sub/untracked.txt"), "untracked submodule\n");
        let saved = stash_of(&checkpoints).await;
        assert_eq!(
            repo.git(&["show", &format!("{saved}:sub")]),
            "saved file instead of submodule\n"
        );
    });
}

#[test]
fn automatic_fetch_never_writes_through_a_linked_remote_ref_directory() {
    use std::os::unix::fs::symlink;

    run(async {
        for (case, directory, target, spec) in [
            (
                "root",
                ".git/refs/remotes",
                "heads",
                "+refs/heads/main:refs/remotes/main",
            ),
            (
                "child",
                ".git/refs/remotes/aliased",
                "../heads",
                "+refs/heads/main:refs/remotes/aliased/main",
            ),
            (
                "wildcard",
                ".git/refs/remotes/aliased",
                "../heads",
                "+refs/heads/*:refs/remotes/aliased/*",
            ),
        ] {
            let clone = Clone::new(&format!("fetch-directory-alias-{case}"));
            let repo = &clone.repo;
            repo.git(&["config", "user.name", "Test"]);
            repo.git(&["config", "user.email", "test@example.com"]);
            repo.write("a.txt", "local commit\n");
            repo.git(&["commit", "-qam", "local"]);
            let head = repo.git(&["rev-parse", "HEAD"]);
            let index = std::fs::read(repo.path().join(".git/index")).expect("index");
            clone.commit_upstream("upstream commit\n");
            repo.git(&["config", "remote.origin.fetch", spec]);
            let directory = repo.path().join(directory);
            if directory.is_dir() {
                std::fs::remove_dir_all(&directory).expect("remove remote refs");
            }
            symlink(target, &directory).expect("link remote ref directory");
            let checkpoints = Checkpoints::new(repo.path());

            let fetched = checkpoints.fetch().await;

            assert!(
                matches!(fetched, Err(DiffError::Git(_))),
                "{case}: {fetched:?}"
            );
            assert_eq!(repo.git(&["rev-parse", "HEAD"]), head, "{case}");
            assert_eq!(repo.git(&["rev-parse", "refs/heads/main"]), head, "{case}");
            assert_eq!(
                std::fs::read(repo.path().join(".git/index")).expect("index"),
                index,
                "{case}"
            );
            assert_eq!(read(repo, "a.txt"), "local commit\n", "{case}");
        }
    });
}
