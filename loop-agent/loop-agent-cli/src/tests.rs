use super::*;
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);
static CURRENT_DIR_LOCK: Mutex<()> = Mutex::new(());

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

struct CurrentDirGuard {
    previous: PathBuf,
}

impl CurrentDirGuard {
    fn enter(path: &Path) -> Self {
        let previous = env::current_dir().expect("current dir is readable");
        env::set_current_dir(path).expect("test workspace becomes current dir");
        Self { previous }
    }
}

impl Drop for CurrentDirGuard {
    fn drop(&mut self) {
        env::set_current_dir(&self.previous).expect("current dir is restored");
    }
}

fn fixture_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join(name)
}

fn workspace_copy(fixture: &str) -> PathBuf {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let target = env::temp_dir().join(format!(
        "watershed-loop-agent-cli-unit-{}-{id}",
        std::process::id()
    ));
    if target.exists() {
        fs::remove_dir_all(&target).expect("stale temp workspace removed");
    }
    copy_fixture_workspace(&fixture_dir(fixture), &target);
    target
}

fn copy_fixture_workspace(source: &Path, target: &Path) {
    copy_dir(source, target);
    copy_workspace_config(source, target);
}

fn copy_dir(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("target directory created");
    for entry in fs::read_dir(source).expect("source directory readable") {
        let entry = entry.expect("source entry readable");
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() && entry.file_name() == ".loop" {
            continue;
        }
        if source_path.is_dir() && entry.file_name() == "out" {
            fs::create_dir_all(&target_path).expect("output directory shape copied");
            continue;
        }
        if source_path.is_dir() {
            copy_dir(&source_path, &target_path);
        } else {
            fs::copy(&source_path, &target_path).expect("fixture file copied");
        }
    }
}

fn copy_workspace_config(source: &Path, target: &Path) {
    let source_config = source.join(".loop/config.yaml");
    if !source_config.exists() {
        return;
    }
    let target_config = target.join(".loop/config.yaml");
    fs::create_dir_all(target_config.parent().expect("config path has parent"))
        .expect("workspace config directory created");
    fs::copy(source_config, target_config).expect("workspace config copied");
}

#[test]
fn cli_argument_helpers_cover_emit_tail_and_usage_edges() {
    assert_eq!(
        emit_mode(&args(&["run", "hello-loop"])).unwrap(),
        EmitMode::Human
    );
    assert_eq!(
        emit_mode(&args(&["run", "hello-loop", "--emit", "jsonl"])).unwrap(),
        EmitMode::Jsonl
    );
    assert!(matches!(
        emit_mode(&args(&["run", "hello-loop", "--emit", "human"])),
        Err(RuntimeError::Usage(message)) if message.contains("unsupported emit mode")
    ));
    assert!(matches!(
        emit_mode(&args(&["run", "hello-loop", "--bad"])),
        Err(RuntimeError::Usage(message)) if message.contains("unknown argument")
    ));

    let (emit, options) = tail_args(&args(&[
        "tail",
        "meta001",
        "--emit",
        "jsonl",
        "--no-follow",
        "--timeout-ms",
        "25",
    ]))
    .expect("tail args parse");
    assert_eq!(emit, EmitMode::Jsonl);
    assert!(!options.follow);
    assert_eq!(options.timeout, Some(Duration::from_millis(25)));
    assert!(matches!(
        tail_args(&args(&["tail", "meta001", "--emit"])),
        Err(RuntimeError::Usage(message)) if message.contains("missing value for --emit")
    ));
    assert!(matches!(
        tail_args(&args(&["tail", "meta001", "--emit", "human"])),
        Err(RuntimeError::Usage(message)) if message.contains("unsupported emit mode")
    ));
    assert!(matches!(
        tail_args(&args(&["tail", "meta001", "--timeout-ms"])),
        Err(RuntimeError::Usage(message)) if message.contains("missing value for --timeout-ms")
    ));
    assert!(matches!(
        tail_args(&args(&["tail", "meta001", "--timeout-ms", "soon"])),
        Err(RuntimeError::Usage(message)) if message.contains("invalid --timeout-ms value")
    ));
    assert!(matches!(
        tail_args(&args(&["tail", "meta001", "--bad"])),
        Err(RuntimeError::Usage(message)) if message.contains("unknown argument")
    ));

    let run_args = args(&["run", "hello-loop"]);
    assert_eq!(
        positional(&run_args, 1, "loop name").expect("loop name exists"),
        "hello-loop"
    );
    assert!(matches!(
        positional(&run_args, 2, "session_id"),
        Err(RuntimeError::Usage(message)) if message.contains("missing session_id")
    ));
    assert!(reject_extra_args(&args(&["sessions"]), 1).is_ok());
    assert!(matches!(
        reject_extra_args(&args(&["sessions", "--bad"]), 1),
        Err(RuntimeError::Usage(message)) if message.contains("unknown argument")
    ));

    assert_eq!(
        os_string_to_string(OsString::from("--version")).expect("version flag converts"),
        "--version"
    );
    assert_eq!(
        os_string_to_string(OsString::from("-V")).expect("short version flag converts"),
        "-V"
    );
    assert_eq!(
        os_string_to_string(OsString::from("run")).expect("utf-8 arg converts"),
        "run"
    );
    assert!(usage().contains("loop run <loop>"));
}

#[test]
fn dispatch_covers_successful_runtime_commands() {
    let _lock = CURRENT_DIR_LOCK.lock().expect("current dir lock acquired");
    let workspace = workspace_copy("smoke-loop");
    let _current_dir = CurrentDirGuard::enter(&workspace);

    dispatch(&args(&["run", "smoke-loop"])).expect("run dispatch succeeds");
    dispatch(&args(&["replay", "smoke001"])).expect("replay dispatch succeeds");
    dispatch(&args(&["tail", "smoke001", "--no-follow"])).expect("tail dispatch succeeds");
    dispatch(&args(&["sessions"])).expect("sessions dispatch succeeds");
}

#[test]
fn dispatch_covers_usage_errors() {
    let _lock = CURRENT_DIR_LOCK.lock().expect("current dir lock acquired");

    assert!(matches!(
        dispatch(&args(&[])),
        Err(RuntimeError::Usage(message)) if message.contains("usage: loop run <loop>")
    ));
    assert!(matches!(
        dispatch(&args(&["unknown"])),
        Err(RuntimeError::Usage(message)) if message.contains("usage: loop run <loop>")
    ));
}
