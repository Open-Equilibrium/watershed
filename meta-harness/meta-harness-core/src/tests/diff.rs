use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::diff::WorkingTree;
use crate::{ChangeStatus, DiffError, FileChange, Whitespace};

/// A throwaway Git repository that ignores the user's Git configuration.
struct Repo(PathBuf);

impl Repo {
    fn new(name: &str) -> Self {
        Self::at(std::env::temp_dir().join(format!("watershed-diff-{name}-{}", std::process::id())))
    }

    fn at(dir: PathBuf) -> Self {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("repository directory");
        let repo = Self(dir);
        repo.git(&["init", "-q"]);
        repo
    }

    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(&self.0)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
            .expect("git runs");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("UTF-8 output")
    }

    fn write(&self, path: &str, text: &str) {
        std::fs::write(self.0.join(path), text).expect("file written");
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run<F: std::future::Future>(test: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime starts")
        .block_on(test)
}

#[test]
fn an_unrepresentable_repository_path_never_selects_its_replacement_sibling() {
    use std::io::Write;

    run(async {
        let parent = Repo::new("root-bytes");
        let sibling = Repo::at(parent.path().join("repo-\u{fffd}"));
        sibling.git(&["symbolic-ref", "HEAD", "refs/heads/sibling"]);
        let mut config = std::fs::OpenOptions::new()
            .append(true)
            .open(parent.path().join(".git/config"))
            .expect("fixture config");
        writeln!(config, "\n[core]").expect("core section");
        write!(config, "worktree = {}/repo-", parent.path().display()).expect("worktree prefix");
        config.write_all(b"\xff\n").expect("raw worktree path");

        let opened = WorkingTree::open(parent.path()).await;

        match opened {
            Err(DiffError::Git(reason)) => assert!(reason.contains("UTF-8"), "{reason}"),
            Err(error) => panic!("unexpected error: {error}"),
            Ok(_) => panic!("opened an unrepresentable repository path"),
        }
        assert_eq!(
            sibling.git(&["symbolic-ref", "HEAD"]),
            "refs/heads/sibling\n"
        );
    });
}

#[test]
fn an_undecodable_working_prefix_never_selects_its_replacement_sibling() {
    const CHILD: &str = "WATERSHED_PREFIX_BYTES_TEST";
    if std::env::var_os(CHILD).is_none() {
        use std::os::unix::fs::PermissionsExt;

        let driver = Repo::new("prefix-driver");
        let host_git = Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .expect("Git path");
        assert!(host_git.status.success());
        let host_git = String::from_utf8(host_git.stdout).expect("Git path text");
        // This filesystem rejects invalid pathname bytes. Supply the exact prefix Git
        // reports on hosts that can hold them, delegating every other command to real Git.
        let wrapper = driver.path().join("git");
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nfor arg do\n  if [ \"$arg\" = --show-prefix ]; then printf 'folder-\\377/\\n'; exit 0; fi\ndone\nexec {} \"$@\"\n",
                super::shell_word(host_git.trim())
            ),
        ).expect("Git wrapper");
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))
            .expect("executable");
        let path = std::env::join_paths(std::iter::once(driver.path().to_owned()).chain(
            std::env::split_paths(&std::env::var_os("PATH").expect("PATH")),
        ))
        .expect("test PATH");
        let output = Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "tests::diff::an_undecodable_working_prefix_never_selects_its_replacement_sibling",
            ])
            .env(CHILD, "1")
            .env("PATH", path)
            .output()
            .expect("isolated test");
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    run(async {
        let repo = Repo::new("prefix-bytes");
        std::fs::create_dir(repo.path().join("folder-\u{fffd}")).expect("sibling");
        repo.write("folder-\u{fffd}/victim", "unchanged\n");
        let opened = WorkingTree::open(repo.path()).await;
        assert!(matches!(opened, Err(DiffError::Git(reason)) if reason.contains("UTF-8")));
        assert_eq!(
            std::fs::read_to_string(repo.path().join("folder-\u{fffd}/victim"))
                .expect("sibling file"),
            "unchanged\n"
        );
    });
}

