#[test]
fn file_helpers_cover_direct_edges() {
    let workspace = empty_workspace("file-and-stream-helpers");
    let missing_dir = workspace.join("missing-dir");
    assert!(!ensure_optional_real_directory(&missing_dir).expect("missing dir is optional"));

    let created_dir = workspace.join("created-dir");
    assert!(ensure_created_real_directory(&created_dir).expect("dir is created"));
    assert!(!ensure_created_real_directory(&created_dir).expect("existing dir is reused"));

    let file_path = workspace.join("file.txt");
    fs::write(&file_path, b"abc").expect("file written");
    assert!(matches!(
        ensure_new_leaf_available(&file_path),
        Err(RuntimeError::Protocol(message)) if message.contains("must not already exist")
    ));
    assert!(matches!(
        ensure_real_file(&workspace),
        Err(RuntimeError::Protocol(message)) if message.contains("must be a file")
    ));

    assert_eq!(
        read_file_range(&file_path, 1, 2).expect("range reads"),
        b"bc"
    );
    assert!(matches!(
        read_file_range(&file_path, 4, 2),
        Err(RuntimeError::Protocol(message)) if message.contains("append-only tail")
    ));
    assert!(matches!(
        read_file_range(&file_path, 0, 2),
        Err(RuntimeError::Protocol(message)) if message.contains("exceeds max")
    ));
    assert_eq!(
        read_file_suffix_to_string(&file_path, 1, 3).expect("suffix reads"),
        "bc"
    );
    assert!(matches!(
        read_file_suffix_to_string(&file_path, 3, 2),
        Err(RuntimeError::Protocol(message)) if message.contains("append-only tail")
    ));
    assert!(matches!(
        read_file_suffix_to_string(&file_path, 1, 4),
        Err(RuntimeError::Protocol(message)) if message.contains("append-only tail")
    ));

    write_existing_file(&file_path, b"rewritten").expect("existing file is rewritten");
    assert_eq!(
        fs::read_to_string(&file_path).expect("rewritten file readable"),
        "rewritten"
    );
    append_existing_file(&file_path, b"+append").expect("existing file is appended");
    assert_eq!(
        fs::read_to_string(&file_path).expect("appended file readable"),
        "rewritten+append"
    );
    append_existing_file_without_link_count(&file_path, b"+fallback")
        .expect("fallback append rewrites through temp file");
    assert_eq!(
        fs::read_to_string(&file_path).expect("fallback appended file readable"),
        "rewritten+append+fallback"
    );
    replace_existing_file_without_link_count(&file_path, b"fallback-replace")
        .expect("fallback replace rewrites through temp file");
    assert_eq!(
        fs::read_to_string(&file_path).expect("fallback replaced file readable"),
        "fallback-replace"
    );
    replace_existing_file_atomically(&file_path, b"atomic-replace")
        .expect("atomic replace succeeds");
    assert_eq!(
        fs::read_to_string(&file_path).expect("atomic replaced file readable"),
        "atomic-replace"
    );

}

