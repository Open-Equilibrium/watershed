use std::path::Path;
use std::time::{Duration, SystemTime};

use crate::{StoredSession, recent_sessions, session_title};

/// Writes a pi session file started in `cwd` with `lines` after its header, modified at `at`.
fn session(dir: &Path, name: &str, cwd: &str, lines: &[&str], at: u64) {
    let mut text = format!(r#"{{"type":"session","version":3,"id":"{name}","cwd":"{cwd}"}}"#);
    for line in lines {
        text.push('\n');
        text.push_str(line);
    }
    text.push('\n');
    let path = dir.join(format!("{name}.jsonl"));
    std::fs::write(&path, text).expect("session file");
    std::fs::File::options()
        .write(true)
        .open(&path)
        .and_then(|file| file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(at)))
        .expect("file time");
}

#[test]
fn lists_the_projects_recent_sessions_newest_first() {
    let dir = std::env::temp_dir().join(format!("watershed-stored-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("directory");
    let user = |text: &str| {
        format!(
            r#"{{"type":"message","id":"u1","parentId":null,"message":{{"role":"user","content":"{text}"}}}}"#
        )
    };
    session(
        &dir,
        "older",
        "/work/acme-api",
        &[&user("Fix the flaky login test\\nand more")],
        1_000,
    );
    session(
        &dir,
        "named",
        "/work/acme-api",
        &[
            &user("Add a limiter"),
            r#"{"type":"session_info","id":"i1","parentId":"u1","name":"Rate limit for /login"}"#,
        ],
        3_000,
    );
    session(
        &dir,
        "elsewhere",
        "/work/web-shop",
        &[&user("Other project")],
        4_000,
    );
    session(
        &dir,
        "current",
        "/work/acme-api",
        &[&user("Running now")],
        5_000,
    );
    std::fs::write(dir.join("notes.txt"), "not a session").expect("other file");

    let recent = recent_sessions(
        &dir,
        Path::new("/work/acme-api"),
        Some(&dir.join("current.jsonl")),
        5,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        recent.expect("listing"),
        [
            StoredSession {
                path: dir.join("named.jsonl"),
                title: "Rate limit for /login".to_owned(),
                modified_ms: 3_000_000,
            },
            StoredSession {
                path: dir.join("older.jsonl"),
                title: "Fix the flaky login test".to_owned(),
                modified_ms: 1_000_000,
            },
        ]
    );
}

#[test]
fn passes_over_a_named_pipe_in_the_session_directory() {
    let dir = std::env::temp_dir().join(format!("watershed-fifo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("directory");
    let status = std::process::Command::new("mkfifo")
        .arg(dir.join("pipe.jsonl"))
        .status()
        .expect("mkfifo runs");
    assert!(status.success());
    let (done, finished) = std::sync::mpsc::channel();
    let listed = dir.clone();
    std::thread::spawn(move || {
        let _ = done.send(
            recent_sessions(&listed, Path::new("/tmp"), None, 5).map(|sessions| sessions.len()),
        );
    });
    let listed = finished
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("a pipe must not block the listing");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(listed.expect("listed"), 0);
}

#[test]
fn lists_a_linked_session_file_but_not_a_linked_pipe() {
    let dir = std::env::temp_dir().join(format!("watershed-links-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("store")).expect("directory");
    session(
        &dir.join("store"),
        "real",
        "/work/acme-api",
        &[
            r#"{"type":"message","id":"u1","parentId":null,"message":{"role":"user","content":"Linked"}}"#,
        ],
        1_000,
    );
    std::os::unix::fs::symlink(dir.join("store/real.jsonl"), dir.join("linked.jsonl"))
        .expect("link");
    let status = std::process::Command::new("mkfifo")
        .arg(dir.join("store/pipe"))
        .status()
        .expect("mkfifo runs");
    assert!(status.success());
    std::os::unix::fs::symlink(dir.join("store/pipe"), dir.join("pipe.jsonl")).expect("link");
    let sessions = recent_sessions(&dir, Path::new("/work/acme-api"), None, 5);
    let _ = std::fs::remove_dir_all(&dir);
    let titles: Vec<_> = sessions
        .expect("listed")
        .into_iter()
        .map(|session| session.title)
        .collect();
    assert_eq!(titles, ["Linked"]);
}

#[test]
fn names_the_open_session_as_the_start_page_does() {
    let dir = std::env::temp_dir().join(format!("watershed-title-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("directory");
    let user = r#"{"type":"message","id":"u1","parentId":null,"message":{"role":"user","content":"Add a limiter\nto /login"}}"#;
    session(&dir, "prompt", "/work/acme-api", &[user], 1);
    session(
        &dir,
        "named",
        "/work/acme-api",
        &[
            user,
            r#"{"type":"session_info","id":"i1","parentId":"u1","name":"Rate limit for /login"}"#,
        ],
        1,
    );
    session(&dir, "empty", "/work/acme-api", &[], 1);
    // A prompt that starts with a blank line is named by its first words, as Liquid names it.
    let spaced = r#"{"type":"message","id":"u1","parentId":null,"message":{"role":"user","content":"\n  Fix login\nand more"}}"#;
    session(&dir, "spaced", "/work/acme-api", &[spaced], 1);
    let project = Path::new("/work/acme-api");
    let title = |name: &str| session_title(&dir.join(format!("{name}.jsonl")), project);
    assert_eq!(title("spaced").as_deref(), Some("Fix login"));

    assert_eq!(title("prompt").as_deref(), Some("Add a limiter"));
    assert_eq!(title("named").as_deref(), Some("Rate limit for /login"));
    assert_eq!(title("empty"), None, "nothing to name it by yet");
    assert_eq!(
        session_title(&dir.join("prompt.jsonl"), Path::new("/work/web-shop")),
        None,
        "a session of another project"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lists_a_session_whose_first_prompt_comes_late() {
    let dir = std::env::temp_dir().join(format!("watershed-late-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("directory");
    // A long system prompt pushes the first prompt past the part read for titles.
    let system = format!(
        r#"{{"type":"message","id":"s1","parentId":null,"message":{{"role":"system","content":"{}"}}}}"#,
        "x".repeat(300 * 1024)
    );
    session(&dir, "late", "/work/acme-api", &[&system], 1);
    let recent = recent_sessions(&dir, Path::new("/work/acme-api"), None, 5);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        recent.expect("listing"),
        [StoredSession {
            path: dir.join("late.jsonl"),
            title: "Untitled session".to_owned(),
            modified_ms: 1_000,
        }]
    );
}
