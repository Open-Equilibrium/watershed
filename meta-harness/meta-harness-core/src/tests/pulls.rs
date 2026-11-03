use super::checkpoint::{Repo, run};
use super::shell_word;
use crate::{
    Checkpoints, DiffError, PullRequestDraft, PullRequestError, PullRequestState, ReviewDecision,
};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

const BRANCH: &str = "feature/retry-after";
const LOGGED_IN: &str =
    r#"{"hosts":{"github.com":[{"state":"success","active":true,"host":"github.com"}]}}"#;
const FORK: &str = r#"{"full_name":"bw/api","fork":true,"default_branch":"main","parent":{"full_name":"acme/api","default_branch":"main"}}"#;
/// The fork's pull request #42 as `gh pr view` reads it.
const OPEN: &str = r#"{"number":42,"title":"Return 429","state":"OPEN","isDraft":false,"url":"https://github.com/acme/api/pull/42","baseRefName":"develop","headRefName":"feature/retry-after","headRepository":{"name":"api"},"headRepositoryOwner":{"login":"bw"},"statusCheckRollup":[],"reviewDecision":""}"#;

/// A branch pushed to a bare repository, whose remote then names GitHub; no Git network runs
/// for a pull request, and the fake gh answers for GitHub.
struct Pushed {
    repo: Repo,
    gh: PathBuf,
}

impl Pushed {
    fn new(name: &str, url: &str) -> Self {
        let repo = Repo::new(&format!("pr-{name}"));
        repo.git(&["branch", "-M", "main"]);
        let bare = repo.path().with_extension("bare");
        let _ = std::fs::remove_dir_all(&bare);
        repo.git(&["clone", "--bare", ".", bare.to_str().unwrap()]);
        repo.git(&["remote", "add", "origin", bare.to_str().unwrap()]);
        repo.git(&["switch", "-c", BRANCH]);
        repo.write("b.txt", "retry\n");
        repo.git(&["add", "b.txt"]);
        repo.git(&["commit", "-m", "Retry after\n\nDetails."]);
        repo.git(&["push", "-u", "origin", BRANCH]);
        repo.git(&["remote", "set-url", "origin", url]);
        let gh = repo.path().with_extension("gh");
        let _ = std::fs::remove_dir_all(&gh);
        std::fs::create_dir_all(&gh).unwrap();
        let pushed = Self { repo, gh };
        pushed.answer("auth", LOGGED_IN);
        pushed.answer("repo", FORK);
        pushed.answer("head", &pushed.head());
        pushed.answer("prs", "");
        pushed.answer("fork-prs", "");
        pushed
    }

    fn head(&self) -> String {
        self.repo.git(&["rev-parse", "HEAD"]).trim().to_owned()
    }

    fn answer(&self, name: &str, text: &str) {
        std::fs::write(self.gh.join(name), text).unwrap();
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.gh.join(name)).unwrap_or_default()
    }

    /// A gh that answers `cases` (shell `case` arms over its arguments) before the shared ones and
    /// logs every call.
    fn checkpoints(&self, cases: &str) -> Checkpoints {
        let dir = shell_word(self.gh.display());
        let script = self.gh.join("gh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nd={dir}\nprintf '%s\\n' \"$*\" >> \"$d/log\"\ncase \"$*\" in\n\
                 {cases}\n\
                 'auth status --json hosts --hostname '*) cat \"$d/auth\" ;;\n\
                 'api --hostname github.com repos/bw/api') cat \"$d/repo\" ;;\n\
                 'api --hostname github.com repos/bw/api/branches/feature/retry-after --jq .commit.sha') cat \"$d/head\" ;;\n\
                 'api --hostname github.com repos/acme/api/pulls?head=bw:feature/retry-after&state=all&per_page=100 --jq '*) cat \"$d/prs\" ;;\n\
                 'api --hostname github.com repos/bw/api/pulls?head=bw:feature/retry-after&state=all&per_page=100 --jq '*) cat \"$d/fork-prs\" ;;\n\
                 'pr view '*' --repo github.com/'*) cat \"$d/view-$3\" ;;\n\
                 'api --hostname github.com repos/acme/api/compare/develop...bw:feature/retry-after --jq '*) echo '{{\"commits\":2,\"url\":\"https://github.com/acme/api/compare\",\"files\":3,\"added\":12,\"removed\":1}}' ;;\n\
                 'api --hostname github.com repos/acme/api/compare/'*) echo 'gh: Not Found (HTTP 404)' >&2; exit 1 ;;\n\
                 *) echo \"unexpected: $*\" >&2; exit 9 ;;\nesac\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut checkpoints = Checkpoints::new(self.repo.path());
        checkpoints.set_gh(&script);
        checkpoints
    }
}

