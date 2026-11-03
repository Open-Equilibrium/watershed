use super::checkpoint::{Repo, at, run};
use crate::{Checkpoints, DiffError, Phase};

struct Remote {
    repo: Repo,
    bare: Repo,
    other: Repo,
}

impl Remote {
    fn new(name: &str) -> Self {
        let repo = Repo::new(name);
        repo.git(&["branch", "-M", "main"]);
        let bare = Repo(repo.path().with_extension("bare"));
        let _ = std::fs::remove_dir_all(bare.path());
        repo.git(&["clone", "--bare", ".", bare.path().to_str().unwrap()]);
        repo.git(&["remote", "add", "origin", bare.path().to_str().unwrap()]);
        repo.git(&["fetch", "origin"]);
        repo.git(&["branch", "--set-upstream-to=origin/main"]);
        repo.git(&["remote", "set-head", "origin", "main"]);
        let other = Repo(repo.path().with_extension("other"));
        let _ = std::fs::remove_dir_all(other.path());
        repo.git(&[
            "clone",
            bare.path().to_str().unwrap(),
            other.path().to_str().unwrap(),
        ]);
        Self { repo, bare, other }
    }

    fn commit(repo: &Repo, path: &str) {
        repo.write(path, "new commit\n");
        repo.git(&["add", path]);
        repo.git(&["commit", "-m", path]);
    }

    fn advance(&self) {
        Self::commit(&self.other, "upstream.txt");
        self.other.git(&["push", "origin", "main"]);
    }
}

#[test]
fn push_confirms_the_default_branch_and_pushes_only_its_commits() {
    run(async {
        let remote = Remote::new("push-default");
        Remote::commit(&remote.repo, "local.txt");
        let checkpoints = Checkpoints::new(remote.repo.path());
        assert!(matches!(
            checkpoints.push(None).await,
            Err(DiffError::PushDefault(_))
        ));
        assert_ne!(
            remote.repo.git(&["rev-parse", "HEAD"]),
            remote.bare.git(&["rev-parse", "main"])
        );
        let DiffError::PushDefault(approval) = checkpoints.push(None).await.unwrap_err() else {
            panic!("confirmation required")
        };
        let pushed = checkpoints.push(Some(approval.confirmation)).await.unwrap();
        assert_eq!(pushed.ahead, 1);
        assert_eq!(pushed.upstream, "origin/main");
        assert!(!pushed.first_push);
        assert_eq!(
            remote.repo.git(&["rev-parse", "HEAD"]),
            remote.bare.git(&["rev-parse", "main"])
        );
    });
}

#[test]
fn first_push_tracks_its_own_name_and_runs_pre_push_during_an_agent_turn() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        let remote = Remote::new("push-first");
        remote.repo.git(&["switch", "-c", "feature/first"]);
        let hook = remote.repo.path().join(".git/hooks/pre-push");
        std::fs::write(&hook, "#!/bin/sh\ntest \"$GIT_TERMINAL_PROMPT\" = 0 || exit 2\nhead -c 5242880 /dev/zero\ntouch .git/pre-push-finished\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        let checkpoints = Checkpoints::new(remote.repo.path());
        checkpoints.capture(&at("t1", Phase::Start)).await.unwrap();
        let pushed = checkpoints.push(None).await.unwrap();
        assert!(pushed.first_push);
        assert_eq!(pushed.ahead, 1);
        assert_eq!(pushed.behind, 0);
        assert_eq!(pushed.upstream, "origin/feature/first");
        assert!(remote.repo.path().join(".git/pre-push-finished").exists());
        assert_eq!(
            remote
                .repo
                .git(&["rev-parse", "--abbrev-ref", "@{upstream}"]),
            "origin/feature/first\n"
        );
        assert_eq!(
            remote.bare.git(&["rev-parse", "feature/first"]),
            remote.repo.git(&["rev-parse", "HEAD"])
        );
    });
}

