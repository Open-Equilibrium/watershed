use crate::runtime::{
    execution_plan::ToolSideEffectMode,
    session_candidates::suffixed_session_id,
    types::{
        EVENT_STREAM_LIMITS, MAX_SESSION_BUNDLE_BYTES, MAX_SESSION_CONTEXT_MANIFEST_BYTES,
        MAX_SESSION_EVENT_BYTES, MAX_SESSION_METADATA_BYTES, MAX_SESSION_OBJECT_TOTAL_BYTES,
        RuntimeError,
    },
};
use std::{io, path::PathBuf};

#[test]
fn productive_event_storage_uses_the_approved_reservation_limit() {
    assert_eq!(MAX_SESSION_EVENT_BYTES, 352 * 1024 * 1024);
    assert_eq!(EVENT_STREAM_LIMITS.max_segments, 22);
    assert_eq!(
        MAX_SESSION_OBJECT_TOTAL_BYTES,
        MAX_SESSION_BUNDLE_BYTES
            - (352 * 1024 * 1024)
            - MAX_SESSION_CONTEXT_MANIFEST_BYTES
            - MAX_SESSION_METADATA_BYTES
    );
}

#[test]
fn runtime_error_exit_classes_and_source_chain_are_observable() {
    let usage = RuntimeError::Usage("usage".to_owned());
    assert_eq!(usage.exit_code(), 64);
    assert!(std::error::Error::source(&usage).is_none());

    let persisted = RuntimeError::PersistedState("persisted state".to_owned());
    assert_eq!(persisted.exit_code(), 65);
    assert_eq!(persisted.to_string(), "persisted state");
    assert!(std::error::Error::source(&persisted).is_none());

    let failure = RuntimeError::Io {
        path: PathBuf::from("session.jsonl"),
        source: io::Error::other("disk full"),
    };
    let session_failure = RuntimeError::session_failed("smoke001", failure);
    assert_eq!(session_failure.exit_code(), 65);
    assert_eq!(
        session_failure.to_string(),
        "session smoke001 failed: session.jsonl: disk full"
    );
    assert!(std::error::Error::source(&session_failure).is_some());
}

