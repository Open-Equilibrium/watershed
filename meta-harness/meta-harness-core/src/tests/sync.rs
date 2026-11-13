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
        for state in ["clean", "diverged", "dirty", "ignored"] {
            let remote = Remote::new(&format!("pull-{state}"));
            remote.advance();
            if state == "diverged" {
                Remote::commit(&remote.repo, "local.txt");
            }
            if state == "dirty" || state == "ignored" {
                remote.repo.write("upstream.txt", "unsaved\n");
            }
            if state == "ignored" {
                remote.repo.write(".git/info/exclude", "upstream.txt\n");
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
                        "{state}: {result:?}"
                    );
                    assert_eq!(
                        std::fs::read_to_string(remote.repo.path().join("upstream.txt")).unwrap(),
                        "unsaved\n",
                        "{state}"
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
fn pull_ignores_an_ignored_file_the_branch_stopped_tracking_ahead_of_its_upstream() {
    run(async {
        let remote = Remote::new("pull-ahead-untracked");
        Remote::commit(&remote.repo, ".env");
        remote.repo.git(&["push", "origin", "main"]);
        remote.repo.git(&["rm", "-q", "--cached", ".env"]);
        remote.repo.git(&["commit", "-qm", "stop tracking .env"]);
        remote.repo.write(".git/info/exclude", ".env\n");
        let before = remote.repo.git(&["rev-parse", "HEAD"]);

        let pulled = Checkpoints::new(remote.repo.path()).pull().await;

        assert!(
            matches!(pulled, Ok(ref transfer) if transfer.behind == 0),
            "{pulled:?}"
        );
        assert_eq!(remote.repo.git(&["rev-parse", "HEAD"]), before);
    });
}

#[test]
fn pull_refuses_an_upstream_omitted_by_fetch_before_merging() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        let remote = Remote::new("pull-excluded-upstream");
        remote.advance();
        remote.repo.git(&["fetch", "origin"]);
        Remote::commit(&remote.other, "latest.txt");
        remote.other.git(&["push", "origin", "main"]);
        remote
            .repo
            .git(&["config", "--add", "remote.origin.fetch", "^refs/heads/main"]);
        let hook = remote.repo.path().join(".git/hooks/post-merge");
        std::fs::write(&hook, "#!/bin/sh\ntouch .git/merge-finished\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        let before = remote.repo.git(&["rev-parse", "HEAD"]);
        let result = Checkpoints::new(remote.repo.path()).pull().await;
        assert!(matches!(result, Err(DiffError::Git(_))), "{result:?}");
        assert_eq!(remote.repo.git(&["rev-parse", "HEAD"]), before);
        assert!(!remote.repo.path().join(".git/merge-finished").exists());
        assert!(remote.repo.git(&["status", "--porcelain"]).is_empty());
    });
}

#[test]
fn pull_advances_head_despite_configured_squash_mode() {
    run(async {
        let remote = Remote::new("pull-configured-squash");
        remote
            .repo
            .git(&["config", "branch.main.mergeOptions", "--squash"]);
        remote.advance();
        let pulled = Checkpoints::new(remote.repo.path()).pull().await.unwrap();
        assert_eq!(pulled.behind, 1);
        assert_eq!(
            remote.repo.git(&["rev-parse", "HEAD"]),
            remote.bare.git(&["rev-parse", "main"])
        );
        assert!(remote.repo.git(&["status", "--porcelain"]).is_empty());
    });
}

#[test]
fn pull_reports_autostash_conflicts_after_advancing_the_branch() {
    run(async {
        for (setting, value) in [
            ("merge.autoStash", "true"),
            ("branch.main.mergeOptions", "--autostash"),
        ] {
            let remote = Remote::new(&format!("pull-autostash-{setting}"));
            remote.repo.git(&["config", setting, value]);
            remote.other.write("a.txt", "remote change\n");
            remote.other.git(&["commit", "-am", "Remote change"]);
            remote.other.git(&["push", "origin", "main"]);
            remote.repo.write("a.txt", "unsaved local change\n");
            let result = Checkpoints::new(remote.repo.path()).pull().await;
            assert!(
                matches!(result, Err(DiffError::TransferFailed(ref detail))
                    if detail.contains("conflicts") && detail.contains("Resolve") && detail.contains("stash")),
                "{setting}: {result:?}"
            );
            assert_eq!(
                remote.repo.git(&["rev-parse", "HEAD"]),
                remote.bare.git(&["rev-parse", "main"])
            );
            assert_eq!(
                remote.repo.git(&["diff", "--name-only", "--diff-filter=U"]),
                "a.txt\n"
            );
            assert_eq!(
                remote.repo.git(&["show", "stash@{0}:a.txt"]),
                "unsaved local change\n"
            );
        }
    });
}