#[test]
fn file_guard_and_reservation_helpers_cover_direct_edges() {
    let workspace = empty_workspace("file-guard-and-reservation-helpers");

    let missing_parent_child = workspace.join("missing-parent").join("child");
    assert!(matches!(
        ensure_created_real_directory(&missing_parent_child),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    let missing_file = workspace.join("missing-file.txt");
    assert!(matches!(
        ensure_real_file(&missing_file),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    assert!(matches!(
        ensure_non_hardlinked_real_file(&missing_file),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    let reserved_dir = workspace.join("reserved-dir.jsonl");
    fs::create_dir(&reserved_dir).expect("reserved dir created");
    assert!(matches!(
        reserve_session_file(&reserved_dir, "reserved001"),
        Err(RuntimeError::Protocol(message)) if message.contains("must be a file")
    ));
    let missing_parent_reserved = workspace.join("missing-reserved-dir").join("session.jsonl");
    assert!(matches!(
        reserve_new_file(&missing_parent_reserved),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    assert!(matches!(
        reserve_session_file(&missing_parent_reserved, "reserved002"),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    let lock_path = workspace.join("active.lock");
    fs::write(&lock_path, b"lock").expect("lock file written");
    assert_active_session(
        reserve_session_lock_file(&lock_path, "active001").expect_err("active lock must reject"),
        "active001",
        "active.lock",
    );
    let missing_parent_lock = workspace.join("missing-lock-dir").join("active.lock");
    assert!(matches!(
        reserve_session_lock_file(&missing_parent_lock, "active002"),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    assert!(is_active_session_error(
        &RuntimeError::ActiveSession {
            session_id: "active001".to_owned(),
            lock_path: lock_path.clone(),
        },
        "active001"
    ));
    assert_eq!(suffixed_session_id(&"a".repeat(140), 42).len(), 128);

    let invalid_utf8 = workspace.join("invalid-utf8.txt");
    fs::write(&invalid_utf8, [0xff]).expect("invalid utf8 written");
    assert!(matches!(
        read_to_string_with_limit(&invalid_utf8, MAX_SESSION_LOG_BYTES),
        Err(RuntimeError::Protocol(message)) if message.contains("valid UTF-8")
    ));

}

#[test]
fn tail_stream_helpers_cover_direct_edges() {
    let workspace = empty_workspace("tail-stream-helpers");
    let file_path = workspace.join("file.txt");
    fs::write(&file_path, b"abc").expect("file written");

    let transient = RuntimeError::Io {
        path: file_path.clone(),
        source: io::Error::from(io::ErrorKind::PermissionDenied),
    };
    assert!(runtime_error_is_transient_tail_read(&transient));
    let not_found = RuntimeError::Io {
        path: file_path.clone(),
        source: io::Error::from(io::ErrorKind::NotFound),
    };
    assert!(runtime_error_is_transient_tail_read(&not_found));
    let other = RuntimeError::Io {
        path: file_path.clone(),
        source: io::Error::from(io::ErrorKind::Other),
    };
    assert!(!runtime_error_is_transient_tail_read(&other));

    let mut attempts = 0usize;
    let retried = retry_tail_transient_read_error(|| {
        attempts += 1;
        if attempts == 1 {
            Err(RuntimeError::Io {
                path: file_path.clone(),
                source: io::Error::from(io::ErrorKind::NotFound),
            })
        } else {
            Ok("ok")
        }
    })
    .expect("transient tail read retries");
    assert_eq!(retried, "ok");
    assert_eq!(attempts, 2);

    assert_eq!(
        session_stream_suffix_bytes("first\nsecond\n", 0).expect("full stream suffix"),
        b"first\nsecond\n"
    );
    assert_eq!(
        session_stream_suffix_bytes("first\nsecond\n", 1).expect("one-line prefix suffix"),
        b"second\n"
    );
    assert!(matches!(
        session_stream_suffix_bytes("first", 1),
        Err(RuntimeError::Protocol(message)) if message.contains("initial event")
    ));
    assert!(matches!(
        session_stream_suffix_bytes("first\n", 2),
        Err(RuntimeError::Protocol(message)) if message.contains("persisted event prefix")
    ));

}

#[test]
fn durable_prefix_helpers_cover_direct_edges() {
    let workspace = empty_workspace("durable-prefix-helpers");

    let reservation = reserve_session_log(&workspace, "helper001").expect("session reserved");
    persist_reserved_session_prefix(&reservation, "helper001", &[base_event()], 1, None)
        .expect("single-event prefix is already durable");
    reservation.rollback();

    let loop_started = EventEnvelope {
        loop_id: Some("loop-001".to_owned()),
        ..EventEnvelope::new(
            "evt-002",
            EventType::LoopStarted,
            "meta001",
            2,
            "2026-01-01T00:00:01Z",
            "loop-agent-cli",
            serde_json::json!({"loop_definition_id":"smoke-loop"}),
        )
    };
    assert_eq!(
        durable_run_prefix_event_count(&[base_event(), loop_started]),
        2
    );
}

#[test]
fn workspace_config_helpers_reject_unsafe_registry_roots() {
    let workspace = empty_workspace("workspace-config-helpers");
    fs::create_dir_all(workspace.join(".loop")).expect("loop config dir");
    fs::create_dir(workspace.join("registry")).expect("registry dir");
    fs::write(workspace.join("registry-file"), "not a dir").expect("registry file");

    assert_eq!(
        config_value(
            "registry_root: \"registry\"\nother: ignored\n",
            "registry_root"
        ),
        Some("registry".to_owned())
    );
    assert_eq!(
        config_value(
            "registry_root: registry # fixture registry\n",
            "registry_root"
        ),
        Some("registry".to_owned())
    );
    assert_eq!(config_value("registry_root:\n", "registry_root"), None);

    fs::write(
        workspace.join(".loop/config.yaml"),
        "stub_model: deterministic\n",
    )
    .expect("config without registry root");
    assert!(matches!(
        load_workspace_config(&workspace),
        Err(RuntimeError::Usage(message)) if message.contains("missing")
    ));

    fs::write(
        workspace.join(".loop/config.yaml"),
        "registry_root: registry\n",
    )
    .expect("valid config");
    let config = load_workspace_config(&workspace).expect("config loads");
    assert_ne!(config.event_clock, EventClock::fixed_fixture());
    assert_eq!(
        registry_root_path(&workspace, &config.registry_root).expect("registry path resolves"),
        workspace.join("registry")
    );
    assert_eq!(
        registry_root_path(&workspace, Path::new("./registry"))
            .expect("curdir registry path resolves"),
        workspace.join("registry")
    );
    fs::write(
        workspace.join(".loop/config.yaml"),
        "registry_root: registry # fixture registry\n",
    )
    .expect("commented config");
    let config = load_workspace_config(&workspace).expect("commented config loads");
    assert_eq!(config.registry_root, PathBuf::from("registry"));

    fs::write(
        workspace.join(".loop/config.yaml"),
        "fixture_profile: stub-model\nregistry_root: registry\nstub_model: deterministic\n",
    )
    .expect("fixture config");
    let config = load_workspace_config(&workspace).expect("fixture config loads");
    assert_eq!(config.event_clock, EventClock::fixed_fixture());

    fs::write(
        workspace.join(".loop/config.yaml"),
        "fixture_profile: stub-model\nregistry_root: registry\n",
    )
    .expect("fixture config without stub model");
    assert!(matches!(
        load_workspace_config(&workspace),
        Err(RuntimeError::Usage(message)) if message.contains("requires stub_model")
    ));

    fs::write(
        workspace.join(".loop/config.yaml"),
        "registry_root: registry\nstub_model: deterministic\n",
    )
    .expect("stub model without fixture profile");
    assert!(matches!(
        load_workspace_config(&workspace),
        Err(RuntimeError::Usage(message)) if message.contains("requires fixture_profile")
    ));

    fs::write(
        workspace.join(".loop/config.yaml"),
        "fixture_profile: live\nregistry_root: registry\nstub_model: deterministic\n",
    )
    .expect("unsupported fixture profile");
    assert!(matches!(
        load_workspace_config(&workspace),
        Err(RuntimeError::Usage(message)) if message.contains("unsupported .loop/config.yaml fixture_profile")
    ));

    fs::write(
        workspace.join(".loop/config.yaml"),
        "fixture_profile: stub-model\nregistry_root: registry\nstub_model: live\n",
    )
    .expect("unsupported stub model");
    assert!(matches!(
        load_workspace_config(&workspace),
        Err(RuntimeError::Usage(message)) if message.contains("unsupported .loop/config.yaml stub_model")
    ));

    fs::write(
        workspace.join(".loop/config.yaml"),
        "registry_root: ../registry\n",
    )
    .expect("unsafe config");
    assert!(matches!(
        load_workspace_config(&workspace),
        Err(RuntimeError::Usage(message)) if message.contains("within the workspace")
    ));
    assert!(matches!(
        registry_root_path(&workspace, Path::new("registry-file")),
        Err(RuntimeError::Usage(message)) if message.contains("through directories")
    ));
    assert!(matches!(
        registry_root_path(&workspace, Path::new("missing-registry")),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    assert!(matches!(
        read_workspace_config_to_string(&workspace.join("missing-config.yaml")),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));

    let oversized_len =
        usize::try_from(core_script::MAX_REGISTRY_FILE_BYTES).expect("limit fits usize") + 1;
    fs::write(
        workspace.join(".loop/config.yaml"),
        format!("registry_root: registry\n{}", "x".repeat(oversized_len)),
    )
    .expect("oversized config written");
    assert!(matches!(
        load_workspace_config(&workspace),
        Err(RuntimeError::Protocol(message)) if message.contains("exceeds max")
    ));
}

#[cfg(unix)]
#[test]
fn workspace_config_rejects_symlinked_config_file() {
    use std::os::unix::fs::symlink;

    let workspace = empty_workspace("workspace-config-symlink");
    let outside = empty_workspace("outside-workspace-config");
    fs::create_dir_all(workspace.join(".loop")).expect("loop config dir");
    let outside_config = outside.join("config.yaml");
    fs::write(&outside_config, "registry_root: registry\n").expect("outside config written");
    symlink(&outside_config, workspace.join(".loop/config.yaml")).expect("config symlink");

    let err = load_workspace_config(&workspace).expect_err("config symlink must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("symlink")));
}

#[cfg(unix)]
#[test]
fn run_loop_rejects_symlinked_log_dir_without_side_effects() {
    use std::os::unix::fs::symlink;

    let workspace = workspace_copy("smoke-loop");
    let outside = empty_workspace("outside-log");
    fs::create_dir_all(workspace.join(".loop")).expect("loop dir");
    symlink(&outside, workspace.join(LOCAL_LOG_DIR)).expect("log dir symlink");

    let err = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
        .expect_err("symlinked log dir must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("symlink")));
    assert!(!outside.join("smoke001.log").exists());
    assert!(!workspace
        .join(LOCAL_SESSION_DIR)
        .join("smoke001.jsonl")
        .exists());
}

#[cfg(unix)]
#[test]
fn run_loop_rejects_symlinked_session_leaf_without_side_effects() {
    use std::os::unix::fs::symlink;

    let workspace = workspace_copy("smoke-loop");
    let outside = empty_workspace("outside-session");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let outside_target = outside.join("victim.jsonl");
    symlink(&outside_target, session_dir.join("smoke001.jsonl")).expect("session leaf symlink");

    let err = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
        .expect_err("symlinked session leaf must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("symlink")));
    assert!(!outside_target.exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).join("smoke001.log").exists());
}

#[cfg(unix)]
#[test]
fn run_loop_rejects_symlinked_summary_leaf_without_side_effects() {
    use std::os::unix::fs::symlink;

    let workspace = workspace_copy("hello-loop");
    let outside = empty_workspace("outside-summary");
    let outside_target = outside.join("summary.txt");
    fs::write(&outside_target, "outside\n").expect("outside target written");
    fs::create_dir_all(workspace.join("out")).expect("out dir");
    symlink(&outside_target, workspace.join("out/summary.txt")).expect("summary leaf symlink");

    let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect_err("symlinked summary leaf must fail");

    assert_denied(
        err,
        core_policy::DenyReasonCode::SymlinkEscapeDenied,
        "symlink",
    );
    assert_eq!(
        fs::read_to_string(&outside_target).expect("outside target readable"),
        "outside\n"
    );
    assert!(!workspace
        .join(LOCAL_SESSION_DIR)
        .join("hello001.jsonl")
        .exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
}

#[test]
fn run_loop_rejects_multi_write_own_script_before_side_effects() {
    let workspace = workspace_copy("hello-loop");
    fs::write(
        workspace.join("registry/tools/write-summary.yaml"),
        r#"tool:
  id: write-summary
  name: WriteSummary
  tool_kind: own-script
  command: script:write-summary
  script_runtime: posix-sh
  script_body: |
    printf 'partial\n' > out/partial.txt
    printf '%s\n' "$SUMMARY" > out/summary.txt
  allowed_parameters: []
  read_scope: ["workspace"]
  write_scope: ["workspace/out"]
  protected_path_grants: []
  network: deny
"#,
    )
    .expect("write-summary fixture mutated");

    let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect_err("multi-write own-script must fail before execution");

    assert!(
        matches!(err, RuntimeError::Protocol(ref message) if message.contains("multiple write operations")),
        "{err:?}"
    );
    assert!(!workspace.join("out/partial.txt").exists());
    assert!(!workspace.join("out/summary.txt").exists());
    assert!(!workspace
        .join(LOCAL_SESSION_DIR)
        .join("hello001.jsonl")
        .exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
}

#[test]
fn run_loop_commits_failure_stream_when_apply_side_effects_fail() {
    let workspace = workspace_copy("hello-loop");
    let summary_path = workspace.join("out/summary.txt");
    for attempt in 0..100 {
        let temp_path =
            replacement_temp_path(&summary_path, attempt).expect("replacement temp path is valid");
        fs::write(temp_path, b"collision").expect("replacement temp collision written");
    }

    let output = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect("apply-time side effect failure is recorded as a failed run");

    assert!(output.failed);
    assert!(
        output.stdout.contains("\"reason\":\"write_denied\""),
        "{}",
        output.stdout
    );
    assert!(!summary_path.exists());
    let events = validate_session_log_text(
        Path::new("apply-denial-temp-collision.jsonl"),
        &output.session_id,
        &output.stdout,
    )
    .expect("failed apply stream validates");
    assert!(events
        .iter()
        .any(|event| event.event_type == EventType::ToolFailed));
    assert_eq!(terminal_failure_reason(&events), Some("write_denied"));
    assert_eq!(
        fs::read_to_string(&output.session_path).expect("session log readable"),
        output.stdout
    );
    assert!(workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
}

#[cfg(unix)]
#[test]
fn run_loop_rejects_symlinked_summary_ancestor_without_side_effects() {
    use std::os::unix::fs::symlink;

    let workspace = workspace_copy("hello-loop");
    let outside = empty_workspace("outside-summary-ancestor");
    fs::remove_dir_all(workspace.join("out")).expect("fixture out directory removed");
    symlink(&outside, workspace.join("out")).expect("summary ancestor symlink");

    let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect_err("symlinked summary ancestor must fail");

    assert_denied(
        err,
        core_policy::DenyReasonCode::SymlinkEscapeDenied,
        "symlink",
    );
    assert!(!outside.join("summary.txt").exists());
    assert!(!workspace
        .join(LOCAL_SESSION_DIR)
        .join("hello001.jsonl")
        .exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
}

#[cfg(windows)]
#[test]
fn run_loop_rejects_junction_summary_ancestor_without_side_effects() {
    let workspace = workspace_copy("hello-loop");
    let outside = empty_workspace("outside-summary-junction");
    fs::remove_dir_all(workspace.join("out")).expect("fixture out directory removed");
    create_windows_junction(&workspace.join("out"), &outside);

    let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect_err("junction summary ancestor must fail");

    assert_denied(
        err,
        core_policy::DenyReasonCode::SymlinkEscapeDenied,
        "reparse",
    );
    assert!(!outside.join("summary.txt").exists());
    assert!(!workspace
        .join(LOCAL_SESSION_DIR)
        .join("hello001.jsonl")
        .exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
}

#[cfg(any(unix, windows))]
#[test]
fn run_loop_rejects_hardlinked_summary_leaf_without_side_effects() {
    let workspace = workspace_copy("hello-loop");
    let outside = empty_workspace("outside-summary-hardlink");
    let outside_target = outside.join("summary.txt");
    fs::write(&outside_target, "outside\n").expect("outside target written");
    fs::create_dir_all(workspace.join("out")).expect("out dir");
    fs::hard_link(&outside_target, workspace.join("out/summary.txt")).expect("summary hard link");

    let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect_err("hard-linked summary leaf must fail");

    assert_denied(err, core_policy::DenyReasonCode::WriteDenied, "hard-linked");
    assert_eq!(
        fs::read_to_string(&outside_target).expect("outside target readable"),
        "outside\n"
    );
    assert!(!workspace
        .join(LOCAL_SESSION_DIR)
        .join("hello001.jsonl")
        .exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
}

#[cfg(not(any(unix, windows)))]
#[test]
fn run_loop_replaces_hardlinked_summary_leaf_without_modifying_link_target_when_link_count_unverified(
) {
    let workspace = workspace_copy("hello-loop");
    fs::create_dir_all(workspace.join("out")).expect("out dir");
    let outside = empty_workspace("outside-summary-hardlink-unverified");
    let outside_target = outside.join("summary.txt");
    fs::write(&outside_target, "outside\n").expect("outside target written");
    let summary_path = workspace.join("out/summary.txt");
    fs::hard_link(&outside_target, &summary_path).expect("summary hard link");

    let output = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect("unverifiable hardlink is safely replaced");

    assert!(!output.failed);
    assert_eq!(
        fs::read_to_string(&outside_target).expect("outside target readable"),
        "outside\n"
    );
    assert_eq!(
        fs::read_to_string(&summary_path).expect("summary is replaced"),
        "hello\n"
    );
}
