use super::checkpoint::{Repo, run};
use super::shell_word;
use crate::{
    Checkpoints, DiffError, PullRequestDraft, PullRequestError, PullRequestState, ReviewDecision,
};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

const BRANCH: &str = "feature/retry-after";
const LOGGED_IN: &str =
    r#"{"hosts":{"github.com":[{"state":"success","active":true,"host":"github.com"}]}}"#;
const FORK: &str = r#"{"full_name":"bw/api","fork":true,"default_branch":"main","parent":{"full_name":"acme/api","default_branch":"main"}}"#;

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
        pushed.answer("prs", "[]");
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
                 'pr list --repo github.com/acme/api --head feature/retry-after '*) cat \"$d/prs\" ;;\n\
                 'pr list --repo github.com/bw/api --head feature/retry-after '*) echo '[]' ;;\n\
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
        pushed.answer(
            "prs",
            r#"[{"number":42,"title":"Return 429","state":"OPEN","isDraft":false,"url":"https://github.com/acme/api/pull/42","baseRefName":"develop","headRefName":"feature/retry-after","headRepository":{"name":"api"},"headRepositoryOwner":{"login":"bw"},"statusCheckRollup":[],"reviewDecision":""}]"#,
        );
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
                 'pr list --repo github.acme.example/bw/api '*) echo '[]' ;;\n\
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
        pushed.answer(
            "prs",
            r#"[
              {"number":43,"title":"Other fork","state":"OPEN","isDraft":false,"url":"u","baseRefName":"main","headRefName":"feature/retry-after","headRepository":{"name":"api"},"headRepositoryOwner":{"login":"someone"},"statusCheckRollup":[],"reviewDecision":""},
              {"number":42,"title":"Return 429","state":"OPEN","isDraft":true,"url":"https://github.com/acme/api/pull/42","baseRefName":"main","headRefName":"feature/retry-after","headRepository":{"name":"api"},"headRepositoryOwner":{"login":"bw"},
               "statusCheckRollup":[{"__typename":"CheckRun","status":"COMPLETED","conclusion":"SUCCESS"},{"__typename":"CheckRun","status":"IN_PROGRESS","conclusion":""},{"__typename":"StatusContext","state":"FAILURE"}],
               "reviewDecision":"CHANGES_REQUESTED"},
              {"number":40,"title":"Earlier","state":"MERGED","isDraft":false,"url":"u","baseRefName":"main","headRefName":"feature/retry-after","headRepository":{"name":"api"},"headRepositoryOwner":{"login":"bw"},"statusCheckRollup":[],"reviewDecision":"APPROVED"}
            ]"#,
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
        // A merged pull request stays history once none is open.
        pushed.answer("prs", &pushed.read("prs").replace("\"OPEN\"", "\"CLOSED\""));
        let latest = pushed
            .checkpoints("")
            .pull_request()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest.number, 42);
        assert_eq!(latest.state, PullRequestState::Closed);
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