#[test]
fn an_autostash_pull_keeps_ignored_files_where_a_tracked_file_returns() {
    run(async {
        // Git autostashes for `merge.autoStash` and for any spelling of `--autostash` in the
        // branch's merge options, such as an unambiguous abbreviation or a quoted word.
        for (index, (setting, value)) in [
            ("merge.autoStash", "true"),
            ("branch.main.mergeOptions", "--autostas"),
            ("branch.main.mergeOptions", "'--autostash'"),
        ]
        .into_iter()
        .enumerate()
        {
            let remote = Remote::new(&format!("pull-autostash-ignored-{index}"));
            remote.repo.write(".gitignore", "slot/\n");
            remote.repo.write("slot", "tracked\n");
            remote.repo.git(&["add", "."]);
            remote.repo.git(&["commit", "-qm", "track slot"]);
            remote.repo.git(&["push", "-q", "origin", "main"]);
            remote.other.git(&["pull", "-q", "origin", "main"]);
            remote.advance();
            remote.repo.git(&["config", setting, value]);
            // Locally, the tracked file is replaced by a folder whose ignored contents autostash
            // would reset away when the file returns.
            std::fs::remove_file(remote.repo.path().join("slot")).unwrap();
            std::fs::create_dir(remote.repo.path().join("slot")).unwrap();
            remote.repo.write("slot/keep", "ignored\n");
            let result = Checkpoints::new(remote.repo.path()).pull().await;
            assert!(
                matches!(result, Err(DiffError::WouldOverwrite(ref files)) if files == &["slot/"]),
                "{setting} {value}: {result:?}"
            );
            assert_eq!(
                std::fs::read_to_string(remote.repo.path().join("slot/keep")).unwrap(),
                "ignored\n",
                "{setting} {value}"
            );
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
        assert!(checkpoints.fetch().await.unwrap().is_some());
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
        for change in ["commit", "branch", "remote", "reference", "repository"] {
            let remote = Remote::new(&format!("push-confirm-{change}"));
            let elsewhere = Repo(remote.repo.path().with_extension("elsewhere"));
            let _ = std::fs::remove_dir_all(elsewhere.path());
            remote.repo.git(&[
                "clone",
                "--bare",
                remote.bare.path().to_str().unwrap(),
                elsewhere.path().to_str().unwrap(),
            ]);
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
            } else if change == "repository" {
                remote.repo.git(&[
                    "remote",
                    "set-url",
                    "origin",
                    elsewhere.path().to_str().unwrap(),
                ]);
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
            assert_eq!(elsewhere.git(&["rev-parse", "main"]), before);
        }
    });
}

#[test]
fn push_confirms_a_destination_that_names_the_default_branch_symbolically() {
    run(async {
        let remote = Remote::new("push-symbolic-default");
        remote
            .bare
            .git(&["symbolic-ref", "refs/heads/alias", "refs/heads/main"]);
        remote.repo.git(&["switch", "-c", "feature"]);
        remote
            .repo
            .git(&["config", "branch.feature.remote", "origin"]);
        remote
            .repo
            .git(&["config", "branch.feature.merge", "refs/heads/alias"]);
        Remote::commit(&remote.repo, "local.txt");
        let before = remote.bare.git(&["rev-parse", "main"]);

        let result = Checkpoints::new(remote.repo.path()).push(None).await;

        assert!(
            matches!(result, Err(DiffError::PushDefault(_))),
            "{result:?}"
        );
        assert_eq!(remote.bare.git(&["rev-parse", "main"]), before);
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
        // Closing must end the fetch alone, so its own deadline outlasts the wait below.
        for (closing, deadline) in [(false, 2), (true, 60)] {
            let remote = Remote::new(&format!("pull-fetch-bounded-{closing}"));
            let (entered, release) = gated_fetch(&remote);
            let checkpoints = std::sync::Arc::new(Checkpoints::with_timeout(
                remote.repo.path(),
                std::time::Duration::from_secs(deadline),
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
            let expected = if closing {
                matches!(error, DiffError::Closed)
            } else {
                matches!(error, DiffError::TimedOut(_))
            };
            assert!(expected, "{error:?}");
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
        assert!(checkpoints.fetch().await.unwrap().is_none());
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

#[test]
fn push_refuses_other_or_multiple_repositories_before_hooks() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        for case in [
            "pushurl",
            "pushurls",
            "urls",
            "push-rewrite",
            "rewrite",
            "branch-default",
            "remote-default",
            "ssh-users",
        ] {
            let remote = Remote::new(&format!("push-endpoints-{case}"));
            let other = Remote::new(&format!("push-endpoints-other-{case}"));
            other.bare.git(&["branch", "release", "main"]);
            other
                .bare
                .git(&["symbolic-ref", "HEAD", "refs/heads/release"]);
            remote.repo.git(&["switch", "-c", "release"]);
            remote
                .repo
                .git(&["branch", "--set-upstream-to=origin/main"]);
            remote
                .repo
                .git(&["config", "branch.release.merge", "refs/heads/release"]);
            Remote::commit(&remote.repo, "local.txt");
            let fetch = remote.bare.path().to_str().unwrap();
            let push = other.bare.path().to_str().unwrap();
            match case {
                "pushurl" => {
                    remote.repo.git(&["config", "remote.origin.pushurl", push]);
                }
                "pushurls" => {
                    remote
                        .repo
                        .git(&["config", "--add", "remote.origin.pushurl", fetch]);
                    remote
                        .repo
                        .git(&["config", "--add", "remote.origin.pushurl", push]);
                }
                "urls" => {
                    remote
                        .repo
                        .git(&["config", "--add", "remote.origin.url", push]);
                }
                "push-rewrite" => {
                    remote
                        .repo
                        .git(&["config", &format!("url.{push}.pushInsteadOf"), fetch]);
                }
                "rewrite" => {
                    remote
                        .repo
                        .git(&["config", "remote.origin.pushurl", "publication:repo"]);
                    remote.repo.git(&[
                        "config",
                        &format!("url.{push}.insteadOf"),
                        "publication:repo",
                    ]);
                }
                "ssh-users" => {
                    // The same host and path, but another account's repository access.
                    remote.repo.git(&["config", "core.sshCommand", "false"]);
                    for (key, user) in [("url", "alice"), ("pushurl", "bob")] {
                        remote.repo.git(&[
                            "config",
                            &format!("remote.origin.{key}"),
                            &format!("ssh://{user}@example.invalid/repo.git"),
                        ]);
                    }
                }
                _ => {
                    remote.repo.git(&["remote", "add", "publication", push]);
                    let key = if case == "branch-default" {
                        "branch.release.pushRemote"
                    } else {
                        "remote.pushDefault"
                    };
                    remote.repo.git(&["config", key, "publication"]);
                }
            }
            let hook = remote.repo.path().join(".git/hooks/pre-push");
            std::fs::write(&hook, "#!/bin/sh\ntouch .git/push-hook-ran\n").unwrap();
            std::fs::set_permissions(hook, std::fs::Permissions::from_mode(0o755)).unwrap();
            let before_fetch = remote.bare.git(&["show-ref"]);
            let before_push = other.bare.git(&["show-ref"]);
            let result = Checkpoints::new(remote.repo.path()).push(None).await;
            assert_eq!(
                result.unwrap_err().to_string(),
                "This remote pushes to other or several repositories. Push it with Git.",
                "{case}"
            );
            assert!(
                !remote.repo.path().join(".git/push-hook-ran").exists(),
                "{case}"
            );
            assert_eq!(remote.bare.git(&["show-ref"]), before_fetch, "{case}");
            assert_eq!(other.bare.git(&["show-ref"]), before_push, "{case}");
        }
    });
}

#[test]
fn a_remote_retargeted_after_preparation_is_not_pushed() {
    run(async {
        let remote = Remote::new("push-retargeted");
        remote.repo.git(&["switch", "-c", "feature/topic"]);
        Remote::commit(&remote.repo, "local.txt");
        let tree = crate::diff::WorkingTree::open(remote.repo.path())
            .await
            .expect("repository");
        let transfer = tree.prepare_push(None).await.expect("prepared");
        let second = Repo::new("push-retargeted-second");
        remote.repo.git(&[
            "remote",
            "set-url",
            "origin",
            second.path().to_str().unwrap(),
        ]);
        let pushed = tree.push(transfer).await;
        assert!(
            matches!(pushed, Err(DiffError::PushDestinations)),
            "{pushed:?}"
        );
        assert_eq!(second.git(&["branch", "--list", "feature/topic"]), "");
    });
}

#[test]
fn push_destination_comparison_preserves_repository_paths() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        for file_url in [true, false] {
            let remote = Remote::new(&format!("push-destination-path-{file_url}"));
            let endpoints = Repo(remote.repo.path().with_extension("endpoints"));
            std::fs::create_dir_all(endpoints.path().join("https:")).unwrap();
            let fetch = format!("{}/https://alice@repo.git", endpoints.path().display());
            let push = format!("{}/https://bob@repo.git", endpoints.path().display());
            for endpoint in [&fetch, &push] {
                remote.repo.git(&[
                    "clone",
                    "--bare",
                    remote.bare.path().to_str().unwrap(),
                    endpoint,
                ]);
            }
            let fetched = Repo(fetch.clone().into());
            let pushed = Repo(push.clone().into());
            let prefix = if file_url { "file://" } else { "" };
            remote
                .repo
                .git(&["config", "remote.origin.url", &format!("{prefix}{fetch}")]);
            remote.repo.git(&[
                "config",
                "remote.origin.pushurl",
                &format!("{prefix}{push}"),
            ]);
            remote.repo.git(&["switch", "-c", "feature/path"]);
            Remote::commit(&remote.repo, "local.txt");
            let hook = remote.repo.path().join(".git/hooks/pre-push");
            std::fs::write(&hook, "#!/bin/sh\ntouch .git/push-hook-ran\n").unwrap();
            std::fs::set_permissions(hook, std::fs::Permissions::from_mode(0o755)).unwrap();
            let before_fetch = fetched.git(&["show-ref"]);
            let before_push = pushed.git(&["show-ref"]);
            let result = Checkpoints::new(remote.repo.path()).push(None).await;
            assert!(
                matches!(result, Err(DiffError::PushDestinations)),
                "{file_url}: {result:?}"
            );
            assert!(!remote.repo.path().join(".git/push-hook-ran").exists());
            assert_eq!(fetched.git(&["show-ref"]), before_fetch);
            assert_eq!(pushed.git(&["show-ref"]), before_push);
        }
    });
}

#[test]
fn push_destination_comparison_preserves_trailing_carriage_returns() {
    run(async {
        for same in [false, true] {
            let remote = Remote::new(&format!("push-destination-carriage-{same}"));
            let fetched = Repo(format!("{}\r", remote.bare.path().display()).into());
            remote.repo.git(&[
                "clone",
                "--bare",
                remote.bare.path().to_str().unwrap(),
                fetched.path().to_str().unwrap(),
            ]);
            remote.repo.git(&[
                "config",
                "remote.origin.url",
                fetched.path().to_str().unwrap(),
            ]);
            remote.repo.git(&[
                "config",
                "remote.origin.pushurl",
                if same {
                    fetched.path()
                } else {
                    remote.bare.path()
                }
                .to_str()
                .unwrap(),
            ]);
            remote.repo.git(&["switch", "-c", "feature/carriage"]);
            Remote::commit(&remote.repo, "local.txt");
            let before_fetch = fetched.git(&["show-ref"]);
            let before_push = remote.bare.git(&["show-ref"]);
            let result = Checkpoints::new(remote.repo.path()).push(None).await;
            if same {
                result.expect("the same carriage-return path remains a valid destination");
                assert_eq!(
                    fetched.git(&["rev-parse", "feature/carriage"]),
                    remote.repo.git(&["rev-parse", "HEAD"])
                );
            } else {
                assert!(
                    matches!(result, Err(DiffError::PushDestinations)),
                    "{result:?}"
                );
                assert_eq!(fetched.git(&["show-ref"]), before_fetch);
            }
            assert_eq!(remote.bare.git(&["show-ref"]), before_push);
        }
    });
}

#[test]
fn push_never_recurses_into_submodules_despite_configuration() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        for mode in ["check", "on-demand", "only"] {
            let remote = Remote::new(&format!("push-submodules-{mode}"));
            let submodule = Remote::new(&format!("push-submodules-child-{mode}"));
            remote.repo.git(&["switch", "-c", "feature"]);
            remote.repo.git(&[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                submodule.bare.path().to_str().unwrap(),
                "child",
            ]);
            remote.repo.git(&["commit", "-m", "Add submodule"]);
            let child = Repo(remote.repo.path().join("child"));
            Remote::commit(&child, "unpublished.txt");
            remote.repo.git(&["commit", "-am", "Advance submodule"]);
            remote.repo.git(&["config", "push.recurseSubmodules", mode]);
            let hooks = child.path().join("push-hooks");
            std::fs::create_dir(&hooks).unwrap();
            let hook = hooks.join("pre-push");
            let marker = remote.repo.path().join(".git/submodule-push-ran");
            std::fs::write(
                &hook,
                format!("#!/bin/sh\ntouch {}\n", super::shell_word(marker.display())),
            )
            .unwrap();
            std::fs::set_permissions(hook, std::fs::Permissions::from_mode(0o755)).unwrap();
            child.git(&["config", "core.hooksPath", hooks.to_str().unwrap()]);
            let before = submodule.bare.git(&["show-ref"]);
            let pushed = Checkpoints::new(remote.repo.path()).push(None).await;
            assert!(pushed.is_ok(), "{mode}: {pushed:?}");
            assert_eq!(
                remote.bare.git(&["rev-parse", "feature"]),
                remote.repo.git(&["rev-parse", "HEAD"])
            );
            assert_eq!(submodule.bare.git(&["show-ref"]), before, "{mode}");
            assert!(!marker.exists(), "{mode}");
        }
    });
}