fn draft(plan: &crate::PullRequestPlan, base: &str) -> PullRequestDraft {
    PullRequestDraft {
        host: plan.host.clone(),
        target: plan.target.clone().unwrap(),
        head_repository: plan.head_repository.clone(),
        head_branch: plan.head_branch.clone(),
        head_commit: plan.head_commit.clone(),
        base: base.to_owned(),
        title: "Return 429 with Retry-After".to_owned(),
        body: "Edited.".to_owned(),
        draft: true,
    }
}

#[test]
fn a_fork_targets_its_parent_and_creates_only_the_reviewed_pull_request() {
    run(async {
        let pushed = Pushed::new("fork", "https://github.com/bw/api.git");
        let created = r#"{"number":42,"title":"Return 429 with Retry-After","html_url":"https://github.com/acme/api/pull/42","draft":true}"#;
        pushed.answer("created", created);
        let checkpoints = pushed.checkpoints(
            "'api --hostname github.com --method POST repos/acme/api/pulls --input -') cat > \"$d/input\"; cat \"$d/created\" ;;",
        );

        let plan = checkpoints
            .pull_request_plan(None, Some("develop"))
            .await
            .unwrap();
        assert_eq!(plan.targets, ["acme/api", "bw/api"]);
        assert_eq!(plan.target.as_deref(), Some("acme/api"));
        assert_eq!(plan.head_repository, "bw/api");
        assert_eq!(plan.head_branch, BRANCH);
        assert_eq!(plan.head_commit, pushed.head());
        assert_eq!(plan.default_branch.as_deref(), Some("main"));
        assert_eq!(plan.base.as_deref(), Some("develop"));
        let comparison = plan.comparison.clone().unwrap();
        assert_eq!((comparison.commits, comparison.files), (2, 3));
        assert_eq!((comparison.added, comparison.removed), (12, 1));
        assert_eq!(
            (plan.subject.as_str(), plan.body.as_str()),
            ("Retry after", "Details.")
        );
        assert!(plan.open.is_none());
        // GitHub's default preselects without a recorded base; a base GitHub lacks is shown.
        let missing = checkpoints.pull_request_plan(None, None).await.unwrap();
        assert_eq!(missing.base.as_deref(), Some("main"));
        assert!(missing.base_missing);

        let pull = checkpoints
            .create_pull_request(&draft(&plan, "develop"))
            .await
            .unwrap();
        assert_eq!(pull.number, 42);
        assert_eq!(pull.state, PullRequestState::Draft);
        assert_eq!(pull.head, "bw:feature/retry-after");
        let sent: serde_json::Value = serde_json::from_str(&pushed.read("input")).unwrap();
        assert_eq!(
            sent,
            serde_json::json!({
                "title": "Return 429 with Retry-After",
                "body": "Edited.",
                "head": "bw:feature/retry-after",
                "base": "develop",
                "draft": true,
            })
        );

        // An open pull request is shown instead of creating another.
        pushed.answer("prs", "42 open bw/api feature/retry-after");
        pushed.answer("view-42", OPEN);
        let calls = pushed.read("log").lines().count();
        let shown = checkpoints
            .pull_request_plan(None, Some("develop"))
            .await
            .unwrap();
        assert_eq!(shown.open.as_ref().map(|pull| pull.number), Some(42));
        assert!(matches!(
            checkpoints.create_pull_request(&draft(&plan, "develop")).await,
            Err(PullRequestError::Exists(pull)) if pull.number == 42
        ));
        assert!(
            !pushed
                .read("log")
                .lines()
                .skip(calls)
                .any(|call| call.contains("POST"))
        );
    });
}

