use std::path::Path;
use std::time::{Duration, SystemTime};

use crate::{
    History, StoredSession, recent_sessions, session_id, session_title, session_title_of,
    session_titles,
};

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

/// Makes a named pipe at `path`.
fn pipe(path: &Path) {
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("mkfifo runs");
    assert!(status.success());
}

/// Runs `read` on its own thread and fails, rather than hangs, when a pipe blocks it.
fn unblocked<T: Send + 'static>(read: impl FnOnce() -> T + Send + 'static) -> T {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(read());
    });
    finished
        .recv_timeout(Duration::from_secs(5))
        .expect("a pipe must not block the read")
}

#[test]
fn passes_over_a_named_pipe_in_the_session_directory() {
    let dir = std::env::temp_dir().join(format!("watershed-fifo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("directory");
    pipe(&dir.join("pipe.jsonl"));
    let listed = dir.clone();
    let listed = unblocked(move || {
        recent_sessions(&listed, Path::new("/tmp"), None, 5).map(|sessions| sessions.len())
    });
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(listed.expect("listed"), 0);
}

#[test]
fn refuses_a_pipe_named_as_the_session_file_without_waiting() {
    let dir = std::env::temp_dir().join(format!("watershed-named-fifo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("directory");
    let file = dir.join("pipe.jsonl");
    pipe(&file);
    let project = Path::new("/tmp");
    let read = unblocked(move || {
        (
            session_title(&file, project),
            session_id(&file, project),
            History::from_file(&file, 1024).map(|history| history.tree().nodes.len()),
        )
    });
    let _ = std::fs::remove_dir_all(&dir);
    assert!(matches!(read, (None, None, Err(_))), "{read:?}");
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
    pipe(&dir.join("store/pipe"));
    std::os::unix::fs::symlink(dir.join("store/pipe"), dir.join("pipe.jsonl")).expect("link");
    let listed = dir.clone();
    let sessions =
        unblocked(move || recent_sessions(&listed, Path::new("/work/acme-api"), None, 5));
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
    // A cleared name gives way to the first prompt, as an unnamed session does.
    session(
        &dir,
        "cleared",
        "/work/acme-api",
        &[
            user,
            r#"{"type":"session_info","id":"i1","parentId":"u1","name":""}"#,
        ],
        1,
    );
    // A prompt that starts with a blank line is named by its first words, as Liquid names it.
    let spaced = r#"{"type":"message","id":"u1","parentId":null,"message":{"role":"user","content":"\n  Fix login\nand more"}}"#;
    session(&dir, "spaced", "/work/acme-api", &[spaced], 1);
    let project = Path::new("/work/acme-api");
    let title = |name: &str| session_title(&dir.join(format!("{name}.jsonl")), project);
    assert_eq!(title("spaced").as_deref(), Some("Fix login"));

    assert_eq!(title("prompt").as_deref(), Some("Add a limiter"));
    assert_eq!(title("named").as_deref(), Some("Rate limit for /login"));
    assert_eq!(title("cleared").as_deref(), Some("Add a limiter"));
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
    let prompt = r#"{"type":"message","id":"u1","parentId":"s1","message":{"role":"user","content":"Add a limiter"}}"#;
    session(&dir, "late", "/work/acme-api", &[&system, prompt], 1);
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

#[test]
fn names_a_session_by_its_id() {
    let dir = std::env::temp_dir().join(format!("watershed-stored-id-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("directory");
    let named = r#"{"type":"session_info","id":"i1","parentId":null,"name":"Fix flaky sync test"}"#;
    session(&dir, "s2", "/work/acme-api", &[named], 1_000);
    session(&dir, "s3", "/work/web-shop", &[named], 2_000);
    session(&dir, "s4", "/work/acme-api", &[], 3_000);
    let project = Path::new("/work/acme-api");
    assert_eq!(
        session_title_of(&dir, project, "s2").as_deref(),
        Some("Fix flaky sync test")
    );
    assert_eq!(
        session_title_of(&dir, project, "s3"),
        None,
        "another project's"
    );
    assert_eq!(
        session_title_of(&dir, project, "s4").as_deref(),
        Some("Untitled session")
    );
    assert_eq!(session_title_of(&dir, project, "s9"), None);
    assert_eq!(
        session_id(&dir.join("s2.jsonl"), project).as_deref(),
        Some("s2")
    );
    assert_eq!(
        session_id(&dir.join("s3.jsonl"), project),
        None,
        "another project's"
    );
    let titles = session_titles(&dir, project);
    assert_eq!(titles.len(), 2, "{titles:?}");
    assert_eq!(titles["s2"], "Fix flaky sync test");
    assert_eq!(titles["s4"], "Untitled session");
    let _ = std::fs::remove_dir_all(&dir);
}
