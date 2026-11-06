use super::checkpoint::{Repo, run};
use super::shell_word;
use crate::{Checkpoints, PullRequestText};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

const MODEL: &str = "anthropic/claude-test";

/// A branch with one commit beyond `main` and a fake pi that logs how it ran and answers
/// `answer`.
struct Branch {
    repo: Repo,
    pi: PathBuf,
    base: String,
    head: String,
}

impl Branch {
    fn new(name: &str, file: &str, text: &str) -> Self {
        let repo = Repo::new(&format!("pr-text-{name}"));
        repo.git(&["branch", "-M", "main"]);
        let base = repo.git(&["rev-parse", "HEAD"]).trim().to_owned();
        repo.git(&["switch", "-q", "-c", "feature"]);
        repo.write(file, text);
        repo.git(&["add", file]);
        repo.git(&["commit", "-q", "-m", "Retry after\n\nBlocked logins wait."]);
        let head = repo.git(&["rev-parse", "HEAD"]).trim().to_owned();
        let pi = repo.path().with_extension("pi");
        let _ = std::fs::remove_dir_all(&pi);
        std::fs::create_dir_all(&pi).unwrap();
        let branch = Self {
            repo,
            pi,
            base,
            head,
        };
        branch.answer(
            "# Return 429 with Retry-After\n\nBlocked logins get 429.\n",
            0,
        );
        branch
    }

    /// Makes pi print `text` and exit with `status`.
    fn answer(&self, text: &str, status: i32) {
        let dir = shell_word(self.pi.display());
        std::fs::write(self.pi.join("answer"), text).unwrap();
        let script = self.pi.join("pi");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nd={dir}\npwd > \"$d/cwd\"\nls -A > \"$d/listing\"\n\
                 for a in \"$@\"; do printf '%s\\n' \"$a\"; done > \"$d/args\"\n\
                 cat > \"$d/input\"\ncat \"$d/answer\"\nexit {status}\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.pi.join(name)).unwrap_or_default()
    }

    async fn write(&self, checkpoints: &Checkpoints) -> Result<PullRequestText, String> {
        checkpoints
            .write_pull_request_text(
                &self.pi.join("pi"),
                &["--from-settings".into()],
                MODEL,
                &self.base,
                &self.head,
            )
            .await
            .map_err(|error| error.to_string())
    }
}

impl Drop for Branch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pi);
    }
}

#[test]
fn the_chats_model_writes_the_text_from_the_commits_and_diff_without_tools_or_project() {
    run(async {
        let branch = Branch::new("writes", "b.txt", "retry\n");
        let checkpoints = Checkpoints::new(branch.repo.path());

        let text = branch.write(&checkpoints).await.unwrap();

        assert_eq!(text.title, "Return 429 with Retry-After");
        assert_eq!(text.body, "Blocked logins get 429.");
        let input = branch.read("input");
        assert!(
            input.contains("Retry after\n\nBlocked logins wait."),
            "{input}"
        );
        assert!(input.contains("b.txt"), "{input}");
        assert!(input.contains("+retry"), "{input}");
        let args = branch.read("args");
        let args: Vec<&str> = args.lines().collect();
        assert_eq!(args[0], "--from-settings");
        for flag in [
            "--print",
            "--no-session",
            "--no-tools",
            "--no-extensions",
            "--no-mcp",
            "--no-skills",
            "--no-prompt-templates",
            "--no-themes",
            "--no-context-files",
            "--no-approve",
            "--offline",
        ] {
            assert!(args.contains(&flag), "{flag} missing from {args:?}");
        }
        let after = |flag: &str| args[args.iter().position(|arg| *arg == flag).unwrap() + 1];
        assert_eq!(after("--provider"), "anthropic");
        assert_eq!(after("--model"), "claude-test");
        assert_eq!(after("--thinking"), "low");
        // pi runs in an empty folder of its own, never in the project, which it then removes.
        let cwd = PathBuf::from(branch.read("cwd").trim_end());
        assert_ne!(
            cwd.canonicalize().ok(),
            branch.repo.path().canonicalize().ok()
        );
        assert_eq!(branch.read("listing"), "");
        assert!(!cwd.exists());
    });
}