#[test]
fn a_changed_head_or_base_asks_for_review_again_before_creating() {
    run(async {
        let pushed = Pushed::new("changed", "https://github.com/bw/api.git");
        let checkpoints = pushed.checkpoints("");
        let plan = checkpoints
            .pull_request_plan(None, Some("develop"))
            .await
            .unwrap();
        pushed.repo.write("c.txt", "more\n");
        pushed.repo.git(&["add", "c.txt"]);
        pushed.repo.git(&["commit", "-m", "More"]);
        pushed.repo.git(&[
            "update-ref",
            "refs/remotes/origin/feature/retry-after",
            "HEAD",
        ]);
        pushed.answer("head", &pushed.head());
        let Err(PullRequestError::Changed(fresh)) = checkpoints
            .create_pull_request(&draft(&plan, "develop"))
            .await
        else {
            panic!("the moved head needs another review")
        };
        assert_eq!(fresh.head_commit, pushed.head());
        assert!(matches!(
            checkpoints.create_pull_request(&draft(&fresh, "gone")).await,
            Err(PullRequestError::Changed(plan)) if plan.base_missing
        ));
        assert!(!pushed.read("log").contains("POST"));
    });
}

#[test]
fn refuses_what_cannot_become_a_pull_request_and_says_why() {
    run(async {
        // Local refusals run no gh.
        let local = Pushed::new("local", "/srv/git/api.git");
        assert!(matches!(
            local.checkpoints("").pull_request_plan(None, None).await,
            Err(PullRequestError::NotGitHub { host: None, .. })
        ));
        let ahead = Pushed::new("ahead", "https://github.com/bw/api.git");
        ahead.repo.write("c.txt", "more\n");
        ahead.repo.git(&["add", "c.txt"]);
        ahead.repo.git(&["commit", "-m", "More"]);
        assert!(matches!(
            ahead.checkpoints("").pull_request_plan(None, None).await,
            Err(PullRequestError::NeedsPush {
                ahead: 1,
                first: false,
                ..
            })
        ));
        ahead.repo.git(&["switch", "-c", "unpublished"]);
        assert!(matches!(
            ahead.checkpoints("").pull_request_plan(None, None).await,
            Err(PullRequestError::NeedsPush { first: true, .. })
        ));
        let elsewhere = Pushed::new("elsewhere", "https://github.com/bw/api.git");
        elsewhere.repo.git(&[
            "config",
            "remote.origin.pushurl",
            "https://github.com/bw/other.git",
        ]);
        assert!(matches!(
            elsewhere
                .checkpoints("")
                .pull_request_plan(None, None)
                .await,
            Err(PullRequestError::Git(DiffError::PushDestinations))
        ));
        for pushed in [&local, &ahead, &elsewhere] {
            assert_eq!(pushed.read("log"), "");
        }

        // gh's answers: missing, signed out, an unknown or Enterprise host, no access, offline.
        let pushed = Pushed::new("gh", "https://github.com/bw/api.git");
        assert!(matches!(
            pushed
                .checkpoints("'auth status '*) echo 'unknown flag: --json' >&2; exit 1 ;;")
                .pull_request_plan(None, None)
                .await,
            Err(PullRequestError::GhOutdated(_))
        ));
        let mut missing = pushed.checkpoints("");
        missing.set_gh(&pushed.gh.join("missing"));
        assert!(matches!(
            missing.pull_request_plan(None, None).await,
            Err(PullRequestError::GhMissing(_))
        ));
        pushed.answer("auth", r#"{"hosts":{}}"#);
        assert!(matches!(
            pushed.checkpoints("").pull_request_plan(None, None).await,
            Err(PullRequestError::SignIn { host }) if host == "github.com"
        ));
        pushed
            .repo
            .git(&["remote", "set-url", "origin", "git@gitlab.com:bw/api.git"]);
        assert!(matches!(
            pushed.checkpoints("").pull_request_plan(None, None).await,
            Err(PullRequestError::NotGitHub { host: Some(host), remote }) if host == "gitlab.com" && remote == "origin"
        ));
        pushed.repo.git(&[
            "remote",
            "set-url",
            "origin",
            "https://github.acme.example/bw/api.git",
        ]);
        pushed.answer(
            "auth",
            r#"{"hosts":{"github.acme.example":[{"state":"error","active":true,"host":"github.acme.example"}]}}"#,
        );
        assert!(matches!(
            pushed.checkpoints("").pull_request_plan(None, None).await,
            Err(PullRequestError::SignIn { host }) if host == "github.acme.example"
        ));
        pushed.answer(
            "auth",
            r#"{"hosts":{"github.acme.example":[{"state":"success","active":true,"host":"github.acme.example"}]}}"#,
        );
        let enterprise = pushed
            .checkpoints(
                "'api --hostname github.acme.example repos/bw/api') echo '{\"full_name\":\"bw/api\",\"fork\":false,\"default_branch\":\"trunk\"}' ;;\n\
                 'api --hostname github.acme.example repos/bw/api/branches/'*) cat \"$d/head\" ;;\n\
                 'api --hostname github.acme.example repos/bw/api/pulls?'*) ;;\n\
                 'api --hostname github.acme.example repos/bw/api/compare/trunk...bw:feature/retry-after --jq '*) echo '{\"commits\":1,\"files\":1,\"added\":1,\"removed\":0,\"url\":\"\"}' ;;",
            )
            .pull_request_plan(None, None)
            .await
            .unwrap();
        assert_eq!(enterprise.host, "github.acme.example");
        assert_eq!(enterprise.targets, ["bw/api"]);
        assert_eq!(enterprise.base.as_deref(), Some("trunk"));

        pushed.repo.git(&[
            "remote",
            "set-url",
            "origin",
            "https://github.com/bw/api.git",
        ]);
        pushed.answer("auth", LOGGED_IN);
        assert!(matches!(
            pushed
                .checkpoints("'api --hostname github.com repos/bw/api') echo 'gh: Not Found (HTTP 404)' >&2; exit 1 ;;")
                .pull_request_plan(None, None)
                .await,
            Err(PullRequestError::NoAccess { repository, .. }) if repository == "bw/api"
        ));
        assert!(matches!(
            pushed
                .checkpoints("'api --hostname github.com repos/bw/api') echo 'error connecting to api.github.com' >&2; exit 1 ;;")
                .pull_request_plan(None, None)
                .await,
            Err(PullRequestError::Unreachable { host, .. }) if host == "github.com"
        ));
    });
}

#[test]
fn a_refused_creation_is_not_created_and_a_silent_one_is_unknown() {
    run(async {
        let pushed = Pushed::new("create-fails", "https://github.com/bw/api.git");
        let refused = pushed.checkpoints(
            "'api --hostname github.com --method POST '*) echo 'gh: Validation Failed (HTTP 422)' >&2; exit 1 ;;",
        );
        let plan = refused
            .pull_request_plan(None, Some("develop"))
            .await
            .unwrap();
        assert!(matches!(
            refused.create_pull_request(&draft(&plan, "develop")).await,
            Err(PullRequestError::NotCreated(detail)) if detail.contains("HTTP 422")
        ));
        let silent = pushed.checkpoints(
            "'api --hostname github.com --method POST '*) echo 'connection reset by peer' >&2; exit 1 ;;",
        );
        assert!(matches!(
            silent.create_pull_request(&draft(&plan, "develop")).await,
            Err(PullRequestError::Unknown(_))
        ));
    });
}

#[test]
fn the_summary_reads_the_open_pull_request_with_checks_and_review() {
    run(async {
        let pushed = Pushed::new("summary", "https://github.com/bw/api.git");
        // GitHub filters by the head's owner; another owner's head is still not taken.
        pushed.answer(
            "prs",
            "43 open someone/api feature/retry-after\n42 open bw/api feature/retry-after\n40 closed bw/api feature/retry-after",
        );
        pushed.answer(
            "view-42",
            r#"{"number":42,"title":"Return 429","state":"OPEN","isDraft":true,"url":"https://github.com/acme/api/pull/42","baseRefName":"main","headRefName":"feature/retry-after","headRepository":{"name":"api"},"headRepositoryOwner":{"login":"bw"},
               "statusCheckRollup":[{"__typename":"CheckRun","status":"COMPLETED","conclusion":"SUCCESS"},{"__typename":"CheckRun","status":"IN_PROGRESS","conclusion":""},{"__typename":"StatusContext","state":"FAILURE"}],
               "reviewDecision":"CHANGES_REQUESTED"}"#,
        );
        let pull = pushed
            .checkpoints("")
            .pull_request()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(pull.number, 42);
        assert_eq!(pull.state, PullRequestState::Draft);
        assert_eq!(pull.repository, "acme/api");
        assert_eq!(pull.head, "bw:feature/retry-after");
        assert_eq!(
            (pull.checks_passed, pull.checks_failed, pull.checks_pending),
            (1, 1, 1)
        );
        assert_eq!(pull.review, Some(ReviewDecision::ChangesRequested));
        // A merged or closed pull request stays history once none is open, in the parent first.
        pushed.answer(
            "prs",
            "42 closed bw/api feature/retry-after\n40 closed bw/api feature/retry-after",
        );
        pushed.answer("fork-prs", "7 open bw/api feature/retry-after");
        pushed.answer(
            "view-7",
            &OPEN
                .replace("\"number\":42", "\"number\":7")
                .replace("acme/api/pull/42", "bw/api/pull/7"),
        );
        let open = pushed
            .checkpoints("")
            .pull_request()
            .await
            .unwrap()
            .unwrap();
        assert_eq!((open.number, open.repository.as_str()), (7, "bw/api"));
        pushed.answer("fork-prs", "");
        pushed.answer(
            "view-42",
            &pushed.read("view-42").replace("\"OPEN\"", "\"CLOSED\""),
        );
        let latest = pushed
            .checkpoints("")
            .pull_request()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest.number, 42);
        assert_eq!(latest.state, PullRequestState::Closed);
        // A full page without an open pull request is not read as none.
        pushed.answer(
            "prs",
            &vec!["40 closed bw/api feature/retry-after"; 100].join("\n"),
        );
        assert!(matches!(
            pushed
                .checkpoints("")
                .pull_request_plan(None, Some("develop"))
                .await,
            Err(PullRequestError::Failed(_))
        ));
        // Without an upstream there is nothing to look up.
        pushed.repo.git(&["switch", "-c", "unpublished"]);
        assert!(
            pushed
                .checkpoints("")
                .pull_request()
                .await
                .unwrap()
                .is_none()
        );
    });
}