#[test]
fn push_time_credential_helper_failures_identify_sign_in() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        let remote = Remote::new("push-time-auth");
        remote.repo.git(&["switch", "-c", "feature"]);
        Remote::commit(&remote.repo, "local.txt");
        let receiver = remote.repo.path().join(".git/auth-receive-pack");
        let marker = remote.repo.path().join(".git/credential-helper-ran");
        std::fs::write(&receiver, format!("#!/bin/sh\nprintf 'protocol=https\\nhost=example.invalid\\n\\n' | git -c credential.helper= -c {} credential fill\nexit 1\n", super::shell_word(format!("credential.helper=!f() {{ touch {}; exit 1; }}; f", super::shell_word(marker.display()))))).unwrap();
        std::fs::set_permissions(&receiver, std::fs::Permissions::from_mode(0o755)).unwrap();
        remote.repo.git(&[
            "config",
            "remote.origin.receivepack",
            receiver.to_str().unwrap(),
        ]);
        let before = remote.bare.git(&["show-ref"]);
        let result = Checkpoints::new(remote.repo.path()).push(None).await;
        assert!(marker.exists(), "push invokes only the synthetic helper");
        assert!(
            matches!(result, Err(DiffError::SignInFailed(ref transfer, ref detail)) if transfer.remote == "origin" && detail.contains("could not read Username")),
            "{result:?}"
        );
        assert_eq!(remote.bare.git(&["show-ref"]), before);
    });
}