#[test]
fn a_long_patch_reaches_the_model_cut_at_64_kib_with_a_note() {
    run(async {
        let long = "x".repeat(99) + "\n";
        let branch = Branch::new("long", "big.txt", &long.repeat(2000));
        let checkpoints = Checkpoints::new(branch.repo.path());

        branch.write(&checkpoints).await.unwrap();

        let input = branch.read("input");
        assert!(input.contains("big.txt"), "{input}");
        assert!(input.contains("cut at 64 KiB"), "the cut is not marked");
        let patch = input.matches(&format!("+{}", "x".repeat(99))).count();
        assert!((600..=660).contains(&patch), "{patch} patch lines");
    });
}

#[test]
fn a_failing_or_empty_answer_writes_no_text() {
    run(async {
        let branch = Branch::new("fails", "b.txt", "retry\n");
        let checkpoints = Checkpoints::new(branch.repo.path());

        branch.answer("No API key for anthropic.", 1);
        assert!(branch.write(&checkpoints).await.is_err());

        branch.answer("\n  \n", 0);
        assert!(branch.write(&checkpoints).await.is_err());
    });
}

#[test]
fn commits_that_are_not_full_ids_never_reach_git_or_pi() {
    run(async {
        let branch = Branch::new("ids", "b.txt", "retry\n");
        let checkpoints = Checkpoints::new(branch.repo.path());

        for (base, head) in [
            ("main", branch.head.as_str()),
            (branch.base.as_str(), "--output=/tmp/x"),
        ] {
            let result = checkpoints
                .write_pull_request_text(&branch.pi.join("pi"), &[], MODEL, base, head)
                .await;
            assert!(result.is_err());
        }
        let result = checkpoints
            .write_pull_request_text(
                &branch.pi.join("pi"),
                &[],
                "-x/claude",
                &branch.base,
                &branch.head,
            )
            .await;
        assert!(result.is_err());
        assert_eq!(branch.read("args"), "");
    });
}

#[test]
fn a_model_that_does_not_answer_in_time_is_stopped() {
    run(async {
        let branch = Branch::new("slow", "b.txt", "retry\n");
        let mut checkpoints = Checkpoints::new(branch.repo.path());
        checkpoints.set_text_timeout(Duration::from_millis(300));
        let dir = shell_word(branch.pi.display());
        std::fs::write(
            branch.pi.join("pi"),
            format!("#!/bin/sh\npwd > {dir}/cwd\nsleep 5\necho late\n"),
        )
        .unwrap();

        let started = std::time::Instant::now();
        let result = branch.write(&checkpoints).await;

        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
        let cwd = PathBuf::from(branch.read("cwd").trim_end());
        assert!(!cwd.exists(), "the stopped pi's folder stays");
    });
}

#[test]
fn a_caller_that_stops_waiting_lets_pi_end_by_itself() {
    run(async {
        let branch = Branch::new("stopped", "b.txt", "retry\n");
        let checkpoints = Checkpoints::new(branch.repo.path());
        let dir = shell_word(branch.pi.display());
        // Killed midway, pi could leave its login half stored.
        std::fs::write(
            branch.pi.join("pi"),
            format!("#!/bin/sh\npwd > {dir}/cwd\nsleep 1\n: > {dir}/ended\necho Title\n"),
        )
        .unwrap();

        let stopped =
            tokio::time::timeout(Duration::from_millis(300), branch.write(&checkpoints)).await;
        assert!(stopped.is_err());
        let ended = branch.pi.join("ended");
        let waited = std::time::Instant::now();
        while !ended.exists() && waited.elapsed() < Duration::from_secs(10) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        assert!(ended.exists(), "pi was killed");
        let cwd = PathBuf::from(branch.read("cwd").trim_end());
        while cwd.exists() && waited.elapsed() < Duration::from_secs(10) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!cwd.exists(), "pi's folder stays");
    });
}