#[test]
fn rejected_push_distinguishes_behind_and_diverged_without_force() {
    run(async {
        for diverged in [false, true] {
            let remote = Remote::new(if diverged {
                "push-diverged"
            } else {
                "push-behind"
            });
            remote.advance();
            if diverged {
                Remote::commit(&remote.repo, "local.txt");
            }
            let before = remote.bare.git(&["rev-parse", "main"]);
            let error = Checkpoints::new(remote.repo.path())
                .push(None)
                .await
                .unwrap_err();
            assert!(
                matches!(error, DiffError::Diverged(ref transfer, _) if transfer.ahead == u64::from(diverged) && transfer.behind == 1),
                "{error:?}"
            );
            assert_eq!(remote.bare.git(&["rev-parse", "main"]), before);
        }
    });
}

#[test]
fn pull_fast_forwards_and_refuses_divergence_and_overwriting_changes() {
    run(async {
        for state in ["clean", "diverged", "dirty"] {
            let remote = Remote::new(&format!("pull-{state}"));
            remote.advance();
            if state == "diverged" {
                Remote::commit(&remote.repo, "local.txt");
            }
            if state == "dirty" {
                remote.repo.write("upstream.txt", "unsaved\n");
            }
            let checkpoints = Checkpoints::new(remote.repo.path());
            let before = remote.repo.git(&["rev-parse", "HEAD"]);
            let result = checkpoints.pull().await;
            match state {
                "clean" => {
                    assert_eq!(result.unwrap().behind, 1);
                    assert_eq!(
                        remote.repo.git(&["rev-parse", "HEAD"]),
                        remote.bare.git(&["rev-parse", "main"])
                    );
                    assert!(remote.repo.path().join("upstream.txt").exists());
                }
                "diverged" => assert!(
                    matches!(result, Err(DiffError::Diverged(_, _))),
                    "{result:?}"
                ),
                _ => {
                    assert!(
                        matches!(result, Err(DiffError::WouldOverwrite(ref files)) if files == &["upstream.txt"]),
                        "{result:?}"
                    );
                    assert_eq!(
                        std::fs::read_to_string(remote.repo.path().join("upstream.txt")).unwrap(),
                        "unsaved\n"
                    );
                }
            }
            if state != "clean" {
                assert_eq!(remote.repo.git(&["rev-parse", "HEAD"]), before);
            }
        }
    });
}

#[test]
fn pull_refuses_a_running_agent_before_fetching() {
    run(async {
        let remote = Remote::new("pull-agent");
        remote.advance();
        let checkpoints = Checkpoints::new(remote.repo.path());
        checkpoints.capture(&at("t1", Phase::Start)).await.unwrap();
        let before = remote.repo.git(&["rev-parse", "origin/main"]);
        assert!(
            matches!(checkpoints.pull().await, Err(DiffError::AgentRunning(ref running)) if running == &["s1"])
        );
        assert_eq!(remote.repo.git(&["rev-parse", "origin/main"]), before);
        checkpoints.capture(&at("t1", Phase::Start)).await.unwrap();
    });
}