#[test]
fn fetches_serialize_across_instances_and_processes_after_remote_advances() {
    const CHILD: &str = "WATERSHED_FETCH_SERIAL_TEST";
    if let Some(path) = std::env::var_os(CHILD) {
        run(async {
            Checkpoints::new(&std::path::PathBuf::from(path))
                .fetch()
                .await
                .unwrap();
        });
        return;
    }
    run(async {
        for case in ["instance", "process", "worktree"] {
            let process = case == "process";
            let remote = Remote::new(&format!("fetch-shared-{case}"));
            let linked = (case == "worktree").then(|| {
                let repo = Repo(remote.repo.path().with_extension("linked"));
                remote.repo.git(&[
                    "worktree",
                    "add",
                    "-b",
                    "linked",
                    repo.path().to_str().unwrap(),
                ]);
                repo
            });
            remote.advance();
            let (entered, release) = gated_fetch(&remote);
            let first = Checkpoints::new(remote.repo.path());
            let fetching = tokio::spawn(async move { first.fetch().await });
            wait_for(&entered).await;
            let mut child = if process {
                Some(std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "tests::sync::fetches_serialize_across_instances_and_processes_after_remote_advances"])
                    .env(CHILD, remote.repo.path())
                    .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())
                    .spawn().unwrap())
            } else {
                None
            };
            let second = if process {
                None
            } else {
                let second = Checkpoints::new(linked.as_ref().unwrap_or(&remote.repo).path());
                Some(tokio::spawn(async move { second.fetch().await }))
            };
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            std::fs::write(release, "").unwrap();
            let first_result = fetching.await.unwrap();
            let second_result = if let Some(second) = second {
                second.await.unwrap()
            } else {
                Ok(Some("origin".to_owned()))
            };
            let output = child.take().map(|child| child.wait_with_output().unwrap());
            assert!(
                !remote.repo.path().join(".git/fetch-overlap").exists(),
                "{case}: {first_result:?} {second_result:?} {output:?}"
            );
            first_result.unwrap();
            second_result.unwrap();
            if let Some(output) = output {
                assert!(output.status.success(), "{output:?}");
            }
            assert_eq!(
                remote.repo.git(&["rev-parse", "origin/main"]),
                remote.bare.git(&["rev-parse", "main"])
            );
        }
    });
}