#[test]
fn diff_listings_never_alias_an_undecodable_path_to_its_replacement_sibling() {
    use std::io::Write;
    use std::process::Stdio;

    run(async {
        let repo = Repo::new("diff-path-bytes");
        repo.write("base", "base\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "base"]);
        let tree = WorkingTree::open(repo.path()).await.expect("repository");
        let before = tree.snapshot().await.expect("before");
        repo.write("file-\u{fffd}", "replacement sibling\n");
        let sibling = repo.git(&["hash-object", "-w", "file-\u{fffd}"]);
        let invalid = repo.git(&["hash-object", "-w", "base"]);
        // Git trees can hold pathname bytes that this host's filesystem cannot create.
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
            .write_all(format!("100644 blob {}\tfile-\u{fffd}\0", sibling.trim()).as_bytes())
            .expect("sibling");
        input
            .write_all(format!("100644 blob {}\tfile-", invalid.trim()).as_bytes())
            .expect("invalid entry");
        input.write_all(b"\xff\0").expect("raw pathname");
        drop(input);
        let made = make_tree.wait_with_output().expect("tree");
        assert!(made.status.success());
        let id = String::from_utf8(made.stdout).expect("tree id");
        let after = tree
            .checkpointed(id.trim())
            .await
            .expect("tree lookup")
            .expect("tree exists");

        for whitespace in [Whitespace::Shown, Whitespace::Hidden] {
            let result = tree.changes(&before, &after, whitespace).await;
            assert!(
                matches!(result, Err(DiffError::Git(ref reason)) if reason.contains("UTF-8")),
                "unsupported pathname was exposed: {result:?}"
            );
        }
        let patch = tree
            .patch(&before, &after, "file-\u{fffd}", Whitespace::Shown)
            .await
            .expect("valid sibling patch");
        assert!(patch.contains("+replacement sibling\n"), "{patch}");
        assert!(!patch.contains("+base\n"), "{patch}");
    });
}

#[test]
fn diffs_a_run_against_its_snapshot_without_touching_the_projects_index() {
    run(async {
        let repo = Repo::new("run");
        repo.write("a.txt", "old\nsame\n");
        repo.write("c.txt", "gone\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "base"]);
        repo.write("pending.txt", "before the run\n");

        let status_before = repo.git(&["status", "--porcelain"]);
        let tree = WorkingTree::open(repo.path())
            .await
            .expect("repository opens");
        let index_before = std::fs::read(repo.path().join(".git/index")).expect("index");
        let snapshot = tree.snapshot().await.expect("snapshot");

        repo.write("a.txt", "new\nsame\n");
        repo.write("b.txt", "added\n");
        std::fs::remove_file(repo.path().join("c.txt")).expect("removed");

        let change = |path: &str, status, added, removed| FileChange {
            path: path.to_owned(),
            status,
            added,
            removed,
        };
        assert_eq!(
            tree.changes(
                &snapshot,
                &tree.snapshot().await.expect("snapshot"),
                Whitespace::Shown
            )
            .await
            .expect("changes"),
            [
                change("a.txt", ChangeStatus::Modified, 1, 1),
                change("b.txt", ChangeStatus::Added, 1, 0),
                change("c.txt", ChangeStatus::Deleted, 0, 1),
            ]
        );
        let patch = tree
            .patch(
                &snapshot,
                &tree.snapshot().await.expect("snapshot"),
                "a.txt",
                Whitespace::Shown,
            )
            .await
            .expect("patch");
        assert!(
            patch.contains("-old\n") && patch.contains("+new\n"),
            "{patch}"
        );
        assert!(
            !patch.contains("pending"),
            "changes before the run stay out of the diff"
        );

        assert_eq!(
            std::fs::read(repo.path().join(".git/index")).expect("index"),
            index_before
        );
        assert_ne!(
            repo.git(&["status", "--porcelain"]),
            status_before,
            "the worktree changed"
        );
        assert!(
            repo.git(&["status", "--porcelain"]).contains("?? b.txt"),
            "b.txt stays untracked"
        );
    });
}

#[test]
fn rejects_a_directory_outside_git() {
    run(async {
        let dir = std::env::temp_dir().join(format!("watershed-no-git-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("directory");
        let outside = WorkingTree::open(&dir).await;
        let _ = std::fs::remove_dir_all(&dir);
        assert!(outside.is_err());
    });
}

#[test]
fn keeps_snapshots_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        let repo = Repo::new("private");
        repo.write("a.txt", "a\n");
        let tree = WorkingTree::open(repo.path())
            .await
            .expect("repository opens");
        tree.snapshot().await.expect("snapshot");
        // Other tests' stores may come and go meanwhile; this tree's store stays until the end.
        let prefix = format!("watershed-snapshots-{}-", std::process::id());
        let modes: Vec<u32> = std::fs::read_dir(std::env::temp_dir())
            .expect("temporary directory")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
            .filter_map(|entry| entry.metadata().ok())
            .map(|metadata| metadata.permissions().mode() & 0o777)
            .collect();
        assert!(!modes.is_empty(), "the store exists");
        assert!(modes.iter().all(|&mode| mode == 0o700), "{modes:?}");
    });
}

#[test]
fn snapshots_without_running_the_repositorys_hooks() {
    use std::os::unix::fs::PermissionsExt;
    run(async {
        let repo = Repo::new("hooks");
        repo.write("a.txt", "a\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "base"]);
        let hook = repo.path().join(".git/hooks/post-index-change");
        std::fs::write(&hook, "#!/bin/sh\ntouch hook-ran\n").expect("hook");
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).expect("mode");
        repo.write("a.txt", "changed\n");
        let tree = WorkingTree::open(repo.path())
            .await
            .expect("repository opens");
        tree.snapshot().await.expect("snapshot");
        assert!(
            !repo.path().join("hook-ran").exists(),
            "a snapshot leaves the project alone"
        );
    });
}

#[test]
fn shows_a_patch_without_running_the_projects_textconv() {
    run(async {
        let repo = Repo::new("textconv");
        repo.write("a.txt", "a\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "base"]);
        let ran = repo.path().join("ran");
        repo.git(&[
            "config",
            "diff.evil.textconv",
            &format!("touch {}; cat", super::shell_word(ran.display())),
        ]);
        repo.write(".gitattributes", "*.txt diff=evil\n");
        let tree = WorkingTree::open(repo.path())
            .await
            .expect("repository opens");
        let before = tree.snapshot().await.expect("snapshot");
        repo.write("a.txt", "changed\n");
        let patch = tree
            .patch(
                &before,
                &tree.snapshot().await.expect("snapshot"),
                "a.txt",
                Whitespace::Shown,
            )
            .await
            .expect("patch");
        assert!(patch.contains("+changed"), "{patch}");
        assert!(!ran.exists(), "the project's textconv did not run");
        repo.git(&["diff", "--", "a.txt"]);
        assert!(ran.exists(), "the textconv runs where Git uses it");
    });
}

#[test]
fn diffs_a_project_given_by_a_relative_path() {
    run(async {
        // Below the working directory, so the path stays relative wherever Git resolves it.
        let relative = PathBuf::from(format!("watershed-diff-relative-{}", std::process::id()));
        let repo = Repo::at(
            std::env::current_dir()
                .expect("working directory")
                .join(&relative),
        );
        repo.write("a.txt", "old\n");
        repo.write("b.txt", "kept\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "base"]);
        // Clean index entries, so the snapshot reuses the repository's objects for b.txt.
        let past = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        for name in ["a.txt", "b.txt"] {
            std::fs::File::options()
                .write(true)
                .open(repo.path().join(name))
                .and_then(|file| file.set_modified(past))
                .expect("file time");
        }
        repo.git(&["update-index", "--refresh"]);
        let tree = WorkingTree::open(&relative)
            .await
            .expect("repository opens");
        let snapshot = tree.snapshot().await.expect("snapshot");
        repo.write("a.txt", "new\n");
        assert_eq!(
            tree.changes(
                &snapshot,
                &tree.snapshot().await.expect("snapshot"),
                Whitespace::Shown
            )
            .await
            .expect("changes"),
            [FileChange {
                path: "a.txt".to_owned(),
                status: ChangeStatus::Modified,
                added: 1,
                removed: 1,
            }]
        );
    });
}

#[test]
fn snapshots_a_project_whose_path_holds_gits_list_separator_and_quotes() {
    run(async {
        let repo = Repo::new("colon:\"quoted\\");
        repo.write("a.txt", "old\n");
        repo.write("b.txt", "kept\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "base"]);
        // Clean index entries, so the snapshot reuses the repository's objects for b.txt.
        let past = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        for name in ["a.txt", "b.txt"] {
            std::fs::File::options()
                .write(true)
                .open(repo.path().join(name))
                .and_then(|file| file.set_modified(past))
                .expect("file time");
        }
        repo.git(&["update-index", "--refresh"]);
        let tree = WorkingTree::open(repo.path())
            .await
            .expect("repository opens");
        let snapshot = tree.snapshot().await.expect("snapshot");
        repo.write("a.txt", "new\n");
        assert_eq!(
            tree.changes(
                &snapshot,
                &tree.snapshot().await.expect("snapshot"),
                Whitespace::Shown
            )
            .await
            .expect("changes"),
            [FileChange {
                path: "a.txt".to_owned(),
                status: ChangeStatus::Modified,
                added: 1,
                removed: 1,
            }]
        );
    });
}

#[test]
fn reads_the_patch_of_a_file_whose_name_looks_like_a_pattern() {
    run(async {
        let repo = Repo::new("pathspec");
        repo.write("a.txt", "one\n");
        repo.write("a*.txt", "one\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "base"]);
        let tree = WorkingTree::open(repo.path())
            .await
            .expect("repository opens");
        let snapshot = tree.snapshot().await.expect("snapshot");
        repo.write("a.txt", "plain\n");
        repo.write("a*.txt", "starred\n");
        let patch = tree
            .patch(
                &snapshot,
                &tree.snapshot().await.expect("snapshot"),
                "a*.txt",
                Whitespace::Shown,
            )
            .await
            .expect("patch");
        assert!(patch.contains("+starred"), "{patch}");
        assert!(!patch.contains("+plain"), "only the named file: {patch}");
    });
}

#[test]
fn reports_a_diff_too_large_to_read_instead_of_cutting_it() {
    run(async {
        let repo = Repo::new("large");
        repo.write("big.txt", "");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "base"]);
        let tree = WorkingTree::open(repo.path())
            .await
            .expect("repository opens");
        let snapshot = tree.snapshot().await.expect("snapshot");
        // More than the 4 MiB Git output limit.
        repo.write("big.txt", &"a line of changed text\n".repeat(200_000));
        assert!(matches!(
            tree.patch(
                &snapshot,
                &tree.snapshot().await.expect("snapshot"),
                "big.txt",
                Whitespace::Shown,
            )
            .await,
            Err(DiffError::TooLarge)
        ));
    });
}

#[test]
fn snapshots_a_project_whose_git_warns_a_lot() {
    run(async {
        let repo = Repo::new("warnings");
        repo.git(&["config", "core.autocrlf", "true"]);
        // Each file makes `git add` warn about line endings, far more than a pipe holds.
        for number in 0..1500 {
            repo.write(&format!("file-with-a-long-name-{number}.txt"), "line\n");
        }
        let tree = WorkingTree::open(repo.path())
            .await
            .expect("repository opens");
        let snapshot = tokio::time::timeout(Duration::from_secs(30), tree.snapshot())
            .await
            .expect("snapshot finishes")
            .expect("snapshot");
        assert_eq!(
            tree.changes(
                &snapshot,
                &tree.snapshot().await.expect("snapshot"),
                Whitespace::Shown
            )
            .await
            .expect("changes"),
            []
        );
    });
}

#[test]
fn keeps_blank_context_lines_whatever_the_projects_diff_settings() {
    run(async {
        let repo = Repo::new("blank");
        repo.git(&["config", "diff.suppressBlankEmpty", "true"]);
        repo.write("a.txt", "one\n\nthree\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "base"]);
        let tree = WorkingTree::open(repo.path())
            .await
            .expect("repository opens");
        let snapshot = tree.snapshot().await.expect("snapshot");
        repo.write("a.txt", "one\n\nTHREE\n");
        let patch = tree
            .patch(
                &snapshot,
                &tree.snapshot().await.expect("snapshot"),
                "a.txt",
                Whitespace::Shown,
            )
            .await
            .expect("patch");
        // A blank context line keeps its leading space, so Liquid reads it as context.
        assert!(patch.contains(" one\n \n-three\n"), "{patch:?}");
    });
}

#[test]
fn shows_new_commits_of_a_submodule_the_project_ignores() {
    run(async {
        let repo = Repo::new("submodule");
        let sub = Repo::at(repo.path().join("sub"));
        sub.write("f.txt", "one\n");
        sub.git(&["add", "."]);
        sub.git(&["commit", "-q", "-m", "one"]);
        repo.write(
            ".gitmodules",
            "[submodule \"sub\"]\n\tpath = sub\n\turl = ./sub\n",
        );
        repo.git(&["add", ".gitmodules", "sub"]);
        repo.git(&["config", "diff.ignoreSubmodules", "all"]);
        repo.git(&["config", "submodule.sub.ignore", "all"]);
        repo.git(&["commit", "-q", "-m", "base"]);
        let tree = WorkingTree::open(repo.path())
            .await
            .expect("repository opens");
        let before = tree.snapshot().await.expect("snapshot");

        sub.write("f.txt", "two\n");
        sub.git(&["commit", "-q", "-a", "-m", "two"]);

        let changes = tree
            .changes(
                &before,
                &tree.snapshot().await.expect("snapshot"),
                Whitespace::Shown,
            )
            .await
            .expect("changes");
        assert_eq!(
            changes
                .iter()
                .map(|change| (change.path.as_str(), change.status))
                .collect::<Vec<_>>(),
            [("sub", ChangeStatus::Modified)]
        );
    });
}

#[test]
fn skips_a_nested_repository_without_a_commit() {
    run(async {
        let repo = Repo::new("nested");
        repo.write("a.txt", "a\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "base"]);
        let tree = WorkingTree::open(repo.path())
            .await
            .expect("repository opens");
        let before = tree.snapshot().await.expect("snapshot");

        // Git cannot record a repository without a commit, so it would refuse the whole snapshot.
        let _empty = Repo::at(repo.path().join("empty"));
        repo.write("b.txt", "b\n");

        let after = tree.snapshot().await.expect("snapshot");
        assert_eq!(
            tree.changes(&before, &after, Whitespace::Shown)
                .await
                .expect("changes"),
            [FileChange {
                path: "b.txt".to_owned(),
                status: ChangeStatus::Added,
                added: 1,
                removed: 0,
            }]
        );
    });
}

#[test]
fn hides_whitespace_changes_when_asked() {
    run(async {
        let repo = Repo::new("whitespace");
        repo.write("spaced.txt", "x = 1\n");
        repo.write("code.txt", "if a {\nb\n}\n");
        repo.write("run.sh", "echo hi\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", "base"]);
        let tree = WorkingTree::open(repo.path())
            .await
            .expect("repository opens");
        let before = tree.snapshot().await.expect("snapshot");

        repo.write("spaced.txt", "x  =  1\r\n");
        // An indented line and one new line; only the new line is more than whitespace.
        repo.write("code.txt", "if a {\n    b\n}\nc\n");
        std::fs::set_permissions(
            repo.path().join("run.sh"),
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .expect("mode");

        let after = tree.snapshot().await.expect("snapshot");
        let listed = |changes: Vec<FileChange>| {
            changes
                .into_iter()
                .map(|change| (change.path, change.added, change.removed))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            listed(
                tree.changes(&before, &after, Whitespace::Shown)
                    .await
                    .expect("changes")
            ),
            [
                ("code.txt".to_owned(), 2, 1),
                ("run.sh".to_owned(), 0, 0),
                ("spaced.txt".to_owned(), 1, 1),
            ]
        );
        assert_eq!(
            listed(
                tree.changes(&before, &after, Whitespace::Hidden)
                    .await
                    .expect("changes")
            ),
            [("code.txt".to_owned(), 1, 0), ("run.sh".to_owned(), 0, 0)],
            "a file whose only change is whitespace drops out, a new mode stays"
        );
        let patch = tree
            .patch(&before, &after, "code.txt", Whitespace::Hidden)
            .await
            .expect("patch");
        assert!(patch.contains("+c\n") && !patch.contains("-b\n"), "{patch}");
    });
}

#[test]
fn the_search_for_repositories_stops_at_its_budget_and_counts_it_as_found() {
    use crate::diff::holds_repository;
    let dir = std::env::temp_dir().join(format!("watershed-budget-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for index in 0..10 {
        std::fs::create_dir_all(dir.join(format!("d{index}"))).expect("folder");
    }
    let mut enough = 100;
    assert!(!holds_repository(&dir, &mut enough), "no repository there");
    assert_eq!(enough, 90, "one per entry read");
    let mut short = 5;
    assert!(
        holds_repository(&dir, &mut short),
        "a search cut short leaves the folder alone"
    );
    assert_eq!(short, 0);
    assert!(
        holds_repository(&dir, &mut short),
        "a spent budget stays spent for the next path"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
