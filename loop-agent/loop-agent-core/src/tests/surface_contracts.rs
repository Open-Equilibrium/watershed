#[test]
fn runtime_error_display_source_and_exit_codes_cover_variants() {
    let io_error = RuntimeError::Io {
        path: PathBuf::from("session.jsonl"),
        source: io::Error::new(io::ErrorKind::Other, "disk full"),
    };
    assert_eq!(io_error.to_string(), "session.jsonl: disk full");
    assert_eq!(io_error.exit_code(), 65);
    assert!(std::error::Error::source(&io_error).is_some());

    let json_error = RuntimeError::from(
        serde_json::from_str::<serde_json::Value>("{").expect_err("invalid JSON"),
    );
    assert!(json_error.to_string().contains("EOF"));
    assert_eq!(json_error.exit_code(), 65);
    assert!(std::error::Error::source(&json_error).is_some());

    let registry_error = RuntimeError::from(
        core_script::load_registry_root(Path::new("missing-registry-root"))
            .expect_err("missing registry root"),
    );
    assert!(registry_error.to_string().contains("missing-registry-root"));
    assert_eq!(registry_error.exit_code(), 65);
    assert!(std::error::Error::source(&registry_error).is_some());

    let policy_error = RuntimeError::from(core_policy::PolicyCompileError::MissingLoop(
        "missing".to_owned(),
    ));
    assert_eq!(
        policy_error.to_string(),
        "policy compile references missing loop missing"
    );
    assert!(std::error::Error::source(&policy_error).is_some());

    let protocol = RuntimeError::Protocol("bad stream".to_owned());
    assert_eq!(protocol.to_string(), "bad stream");
    assert_eq!(protocol.exit_code(), 65);
    assert!(std::error::Error::source(&protocol).is_none());

    let denied = runtime_denied(
        core_policy::DenyReasonCode::WriteDenied,
        "write denied".to_owned(),
    );
    assert_eq!(denied.to_string(), "write denied");
    assert_eq!(denied.exit_code(), 65);
    assert!(std::error::Error::source(&denied).is_none());

    let active = RuntimeError::ActiveSession {
        session_id: "smoke001".to_owned(),
        lock_path: PathBuf::from(".loop/locks/smoke001.lock"),
    };
    assert!(active.to_string().contains("smoke001"));
    assert_eq!(active.exit_code(), 65);
    assert!(std::error::Error::source(&active).is_none());

    let exists = RuntimeError::SessionLogExists("smoke001".to_owned());
    assert_eq!(
        exists.to_string(),
        "session log already exists for smoke001"
    );
    assert_eq!(exists.exit_code(), 65);

    let terminal = RuntimeError::TerminalSession("smoke001".to_owned());
    assert_eq!(
        terminal.to_string(),
        "cannot resume terminal session smoke001"
    );
    assert_eq!(terminal.exit_code(), 65);

    let usage = RuntimeError::Usage("usage".to_owned());
    assert_eq!(usage.to_string(), "usage");
    assert_eq!(usage.exit_code(), 64);
}

#[test]
fn session_id_validation_uses_protocol_contract() {
    assert!(validate_session_id("hello001"));
    assert!(!validate_session_id("Hello001"));
    assert!(!validate_session_id("../hello001"));
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
    assert!(validate_session_id(&session_id));
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