#[test]
fn push_uses_the_first_configured_merge_ref_like_git() {
    run(async {
        let remote = Remote::new("push-multiple-merge-refs");
        remote
            .repo
            .git(&["push", "origin", "HEAD:refs/heads/release"]);
        remote.repo.git(&["switch", "-c", "topic"]);
        Remote::commit(&remote.repo, "local.txt");
        remote
            .repo
            .git(&["config", "branch.topic.remote", "origin"]);
        for reference in ["refs/heads/main", "refs/heads/release"] {
            remote
                .repo
                .git(&["config", "--add", "branch.topic.merge", reference]);
        }
        assert_eq!(
            remote.repo.git(&[
                "for-each-ref",
                "--format=%(upstream:short)",
                "refs/heads/topic"
            ]),
            "origin/main\n"
        );
        let before = remote.bare.git(&["show-ref"]);
        let DiffError::PushDefault(confirmation) = Checkpoints::new(remote.repo.path())
            .push(None)
            .await
            .unwrap_err()
        else {
            panic!("confirmation required")
        };
        assert_eq!(confirmation.confirmation.reference, "refs/heads/main");
        assert_eq!(confirmation.upstream, "origin/main");
        assert_eq!(confirmation.ahead, 1);
        assert_eq!(remote.bare.git(&["show-ref"]), before);
        let release = remote.bare.git(&["rev-parse", "release"]);
        Checkpoints::new(remote.repo.path())
            .push(Some(confirmation.confirmation))
            .await
            .unwrap();
        assert_eq!(
            remote.bare.git(&["rev-parse", "main"]),
            remote.repo.git(&["rev-parse", "HEAD"])
        );
        assert_eq!(remote.bare.git(&["rev-parse", "release"]), release);
        assert_eq!(
            remote
                .repo
                .git(&["config", "--get-all", "branch.topic.merge"]),
            "refs/heads/main\nrefs/heads/release\n"
        );
    });
}

