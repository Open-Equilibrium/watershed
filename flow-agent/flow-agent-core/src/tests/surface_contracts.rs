use super::*;

#[test]
fn runtime_error_exit_classes_and_source_chain_are_observable() {
    let usage = RuntimeError::Usage("usage".to_owned());
    assert_eq!(usage.exit_code(), 64);
    assert!(std::error::Error::source(&usage).is_none());

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
fn suffixed_session_id_preserves_maximum_length() {
    let long = "a".repeat(128);
    let suffixed = suffixed_session_id(&long, 10_000);
    assert_eq!(suffixed.len(), 128);
    assert!(suffixed.ends_with("-10000"));
}