#[test]
fn a_busy_parent_does_not_hide_the_forks_pull_request() {
    run(async {
        let pushed = Pushed::new("busy", "https://github.com/bw/api.git");
        let others = vec![
            r#"{"number":1,"title":"Other","state":"OPEN","isDraft":false,"url":"u","baseRefName":"main","headRefName":"feature/retry-after","headRepository":{"name":"api"},"headRepositoryOwner":{"login":"someone"},"statusCheckRollup":[],"reviewDecision":""}"#;
            20
        ];
        pushed.answer("list", &format!("[{}]", others.join(",")));
        pushed.answer("prs", "42 open bw/api feature/retry-after");
        pushed.answer("view-42", OPEN);
        let checkpoints = pushed.checkpoints(
            "'pr list --repo github.com/acme/api '*) cat \"$d/list\" ;;\n\
             'pr list --repo github.com/bw/api '*) echo '[]' ;;",
        );
        let plan = checkpoints
            .pull_request_plan(None, Some("develop"))
            .await
            .unwrap();
        assert_eq!(plan.open.map(|pull| pull.number), Some(42));
        let pull = checkpoints.pull_request().await.unwrap().unwrap();
        assert_eq!(pull.number, 42);
    });
}

#[test]
fn a_remote_gh_has_no_github_login_for_is_never_asked_about() {
    run(async {
        let pushed = Pushed::new("gitlab", "https://gitlab.com/bw/api.git");
        pushed.answer("auth", r#"{"hosts":{}}"#);
        let checkpoints = pushed.checkpoints("");
        assert!(matches!(
            checkpoints.pull_request().await,
            Err(PullRequestError::NotGitHub { host: Some(host), remote }) if host == "gitlab.com" && remote == "origin"
        ));
        assert!(matches!(
            checkpoints.pull_request_bases("gitlab.com", "bw/api").await,
            Err(PullRequestError::SignIn { host }) if host == "gitlab.com"
        ));
        assert!(
            pushed
                .read("log")
                .lines()
                .all(|call| call.starts_with("auth status ")),
            "{}",
            pushed.read("log")
        );
    });
}

#[test]
fn bases_are_read_page_by_page_up_to_a_bound() {
    run(async {
        let pushed = Pushed::new("bases", "https://github.com/bw/api.git");
        let checkpoints = pushed.checkpoints(
            "'api --hostname github.com repos/acme/api/branches?per_page=100&page=1 --jq .[].name') seq -f 'b%g' 1 100 ;;\n\
             'api --hostname github.com repos/acme/api/branches?per_page=100&page=2 --jq .[].name') printf 'main\\nrelease/1.2\\n' ;;\n\
             'api --hostname github.com repos/bw/api/branches?per_page=100&page='*) seq -f 'f%g' 1 100 ;;",
        );
        let (names, more) = checkpoints
            .pull_request_bases("github.com", "acme/api")
            .await
            .unwrap();
        assert_eq!((names.len(), more), (102, false));
        assert_eq!(names.last().map(String::as_str), Some("release/1.2"));
        let (names, more) = checkpoints
            .pull_request_bases("github.com", "bw/api")
            .await
            .unwrap();
        assert_eq!((names.len(), more), (500, true));
    });
}

#[test]
fn a_base_github_cannot_compare_keeps_the_plan_and_an_unsafe_base_is_missing() {
    run(async {
        let pushed = Pushed::new("compare", "https://github.com/bw/api.git");
        let checkpoints = pushed.checkpoints(
            "'api --hostname github.com repos/acme/api/compare/unrelated...'*) echo 'gh: No common ancestor (HTTP 422)' >&2; exit 1 ;;\n\
             'api --hostname github.com repos/acme/api/compare/busy...'*) echo 'gh: Server Error (HTTP 502)' >&2; exit 1 ;;",
        );
        for base in ["unrelated", "busy"] {
            let plan = checkpoints
                .pull_request_plan(None, Some(base))
                .await
                .unwrap();
            assert!(
                plan.comparison.is_none() && plan.comparison_failed,
                "{base}"
            );
            assert!(!plan.base_missing, "{base}");
        }
        let calls = pushed.read("log").lines().count();
        for base in ["", "-x", "../main", "a/./b", "a\u{7}b"] {
            let plan = checkpoints
                .pull_request_plan(None, Some(base))
                .await
                .unwrap();
            assert!(plan.base_missing && !plan.comparison_failed, "{base:?}");
        }
        assert!(
            !pushed
                .read("log")
                .lines()
                .skip(calls)
                .any(|call| call.contains("/compare/")),
        );
    });
}

#[test]
fn a_recheck_that_times_out_created_nothing_and_a_stopped_request_is_unknown() {
    run(async {
        let pushed = Pushed::new("slow", "https://github.com/bw/api.git");
        let plan = pushed
            .checkpoints("")
            .pull_request_plan(None, Some("develop"))
            .await
            .unwrap();
        let mut rechecking = pushed
            .checkpoints("'api --hostname github.com repos/bw/api') sleep 5; cat \"$d/repo\" ;;");
        rechecking.set_timeout(Duration::from_millis(1500));
        assert!(matches!(
            rechecking
                .create_pull_request(&draft(&plan, "develop"))
                .await,
            Err(PullRequestError::Git(DiffError::TimedOut(_)))
        ));
        assert!(!pushed.read("log").contains("POST"));
        let mut posting =
            pushed.checkpoints("'api --hostname github.com --method POST '*) sleep 5 ;;");
        posting.set_timeout(Duration::from_millis(1500));
        assert!(matches!(
            posting.create_pull_request(&draft(&plan, "develop")).await,
            Err(PullRequestError::Unknown(_))
        ));
    });
}