#[test]
fn push_counts_use_the_advertised_destination_when_fetch_omits_it() {
    run(async {
        for state in ["ahead", "behind", "diverged", "excluded"] {
            let remote = Remote::new(&format!("push-unmapped-counts-{state}"));
            remote
                .repo
                .git(&["push", "origin", "HEAD:refs/heads/release"]);
            remote.repo.git(&["switch", "-c", "topic"]);
            remote
                .repo
                .git(&["config", "branch.topic.remote", "origin"]);
            remote
                .repo
                .git(&["config", "branch.topic.merge", "refs/heads/main"]);
            if state == "excluded" {
                remote
                    .repo
                    .git(&["config", "--add", "remote.origin.fetch", "^refs/heads/main"]);
            } else {
                remote.repo.git(&[
                    "config",
                    "remote.origin.fetch",
                    "+refs/heads/release:refs/remotes/origin/release",
                ]);
            }
            if state != "ahead" {
                remote.advance();
            }
            if state == "ahead" || state == "diverged" {
                Remote::commit(&remote.repo, "local.txt");
            }
            let before = remote.bare.git(&["show-ref"]);
            let local = remote.repo.git(&["show-ref"]);
            let fetch_head = std::fs::read(remote.repo.path().join(".git/FETCH_HEAD")).unwrap();
            let error = Checkpoints::new(remote.repo.path())
                .push(None)
                .await
                .unwrap_err();
            match (state, error) {
                ("ahead", DiffError::PushDefault(confirmation)) => {
                    assert_eq!((confirmation.ahead, confirmation.behind), (1, 0));
                    assert_eq!(confirmation.confirmation.reference, "refs/heads/main");
                }
                (_, DiffError::Diverged(transfer, _)) => {
                    assert_eq!(
                        (transfer.ahead, transfer.behind),
                        (u64::from(state == "diverged"), 1)
                    );
                }
                (_, error) => panic!("{state}: {error:?}"),
            }
            assert_eq!(remote.bare.git(&["show-ref"]), before);
            assert_eq!(remote.repo.git(&["show-ref"]), local);
            assert_eq!(
                std::fs::read(remote.repo.path().join(".git/FETCH_HEAD")).unwrap(),
                fetch_head
            );
            assert!(remote.repo.git(&["status", "--porcelain"]).is_empty());
        }
    });
}

