use crate::runtime::{
    execution_plan::{FlowExecutionOptions, ToolSideEffectMode},
    fs_guards::path_io_error,
    types::{EventClock, RuntimeError},
    validate::{SessionAppendValidationState, validate_session_log_text},
};
use proto::{EventEnvelope, EventType};
use std::{fs, io::Write, path::Path};

#[test]
fn replay_segment_cleanup_preserves_unrelated_entries() {
    let workspace = super::test_support::TempWorkspace::fresh("replay-replacement");
    let run = workspace.join("run");
    fs::create_dir_all(&run).expect("replay run directory is created");
    fs::write(run.join("events.jsonl"), b"first").expect("first segment is written");
    fs::write(run.join("events.000005.jsonl"), b"stale").expect("stale segment is written");
    fs::write(run.join("unrelated"), b"keep").expect("unrelated fixture is written");

    super::test_support::remove_replay_segments(&run);

    assert!(!run.join("events.jsonl").exists());
    assert!(!run.join("events.000005.jsonl").exists());
    assert_eq!(
        fs::read(run.join("unrelated")).expect("unrelated fixture remains readable"),
        b"keep"
    );
}

#[test]
fn session_home_setup_preserves_the_environment_and_live_home() {
    use std::sync::Barrier;

    if super::test_support::run_current_test_isolated_session_home() {
        return;
    }

    let parent_home = std::env::var_os("FLOW_AGENT_HOME");
    let first = super::test_support::TempWorkspace::fresh("watershed-session-home-owner");
    let session_home = super::test_support::session_home_path();
    fs::create_dir_all(&session_home).expect("test session home is created");
    let marker = session_home.join("live-home-marker");
    fs::write(&marker, b"live").expect("live home marker is written");
    drop(first);

    let barrier = std::sync::Arc::new(Barrier::new(3));
    let workers = (0..2)
        .map(|index| {
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                super::test_support::TempWorkspace::fresh(&format!(
                    "watershed-session-home-handoff-{index}"
                ))
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let replacements = workers
        .into_iter()
        .map(|worker| worker.join().expect("home handoff worker joins"))
        .collect::<Vec<_>>();

    assert_eq!(
        std::env::var_os("FLOW_AGENT_HOME"),
        parent_home,
        "session-home setup must not mutate the parent test environment"
    );
    assert!(marker.is_file(), "a live session home cannot be deleted");
    drop(replacements);
}

pub(crate) fn run_isolated_test(child_env: &str) -> bool {
    if std::env::var_os(child_env).is_some() || std::env::var_os("NEXTEST").is_some() {
        return false;
    }
    let test_name = super::test_support::current_test_name();
    let status =
        std::process::Command::new(std::env::current_exe().expect("core test executable resolves"))
            .args(["--exact", &test_name, "--nocapture"])
            .env(child_env, "1")
            .status()
            .expect("isolated core test starts");
    assert!(status.success(), "isolated core test failed");
    true
}

impl FlowExecutionOptions {
    pub(super) fn new(clock: EventClock, side_effect_mode: ToolSideEffectMode) -> Self {
        Self::with_stub_model_fixture_profile(clock, side_effect_mode, true)
    }
}

pub(super) fn validate_appended_session_log_text(
    path: &Path,
    expected_session_id: &str,
    prior_events: &[EventEnvelope],
    text: &str,
) -> Result<Vec<EventEnvelope>, RuntimeError> {
    if prior_events.is_empty() {
        return validate_session_log_text(path, expected_session_id, text);
    }
    SessionAppendValidationState::from_prior_events(path, expected_session_id, prior_events)?
        .validate_appended(path, text)
}

pub(super) fn append_session_log_line(path: &Path, line: &str) -> Result<(), RuntimeError> {
    fs::OpenOptions::new()
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(line.as_bytes()))
        .map_err(|source| path_io_error(path, source))
}

pub(super) fn event_timestamp(sequence: u64) -> String {
    EventClock::fixed_fixture()
        .timestamp(sequence)
        .expect("fixture timestamp is valid")
}

pub(super) fn write_registry_definition(workspace: &Path, kind: &str, id: &str, source: &str) {
    let relative = Path::new("registry").join(kind).join(format!("{id}.yaml"));
    for root in [
        workspace.to_path_buf(),
        super::test_support::session_home_path(),
    ] {
        let path = root.join(&relative);
        if path.parent().is_some_and(Path::is_dir) {
            fs::write(&path, source)
                .unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
        }
    }
}

pub(super) fn completed_phase_result<'a>(
    events: &'a [EventEnvelope],
    phase_id: &str,
) -> &'a serde_json::Value {
    &events
        .iter()
        .find(|event| {
            event.event_type == EventType::PhaseCompleted && event.payload["phase_id"] == phase_id
        })
        .unwrap_or_else(|| panic!("Phase {phase_id} completes"))
        .payload["result"]
}

pub(super) fn assert_denied(
    err: RuntimeError,
    reason: core_policy::DenyReasonCode,
    message_fragment: &str,
) {
    match err {
        RuntimeError::Denied {
            reason: actual,
            message,
        } => {
            assert_eq!(actual, reason);
            assert!(
                message.contains(message_fragment),
                "{message:?} did not contain {message_fragment:?}"
            );
        }
        other => panic!("expected {reason:?} denial, got {other:?}"),
    }
}

pub(super) fn assert_active_session(err: RuntimeError, session_id: &str, lock_name: &str) {
    let message = err.to_string();
    match err {
        RuntimeError::ActiveSession {
            session_id: actual,
            lock_path,
        } => {
            assert_eq!(actual, session_id);
            assert!(
                lock_path.ends_with(lock_name),
                "{} did not end with {lock_name}",
                lock_path.display()
            );
            assert!(message.contains("already active"));
            assert!(message.contains("host-local ownership lease"));
        }
        other => panic!("expected active session error, got {other:?}"),
    }
}
