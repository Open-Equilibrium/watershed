use sha2::{Digest as _, Sha256};
use std::{
    fs,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);
static SESSION_HOME_PATH: OnceLock<PathBuf> = OnceLock::new();

pub(crate) fn fixture_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join(name)
}

#[derive(Clone)]
pub(crate) struct TempWorkspace(Arc<OwnedTempWorkspace>);

struct OwnedTempWorkspace {
    path: PathBuf,
}

impl TempWorkspace {
    pub(crate) fn fresh(prefix: &str) -> Self {
        Self::fresh_under(&std::env::temp_dir(), prefix)
    }

    fn fresh_under(parent: &Path, prefix: &str) -> Self {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let target = Self(Arc::new(OwnedTempWorkspace {
            path: parent.join(format!("{prefix}-{}-{id}", std::process::id())),
        }));
        if target.exists() {
            fs::remove_dir_all(&target).expect("stale temp workspace removed");
        }
        target
    }
}

impl Deref for TempWorkspace {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.0.path
    }
}

impl AsRef<Path> for TempWorkspace {
    fn as_ref(&self) -> &Path {
        &self.0.path
    }
}

impl Drop for OwnedTempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[allow(dead_code)]
pub(crate) fn session_home_path() -> PathBuf {
    test_session_home().clone()
}

#[allow(dead_code)]
pub(crate) fn workspace_session_dir(workspace: &Path) -> PathBuf {
    workspace_store_dir(workspace).join("sessions")
}

#[allow(dead_code)]
pub(crate) fn workspace_log_dir(workspace: &Path) -> PathBuf {
    workspace_store_dir(workspace).join("logs")
}

#[allow(dead_code)]
fn workspace_store_dir(workspace: &Path) -> PathBuf {
    let canonical = fs::canonicalize(workspace).expect("workspace canonicalizes");
    let path = stable_native_path_bytes(&canonical);
    let mut digest = Sha256::new();
    digest.update(b"watershed-flow-agent-workspace-v1\0");
    digest.update((path.len() as u64).to_le_bytes());
    digest.update(path);
    let key = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    session_home_path()
        .join("workspaces")
        .join(format!("workspace-v1-{key}"))
}

#[cfg(unix)]
fn stable_native_path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt as _;

    path.as_os_str().as_bytes().to_vec()
}

#[cfg(windows)]
#[allow(dead_code)]
fn stable_native_path_bytes(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt as _;

    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn test_session_home() -> &'static PathBuf {
    SESSION_HOME_PATH.get_or_init(|| {
        std::env::var_os("FLOW_AGENT_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| unique_session_home("process"))
    })
}

#[allow(dead_code)]
pub(crate) fn run_current_test_isolated_session_home() -> bool {
    run_current_test_isolated_session_home_with_args(&[])
}

#[allow(dead_code)]
pub(crate) fn run_current_ignored_test_isolated_session_home() -> bool {
    run_current_test_isolated_session_home_with_args(&["--ignored"])
}

fn run_current_test_isolated_session_home_with_args(extra_args: &[&str]) -> bool {
    const CHILD_ENV: &str = "WATERSHED_ISOLATED_SESSION_HOME_CHILD";

    if std::env::var_os("NEXTEST").is_some() {
        return false;
    }
    if let Some(expected_home) = std::env::var_os(CHILD_ENV) {
        assert_eq!(
            std::env::var_os("FLOW_AGENT_HOME"),
            Some(expected_home),
            "isolated test child receives its selected session home"
        );
        return false;
    }

    let parent_home = std::env::var_os("FLOW_AGENT_HOME");
    let session_home = unique_session_home("isolated");
    let test_name = super::current_test_name();
    let mut command = std::process::Command::new(
        std::env::current_exe().expect("test executable resolves for session-home isolation"),
    );
    command.args(["--exact", &test_name, "--nocapture"]);
    command.args(extra_args);
    let status = command
        .env(CHILD_ENV, &session_home)
        .env("FLOW_AGENT_HOME", &session_home)
        .status()
        .expect("isolated session-home test starts");
    assert_eq!(
        std::env::var_os("FLOW_AGENT_HOME"),
        parent_home,
        "isolated test launch preserves the parent environment"
    );
    let _ = fs::remove_dir_all(&session_home);
    assert!(status.success(), "isolated session-home test failed");
    true
}

fn unique_session_home(label: &str) -> PathBuf {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time follows the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "watershed-flow-agent-home-{label}-{}-{timestamp}-{id}",
        std::process::id()
    ))
}

pub(crate) fn workspace_copy(fixture: &str) -> TempWorkspace {
    let target = TempWorkspace::fresh("watershed-flow-agent");
    copy_fixture_workspace(&fixture_dir(fixture), &target);
    target
}

#[allow(dead_code)]
pub(crate) fn empty_workspace() -> TempWorkspace {
    let target = TempWorkspace::fresh("watershed-flow-agent-empty");
    fs::create_dir(&target).expect("empty temp workspace created");
    target
}

#[allow(dead_code)]
pub(crate) fn empty_workspace_under(parent: &Path) -> TempWorkspace {
    let target = TempWorkspace::fresh_under(parent, "watershed-flow-agent-empty");
    fs::create_dir(&target).expect("empty temp workspace created");
    target
}

#[allow(dead_code)]
pub(crate) fn expected_stream(fixture: &str, stream: &str) -> String {
    fs::read_to_string(fixture_dir(fixture).join("expected").join(stream))
        .expect("expected stream is readable")
}

#[allow(dead_code)]
pub(crate) fn stream_prefix(stream: &str, line_count: usize) -> String {
    stream
        .lines()
        .take(line_count)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn copy_fixture_workspace(source: &Path, target: &Path) {
    copy_dir(&source.join("registry"), &target.join("registry"));
    let source_config = source.join(".flow/config.yaml");
    if source_config.exists() {
        let target_config = target.join(".flow/config.yaml");
        fs::create_dir_all(target_config.parent().expect("config path has parent"))
            .expect("workspace config directory created");
        fs::copy(source_config, target_config).expect("workspace config copied");
    }
    if source.join("out").is_dir() {
        fs::create_dir_all(target.join("out")).expect("output directory shape copied");
    }
}

pub(crate) fn copy_dir(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("target directory created");
    for entry in fs::read_dir(source).expect("source directory readable") {
        let entry = entry.expect("source entry readable");
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir(&source_path, &target_path);
        } else {
            fs::copy(source_path, target_path).expect("fixture file copied");
        }
    }
}