#[test]
fn runtime_error_diagnostics_cover_every_controlled_failure_shape() {
    fn failure(message: &str) -> Box<RuntimeError> {
        Box::new(RuntimeError::Protocol(message.to_owned()))
    }

    let cases = [
        (
            RuntimeError::ContextBudgetExceeded {
                input_budget_tokens: 8,
                required_bytes: 13,
            },
            "context_budget_exceeded: mandatory context is 13 canonical bytes (one estimated token per byte), input budget is 8 tokens",
            false,
        ),
        (
            RuntimeError::ReplayOutputLimitExceeded {
                limit_bytes: 64 * 1024 * 1024,
            },
            "replay_output_limit_exceeded: in-memory replay output exceeds 67108864 bytes",
            false,
        ),
        (
            RuntimeError::ExecutionBackendUnavailable,
            "execution_backend_unavailable: M1 requires the explicit stub-model fixture profile",
            false,
        ),
        (
            RuntimeError::ProductiveExecutionUnavailable,
            "execution_backend_unavailable: productive execution is unavailable on this platform",
            false,
        ),
        (
            RuntimeError::Cancelled,
            "productive execution cancelled",
            false,
        ),
        (
            RuntimeError::EventWriter(failure("write")),
            "event writer: write",
            true,
        ),
        (
            RuntimeError::EventWriterFailures(vec![failure("first"), failure("second")]),
            "event writer failures; failure 1: first; failure 2: second",
            true,
        ),
        (
            RuntimeError::TemporaryReplacementFailures {
                operation: failure("replace"),
                cleanup: failure("unlink"),
            },
            "temporary replacement operation failed: replace; temporary replacement cleanup failed: unlink",
            true,
        ),
        (
            RuntimeError::PublishedOutputCleanupFailure {
                output: PathBuf::from("out/result.txt"),
                temporary: PathBuf::from("out/result.tmp"),
                source: failure("unlink"),
            },
            "own-script output out/result.txt was published, but temporary path out/result.tmp cleanup failed: unlink",
            true,
        ),
        (
            RuntimeError::PublishedOutputFinalizationFailure {
                output: PathBuf::from("registry/tools/inspect.yaml"),
                source: failure("sync"),
            },
            "output registry/tools/inspect.yaml was published, but finalization failed: sync",
            true,
        ),
        (
            RuntimeError::PublishedCredentialFinalizationFailure {
                source: failure("secret-bearing detail"),
            },
            "credential_published_not_finalized: replacement credential was published but finalization failed",
            true,
        ),
        (
            RuntimeError::ControlledStageFailures {
                operation: Some(failure("run")),
                finalization: Some(failure("finalize")),
                cleanup: Some(failure("unlock")),
            },
            "operation failed: run; event writer finalization failed: finalize; ownership cleanup failed: unlock",
            true,
        ),
        (
            RuntimeError::ControlledStageFailures {
                operation: None,
                finalization: Some(failure("finalize")),
                cleanup: Some(failure("unlock")),
            },
            "event writer finalization failed: finalize; ownership cleanup failed: unlock",
            true,
        ),
        (
            RuntimeError::SessionCleanupFailures(vec![failure("lock"), failure("marker")]),
            "session reservation cleanup failed; cleanup failure 1: lock; cleanup failure 2: marker",
            true,
        ),
        (
            RuntimeError::ActiveSession {
                session_id: "session001".to_owned(),
                lock_path: PathBuf::from("session001.lock"),
            },
            "session session001 is already active under a host-local ownership lease; session001.lock is its non-authoritative workspace marker. Retry after the owning Flow Agent process exits.",
            false,
        ),
        (
            RuntimeError::SessionLogExists("session001".to_owned()),
            "session log already exists for session001",
            false,
        ),
        (
            RuntimeError::TerminalSession("session001".to_owned()),
            "cannot resume terminal session session001",
            false,
        ),
    ];

    for (error, expected, has_source) in cases {
        assert_eq!(error.to_string(), expected);
        assert_eq!(std::error::Error::source(&error).is_some(), has_source);
        assert_eq!(error.exit_code(), 65);
    }

    let json_error =
        serde_json::from_str::<serde_json::Value>("{").expect_err("invalid JSON creates an error");
    let json: RuntimeError = json_error.into();
    assert!(json.to_string().contains("EOF while parsing an object"));
    assert!(std::error::Error::source(&json).is_some());

    let registry: RuntimeError =
        core_script::RegistryError::InvalidBlockId("bad id".to_owned()).into();
    assert!(registry.to_string().contains("bad id"));
    assert!(std::error::Error::source(&registry).is_some());

    let policy: RuntimeError =
        core_policy::PolicyCompileError::MissingFlow("missing".to_owned()).into();
    assert_eq!(
        policy.to_string(),
        "policy compile references missing flow missing"
    );
    assert!(std::error::Error::source(&policy).is_some());

    let denied = RuntimeError::Denied {
        reason: core_policy::DenyReasonCode::ToolOutOfPhase,
        message: "Tool is unavailable in this Phase".to_owned(),
    };
    assert_eq!(denied.to_string(), "Tool is unavailable in this Phase");
    assert!(std::error::Error::source(&denied).is_none());

    let protocol = RuntimeError::Protocol("protocol invariant".to_owned());
    let usage = RuntimeError::Usage("invalid command".to_owned());
    assert_eq!(protocol.to_string(), "protocol invariant");
    assert_eq!(usage.to_string(), "invalid command");
    assert!(std::error::Error::source(&protocol).is_none());
    assert!(std::error::Error::source(&usage).is_none());
}

#[test]
fn tool_side_effect_modes_separate_planning_preflight_and_execution() {
    assert!(ToolSideEffectMode::Apply.should_execute_tool(1));
    assert!(!ToolSideEffectMode::Plan.should_execute_tool(1));
    assert!(
        !ToolSideEffectMode::PreflightResume {
            prefix_event_count: 1
        }
        .should_execute_tool(2)
    );
    assert!(
        ToolSideEffectMode::Resume {
            prefix_event_count: 1
        }
        .should_execute_tool(2)
    );
    assert!(
        !ToolSideEffectMode::Resume {
            prefix_event_count: 1
        }
        .should_execute_tool(1)
    );
    assert!(
        ToolSideEffectMode::PreflightResume {
            prefix_event_count: 1
        }
        .should_preflight_tool(2)
    );
    assert!(
        !ToolSideEffectMode::PreflightResume {
            prefix_event_count: 1
        }
        .should_preflight_tool(1)
    );
    for mode in [
        ToolSideEffectMode::Apply,
        ToolSideEffectMode::Plan,
        ToolSideEffectMode::Resume {
            prefix_event_count: 0,
        },
    ] {
        assert!(!mode.should_preflight_tool(1));
    }
}

#[test]
fn suffixed_session_id_preserves_maximum_length() {
    let long = "a".repeat(128);
    let suffixed = suffixed_session_id(&long, 10_000);
    assert_eq!(suffixed.len(), 128);
    assert!(suffixed.ends_with("-10000"));
}