#[test]
fn pis_own_error_survives_input_it_never_read() {
    run(async {
        let long = "x".repeat(99) + "\n";
        let branch = Branch::new("early", "big.txt", &long.repeat(2000));
        let checkpoints = Checkpoints::new(branch.repo.path());
        std::fs::write(
            branch.pi.join("pi"),
            "#!/bin/sh\necho 'No API key for anthropic.' >&2\nexit 1\n",
        )
        .unwrap();

        let error = branch.write(&checkpoints).await.unwrap_err();

        assert!(error.contains("No API key for anthropic."), "{error}");
    });
}

#[test]
fn reading_signed_commits_runs_no_signature_program() {
    run(async {
        let branch = Branch::new("signed", "b.txt", "retry\n");
        let verifier = branch.pi.join("gpg");
        let ran = branch.pi.join("verified");
        std::fs::write(
            &verifier,
            format!("#!/bin/sh\n: > {}\n", shell_word(ran.display())),
        )
        .unwrap();
        std::fs::set_permissions(&verifier, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A signed commit on the branch, whose log the project asks Git to verify.
        let tree = branch.repo.git(&["rev-parse", "HEAD^{tree}"]);
        let signed = format!(
            "tree {}\nparent {}\nauthor T <t@example.com> 0 +0000\ncommitter T <t@example.com> 0 +0000\ngpgsig -----BEGIN PGP SIGNATURE-----\n \n -----END PGP SIGNATURE-----\n\nSigned retry\n",
            tree.trim(),
            branch.head
        );
        let object = branch.repo.path().join("signed-commit");
        std::fs::write(&object, signed).unwrap();
        let head = branch
            .repo
            .git(&["hash-object", "-t", "commit", "-w", "signed-commit"]);
        std::fs::remove_file(object).unwrap();
        branch.repo.git(&["config", "log.showSignature", "true"]);
        branch
            .repo
            .git(&["config", "gpg.program", verifier.to_str().unwrap()]);
        let checkpoints = Checkpoints::new(branch.repo.path());

        checkpoints
            .write_pull_request_text(&branch.pi.join("pi"), &[], MODEL, &branch.base, head.trim())
            .await
            .unwrap();

        assert!(branch.read("input").contains("Signed retry"));
        assert!(!ran.exists(), "Git ran the project's signature program");
    });
}

#[test]
fn before_a_commit_the_model_reads_the_commits_the_base_lacks_and_the_selected_files() {
    run(async {
        let branch = Branch::new("draft", "b.txt", "retry\n");
        branch
            .repo
            .git(&["remote", "add", "origin", "/nonexistent/origin.git"]);
        branch
            .repo
            .git(&["update-ref", "refs/remotes/origin/main", &branch.base]);
        branch.repo.write("a.txt", "changed\n");
        branch.repo.write("c.txt", "selected new file\n");
        branch.repo.write("d.txt", "left out\n");
        let checkpoints = Checkpoints::new(branch.repo.path());
        let snapshot = checkpoints.commit_plan(&[]).await.unwrap().snapshot;
        // The model reads the files as the commit takes them, not as they are later.
        branch
            .repo
            .write("c.txt", "changed after the dialog opened\n");
        let selected = ["a.txt".to_owned(), "c.txt".to_owned()];

        let text = draft(&branch, &checkpoints, Some("main"), &snapshot, &selected)
            .await
            .unwrap();

        assert_eq!(text.title, "Return 429 with Retry-After");
        assert_eq!(text.commits, 1);
        let input = branch.read("input");
        for part in ["Retry after", "+retry", "+changed", "+selected new file"] {
            assert!(input.contains(part), "{part} missing from {input}");
        }
        for part in ["left out", "after the dialog opened"] {
            assert!(!input.contains(part), "{part} in {input}");
        }

        // Without the base on the remote and without an upstream, only the files are read.
        let text = draft(&branch, &checkpoints, Some("develop"), &snapshot, &selected)
            .await
            .unwrap();
        assert_eq!(text.commits, 0);
        let input = branch.read("input");
        assert!(!input.contains("Retry after"), "{input}");
        assert!(input.contains("+selected new file"), "{input}");

        // With neither commits nor files there is nothing to describe, and pi is not asked:
        // without a base, or with one the branch has no commits beyond.
        std::fs::remove_file(branch.pi.join("args")).unwrap();
        branch
            .repo
            .git(&["update-ref", "refs/remotes/origin/caught-up", &branch.head]);
        for base in ["develop", "caught-up"] {
            assert!(
                draft(&branch, &checkpoints, Some(base), &snapshot, &[])
                    .await
                    .is_none(),
                "{base}"
            );
        }
        assert_eq!(branch.read("args"), "");
    });
}

/// The draft text for `selected` files of `snapshot` with `base`; none when pi was not asked.
async fn draft(
    branch: &Branch,
    checkpoints: &Checkpoints,
    base: Option<&str>,
    snapshot: &str,
    selected: &[String],
) -> Option<PullRequestText> {
    checkpoints
        .write_pull_request_draft_text(&branch.pi.join("pi"), &[], MODEL, base, snapshot, selected)
        .await
        .unwrap()
}

#[test]
fn without_the_base_the_model_reads_the_commits_the_upstream_lacks() {
    run(async {
        let branch = Branch::new("upstream", "b.txt", "retry\n");
        branch
            .repo
            .git(&["remote", "add", "origin", "/nonexistent/origin.git"]);
        branch
            .repo
            .git(&["update-ref", "refs/remotes/origin/feature", &branch.base]);
        branch
            .repo
            .git(&["branch", "--set-upstream-to=origin/feature"]);
        let checkpoints = Checkpoints::new(branch.repo.path());

        let text = draft(&branch, &checkpoints, None, "", &[]).await.unwrap();

        assert_eq!(text.commits, 1);
        assert!(branch.read("input").contains("Retry after"));
    });
}

#[test]
fn a_folder_that_replaced_a_selected_file_stays_with_the_files_left_out() {
    run(async {
        let branch = Branch::new("folder", "foo", "old file\n");
        std::fs::remove_file(branch.repo.path().join("foo")).unwrap();
        std::fs::create_dir(branch.repo.path().join("foo")).unwrap();
        branch.repo.write("foo/bar", "not selected\n");
        let checkpoints = Checkpoints::new(branch.repo.path());
        let snapshot = checkpoints.commit_plan(&[]).await.unwrap().snapshot;

        draft(&branch, &checkpoints, None, &snapshot, &["foo".to_owned()])
            .await
            .unwrap();

        let input = branch.read("input");
        assert!(input.contains("-old file"), "{input}");
        assert!(!input.contains("not selected"), "{input}");
    });
}

#[test]
fn a_selection_longer_than_one_diff_reaches_the_model() {
    run(async {
        let branch = Branch::new("many", "b.txt", "retry\n");
        let selected: Vec<String> = (0..150).map(|n| format!("f{n:03}.txt")).collect();
        for (n, path) in selected.iter().enumerate() {
            branch.repo.write(path, &format!("file {n}\n"));
        }
        let checkpoints = Checkpoints::new(branch.repo.path());
        let snapshot = checkpoints.commit_plan(&[]).await.unwrap().snapshot;

        draft(&branch, &checkpoints, None, &snapshot, &selected)
            .await
            .unwrap();

        let input = branch.read("input");
        for part in ["+file 0", "+file 149", "f149.txt"] {
            assert!(input.contains(part), "{part} missing");
        }
    });
}
