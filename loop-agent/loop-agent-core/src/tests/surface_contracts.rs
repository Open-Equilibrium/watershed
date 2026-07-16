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
fn fallback_session_ids_preserve_valid_loop_id_separators() {
    assert_eq!(session_id_for_loop("foo-bar"), "foo-bar001");
    assert_eq!(session_id_for_loop("foo_bar"), "foo_bar001");
    assert_eq!(session_id_for_loop("foobar"), "foobar001");
    assert_ne!(
        session_id_for_loop("foo-bar"),
        session_id_for_loop("foo_bar")
    );

    let long = "a".repeat(128);
    let session_id = session_id_for_loop(&long);
    assert!(proto::is_valid_session_id(&session_id));
    assert!(session_id.len() <= 128);
    assert_ne!(session_id, session_id_for_loop(&format!("{long}b")));
}

#[test]
fn session_id_generation_helpers_cover_edges() {
    assert_eq!(
        session_id_for_loop("sandbox-negative-custom-word"),
        "negcustomword001"
    );
    assert_eq!(session_id_for_loop("!!!"), "session001");

    let long = "a".repeat(128);
    let suffixed = suffixed_session_id(&long, 10_000);
    assert_eq!(suffixed.len(), 128);
    assert!(suffixed.ends_with("-10000"));
}
