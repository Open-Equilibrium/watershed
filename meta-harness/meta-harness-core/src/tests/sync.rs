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
            checkpoints.push(false).await,
            Err(DiffError::PushDefault(_, _))
        ));
        assert_ne!(
            remote.repo.git(&["rev-parse", "HEAD"]),
            remote.bare.git(&["rev-parse", "main"])
        );
        let pushed = checkpoints.push(true).await.unwrap();
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
        let pushed = checkpoints.push(false).await.unwrap();
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
                .push(true)
                .await
                .unwrap_err();
            assert!(
                matches!(error, DiffError::PushRejected(ref transfer, _) if transfer.ahead == u64::from(diverged) && transfer.behind == 1),
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
                    matches!(result, Err(DiffError::PushRejected(_, _))),
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
        assert!(
            !checkpoints.fetch().await.unwrap(),
            "background fetch needs an upstream"
        );
        assert!(checkpoints.fetch_now().await.unwrap());
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
            checkpoints.fetch_now().await,
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
        let result = Checkpoints::new(remote.repo.path()).push(false).await;
        assert!(
            matches!(result, Err(DiffError::PushDefault(ref branch, _)) if branch == "main"),
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
            let result = Checkpoints::new(remote.repo.path()).push(true).await;
            let error = result.unwrap_err();
            assert!(!error.to_string().contains("synthetic"), "{error}");
            assert!(error.to_string().contains("example.invalid"));
            assert_eq!(
                matches!(error, DiffError::SignInFailed(_, _)),
                authentication
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
        let confirmed = checkpoints.push(false).await.unwrap_err();
        assert!(
            matches!(confirmed, DiffError::PushDefault(_, ref transfer) if transfer.ahead == 1 && transfer.first_push)
        );
        let pushed = checkpoints.push(true).await.unwrap();
        assert_eq!(pushed.ahead, 1);
        assert!(pushed.first_push);
    });
}