#[test]
fn requested_fetch_uses_origin_without_upstream_and_checks_destinations() {
    run(async {
        let remote = Remote::new("fetch-requested");
        remote.repo.git(&["switch", "-c", "feature/unpublished"]);
        remote.advance();
        let checkpoints = Checkpoints::new(remote.repo.path());
        assert!(checkpoints.fetch().await.unwrap());
        assert_eq!(
            remote.repo.git(&["rev-parse", "origin/main"]),
            remote.bare.git(&["rev-parse", "main"])
        );
        remote.repo.git(&[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/heads/*",
        ]);
        assert!(matches!(
            checkpoints.fetch().await,
            Err(DiffError::FetchOutsideRemotes(_))
        ));
    });
}

#[test]
fn push_checks_the_advertised_default_without_a_cached_remote_head() {
    run(async {
        let remote = Remote::new("push-uncached-default");
        remote
            .repo
            .git(&["symbolic-ref", "--delete", "refs/remotes/origin/HEAD"]);
        remote.repo.git(&["branch", "-m", "local-name"]);
        let result = Checkpoints::new(remote.repo.path()).push(None).await;
        assert!(
            matches!(result, Err(DiffError::PushDefault(ref transfer)) if transfer.confirmation.reference == "refs/heads/main"),
            "{result:?}"
        );
    });
}

#[test]
fn network_failures_redact_url_credentials_and_identify_sign_in() {
    run(async {
        for authentication in [false, true] {
            let remote = Remote::new(if authentication {
                "push-auth"
            } else {
                "push-network"
            });
            let message = if authentication {
                "Authentication failed"
            } else {
                "Could not resolve host"
            };
            remote.repo.git(&["config", "remote.origin.uploadpack", &format!("sh -c 'echo \"{message}: https://example:synthetic@example.invalid/repo\" >&2; exit 1'")]);
            let result = Checkpoints::new(remote.repo.path()).push(None).await;
            let error = result.unwrap_err();
            assert!(!error.to_string().contains("synthetic"), "{error}");
            assert!(error.to_string().contains("example.invalid"));
            assert_eq!(
                matches!(error, DiffError::SignInFailed(_, _)),
                authentication
            );
            assert_eq!(
                matches!(error, DiffError::FetchFailed(_, _)),
                !authentication
            );
            assert_eq!(
                remote.repo.git(&["rev-parse", "HEAD"]),
                remote.bare.git(&["rev-parse", "main"])
            );
        }
    });
}

#[test]
fn first_push_counts_commits_against_its_destination_before_confirmation() {
    run(async {
        let remote = Remote::new("push-first-counts");
        remote.repo.git(&["branch", "--unset-upstream"]);
        Remote::commit(&remote.repo, "local.txt");
        let checkpoints = Checkpoints::new(remote.repo.path());
        let confirmed = checkpoints.push(None).await.unwrap_err();
        assert!(
            matches!(confirmed, DiffError::PushDefault(ref transfer) if transfer.ahead == 1 && transfer.first_push)
        );
        let DiffError::PushDefault(approval) = confirmed else {
            panic!("confirmation required")
        };
        let pushed = checkpoints.push(Some(approval.confirmation)).await.unwrap();
        assert_eq!(pushed.ahead, 1);
        assert!(pushed.first_push);
    });
}

#[test]
fn default_push_confirmation_expires_when_the_commit_or_branch_changes() {
    run(async {
        for change in ["commit", "branch", "remote", "reference"] {
            let remote = Remote::new(&format!("push-confirm-{change}"));
            Remote::commit(&remote.repo, "local.txt");
            let checkpoints = Checkpoints::new(remote.repo.path());
            let DiffError::PushDefault(approval) = checkpoints.push(None).await.unwrap_err() else {
                panic!("confirmation required")
            };
            let before = remote.bare.git(&["rev-parse", "main"]);
            if change == "commit" {
                Remote::commit(&remote.repo, "later.txt");
            } else if change == "remote" {
                remote.repo.git(&["remote", "rename", "origin", "github"]);
            } else if change == "reference" {
                remote
                    .repo
                    .git(&["config", "branch.main.merge", "refs/heads/other"]);
            } else {
                remote.repo.git(&["switch", "-c", "different"]);
                remote
                    .repo
                    .git(&["branch", "--set-upstream-to=origin/main"]);
            }
            let result = checkpoints.push(Some(approval.confirmation)).await;
            assert!(
                matches!(result, Err(DiffError::PushDefault(_))),
                "{change}: {result:?}"
            );
            assert_eq!(remote.bare.git(&["rev-parse", "main"]), before);
        }
    });
}

#[test]
fn push_refuses_short_upstream_refs_and_confirms_the_cached_default() {
    run(async {
        for case in ["short", "cached"] {
            let remote = Remote::new(&format!("push-upstream-{case}"));
            Remote::commit(&remote.repo, "local.txt");
            if case == "short" {
                remote.repo.git(&["config", "branch.main.merge", "main"]);
            } else {
                remote
                    .bare
                    .git(&["symbolic-ref", "HEAD", "refs/heads/other"]);
            }
            let before = remote.bare.git(&["rev-parse", "main"]);
            let result = Checkpoints::new(remote.repo.path()).push(None).await;
            assert!(result.is_err(), "{case}: {result:?}");
            assert_eq!(remote.bare.git(&["rev-parse", "main"]), before);
        }
    });
}

#[test]
fn a_deleted_upstream_can_be_pushed_again_without_counting_full_history() {
    run(async {
        let remote = Remote::new("push-gone");
        remote.repo.git(&["switch", "-c", "feature/gone"]);
        remote.repo.git(&["push", "-u", "origin", "HEAD"]);
        remote.repo.git(&["config", "fetch.prune", "true"]);
        remote
            .bare
            .git(&["update-ref", "-d", "refs/heads/feature/gone"]);
        Remote::commit(&remote.repo, "local.txt");
        let checkpoints = Checkpoints::new(remote.repo.path());
        assert!(!checkpoints.status().await.unwrap().gone);
        let pull = checkpoints.pull().await;
        assert!(matches!(pull, Err(DiffError::Git(_))), "{pull:?}");
        let status = checkpoints.status().await.unwrap();
        assert!(status.upstream.is_some());
        assert!(status.gone);
        let pushed = checkpoints.push(None).await.unwrap();
        assert_eq!(
            pushed.ahead, 0,
            "a gone upstream is not a first publication"
        );
        assert_eq!(
            remote.repo.git(&["rev-parse", "@{upstream}"]),
            remote.repo.git(&["rev-parse", "HEAD"])
        );
    });
}

fn gated_fetch(remote: &Remote) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let entered = remote.repo.path().join(".git/fetch-entered");
    let release = remote.repo.path().join(".git/fetch-release");
    let script = remote.repo.path().join(".git/gated-upload-pack");
    std::fs::write(&script, format!("#!/bin/sh\nmkdir {} || {{ touch {}; exit 1; }}\ntrap 'rmdir {}' EXIT\ntouch {}\nwhile ! test -e {}; do sleep 0.01; done\ngit-upload-pack \"$@\"\n", super::shell_word(remote.repo.path().join(".git/fetch-active").display()), super::shell_word(remote.repo.path().join(".git/fetch-overlap").display()), super::shell_word(remote.repo.path().join(".git/fetch-active").display()), super::shell_word(entered.display()), super::shell_word(release.display()))).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    remote.repo.git(&[
        "config",
        "remote.origin.uploadpack",
        script.to_str().unwrap(),
    ]);
    (entered, release)
}

async fn wait_for(path: &std::path::Path) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !path.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Git reaches the fetch gate");
}

#[test]
fn every_fetch_of_a_working_tree_is_serialized() {
    run(async {
        for action in ["fetch", "push", "pull"] {
            let remote = Remote::new(&format!("fetch-serial-{action}"));
            remote.repo.git(&["switch", "-c", "feature"]);
            remote.repo.git(&["push", "-u", "origin", "HEAD"]);
            let (entered, release) = gated_fetch(&remote);
            let checkpoints = std::sync::Arc::new(Checkpoints::new(remote.repo.path()));
            let fetching = tokio::spawn({
                let checkpoints = checkpoints.clone();
                async move { checkpoints.fetch().await }
            });
            wait_for(&entered).await;
            let second = tokio::spawn({
                let checkpoints = checkpoints.clone();
                async move {
                    match action {
                        "push" => checkpoints.push(None).await.map(|_| ()),
                        "pull" => checkpoints.pull().await.map(|_| ()),
                        _ => checkpoints.fetch().await.map(|_| ()),
                    }
                }
            });
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            std::fs::write(release, "").unwrap();
            let first_result = fetching.await.unwrap();
            let second_result = second.await.unwrap();
            assert!(
                !remote.repo.path().join(".git/fetch-overlap").exists(),
                "{action}: {first_result:?} {second_result:?}"
            );
            first_result.unwrap();
            second_result.unwrap();
        }
    });
}

#[test]
fn a_turn_can_start_during_pull_fetch_and_pull_rechecks_its_lock() {
    run(async {
        let remote = Remote::new("pull-fetch-turn");
        remote.advance();
        let (entered, release) = gated_fetch(&remote);
        let checkpoints = std::sync::Arc::new(Checkpoints::new(remote.repo.path()));
        let pulling = tokio::spawn({
            let checkpoints = checkpoints.clone();
            async move { checkpoints.pull().await }
        });
        wait_for(&entered).await;
        let start = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            checkpoints.capture(&at("t1", Phase::Start)),
        )
        .await;
        std::fs::write(release, "").unwrap();
        let result = pulling.await.unwrap();
        assert!(
            start.is_ok(),
            "a pending network read must not block a turn"
        );
        start.unwrap().unwrap();
        assert!(
            matches!(result, Err(DiffError::AgentRunning(_))),
            "{result:?}"
        );
        assert_ne!(
            remote.repo.git(&["rev-parse", "HEAD"]),
            remote.bare.git(&["rev-parse", "main"])
        );
    });
}

#[test]
fn pull_fetch_is_bounded_and_stops_when_the_session_closes() {
    run(async {
        for closing in [false, true] {
            let remote = Remote::new(&format!("pull-fetch-bounded-{closing}"));
            let (entered, release) = gated_fetch(&remote);
            let checkpoints = std::sync::Arc::new(Checkpoints::with_timeout(
                remote.repo.path(),
                std::time::Duration::from_secs(2),
            ));
            let mut pulling = tokio::spawn({
                let checkpoints = checkpoints.clone();
                async move { checkpoints.pull().await }
            });
            wait_for(&entered).await;
            if closing {
                checkpoints.close().await;
            }
            let result =
                tokio::time::timeout(std::time::Duration::from_secs(4), &mut pulling).await;
            std::fs::write(release, "").unwrap();
            assert!(
                result.is_ok(),
                "network fetch must stop on deadline or close"
            );
            let error = result.unwrap().unwrap().unwrap_err();
            assert!(
                matches!(error, DiffError::TimedOut(_) | DiffError::Closed),
                "{error:?}"
            );
        }
    });
}

#[test]
fn pushes_use_the_only_remote_and_refuse_an_ambiguous_destination() {
    run(async {
        let remote = Remote::new("push-sole-remote");
        remote.repo.git(&["remote", "rename", "origin", "github"]);
        remote.repo.git(&["switch", "-c", "feature/sole"]);
        let checkpoints = Checkpoints::new(remote.repo.path());
        let status = checkpoints.status().await.unwrap();
        assert_eq!(status.remote.as_deref(), Some("github"));
        assert!(status.has_remotes);
        let pushed = checkpoints.push(None).await.unwrap();
        assert_eq!(pushed.remote, "github");
        remote.repo.git(&["switch", "-c", "feature/ambiguous"]);
        remote.repo.git(&[
            "remote",
            "add",
            "backup",
            remote.bare.path().to_str().unwrap(),
        ]);
        let status = checkpoints.status().await.unwrap();
        assert!(status.remote.is_none());
        assert!(status.has_remotes);
        assert!(!checkpoints.fetch().await.unwrap());
        assert!(checkpoints.push(None).await.is_err());
        // A leftover remote setting alone does not configure an upstream.
        remote
            .repo
            .git(&["config", "branch.feature/ambiguous.remote", "github"]);
        assert!(checkpoints.status().await.unwrap().remote.is_none());
        remote.repo.git(&["remote", "remove", "github"]);
        remote.repo.git(&["remote", "remove", "backup"]);
        assert!(!checkpoints.status().await.unwrap().has_remotes);
    });
}

#[test]
fn a_push_hook_cannot_expand_the_confirmed_commit_range() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        let remote = Remote::new("push-exact-commit");
        Remote::commit(&remote.repo, "approved.txt");
        let checkpoints = Checkpoints::new(remote.repo.path());
        let DiffError::PushDefault(approval) = checkpoints.push(None).await.unwrap_err() else {
            panic!("confirmation required")
        };
        let approved = approval.confirmation.commit.clone();
        let hook = remote.repo.path().join(".git/hooks/pre-push");
        std::fs::write(
            &hook,
            "#!/bin/sh\ngit -c core.hooksPath=/dev/null -c user.name=Test -c user.email=test@example.invalid commit --allow-empty -qm later\n",
        )
        .unwrap();
        std::fs::set_permissions(hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        let pushed = checkpoints.push(Some(approval.confirmation)).await.unwrap();
        assert_eq!(pushed.ahead, 1);
        assert_eq!(remote.bare.git(&["rev-parse", "main"]).trim(), approved);
        assert_ne!(remote.repo.git(&["rev-parse", "HEAD"]).trim(), approved);
    });
}