#[test]
fn first_push_ignores_a_merge_ref_without_a_remote() {
    run(async {
        let remote = Remote::new("push-incomplete-upstream");
        remote
            .repo
            .git(&["push", "origin", "HEAD:refs/heads/release"]);
        remote.repo.git(&["switch", "-c", "topic"]);
        Remote::commit(&remote.repo, "local.txt");
        remote
            .repo
            .git(&["config", "branch.topic.merge", "refs/heads/release"]);
        let release = remote.bare.git(&["rev-parse", "release"]);
        let pushed = Checkpoints::new(remote.repo.path())
            .push(None)
            .await
            .unwrap();
        assert!(pushed.first_push);
        assert_eq!(pushed.confirmation.reference, "refs/heads/topic");
        assert_eq!(pushed.upstream, "origin/topic");
        assert_eq!(remote.bare.git(&["rev-parse", "release"]), release);
        assert_eq!(
            remote.bare.git(&["rev-parse", "topic"]),
            remote.repo.git(&["rev-parse", "HEAD"])
        );
    });
}

#[test]
fn push_confirmation_names_an_unmapped_destination_branch() {
    run(async {
        let remote = Remote::new("push-unmapped-destination");
        remote
            .repo
            .git(&["push", "origin", "HEAD:refs/heads/release"]);
        remote.repo.git(&["switch", "-c", "topic"]);
        Remote::commit(&remote.repo, "local.txt");
        remote
            .repo
            .git(&["config", "branch.topic.remote", "origin"]);
        remote
            .repo
            .git(&["config", "branch.topic.merge", "refs/heads/main"]);
        remote.repo.git(&[
            "config",
            "remote.origin.fetch",
            "+refs/heads/release:refs/remotes/origin/release",
        ]);
        let before = remote.bare.git(&["show-ref"]);
        let DiffError::PushDefault(confirmation) = Checkpoints::new(remote.repo.path())
            .push(None)
            .await
            .unwrap_err()
        else {
            panic!("confirmation required")
        };
        assert_eq!(confirmation.confirmation.reference, "refs/heads/main");
        assert_eq!(confirmation.upstream, "origin/main");
        assert_eq!(remote.bare.git(&["show-ref"]), before);
    });
}

#[test]
fn push_preserves_the_configured_remote_without_a_tracking_ref_mapping() {
    run(async {
        let origin = Remote::new("push-unmapped-origin");
        let upstream = Remote::new("push-unmapped-upstream");
        origin.repo.git(&["switch", "-c", "topic"]);
        Remote::commit(&origin.repo, "local.txt");
        origin.repo.git(&[
            "remote",
            "add",
            "upstream",
            upstream.bare.path().to_str().unwrap(),
        ]);
        origin.repo.git(&[
            "config",
            "remote.upstream.fetch",
            "+refs/heads/main:refs/remotes/upstream/main",
        ]);
        origin
            .repo
            .git(&["config", "branch.topic.remote", "upstream"]);
        origin
            .repo
            .git(&["config", "branch.topic.merge", "refs/heads/topic"]);
        assert!(
            origin
                .repo
                .git(&[
                    "for-each-ref",
                    "--format=%(upstream:remotename)",
                    "refs/heads/topic"
                ])
                .trim()
                .is_empty()
        );
        let before = origin.bare.git(&["show-ref"]);
        let pushed = Checkpoints::new(origin.repo.path())
            .push(None)
            .await
            .unwrap();
        assert_eq!(pushed.remote, "upstream");
        assert_eq!(origin.bare.git(&["show-ref"]), before);
        assert_eq!(
            upstream.bare.git(&["rev-parse", "topic"]),
            origin.repo.git(&["rev-parse", "HEAD"])
        );
        assert_eq!(
            origin.repo.git(&["config", "branch.topic.remote"]),
            "upstream\n"
        );
    });
}
