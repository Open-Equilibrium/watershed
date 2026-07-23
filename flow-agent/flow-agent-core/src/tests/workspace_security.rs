#[test]
fn workspace_config_helpers_reject_unsafe_registry_roots() {
    let workspace = empty_workspace("workspace-config-helpers");
    fs::create_dir_all(workspace.join(".flow")).expect("loop config dir");
    fs::create_dir(workspace.join("registry")).expect("registry dir");
    fs::write(workspace.join("registry-file"), "not a dir").expect("registry file");

    for (label, source, expected) in [
        (
            "unknown field",
            "registry_root: registry\nother: ignored\n",
            "unknown field",
        ),
        (
            "duplicate field",
            "registry_root: registry\nregistry_root: other\n",
            "duplicate",
        ),
        (
            "explicit tag",
            "registry_root: !!str registry\n",
            "explicit YAML tag",
        ),
        (
            "multiple documents",
            "registry_root: registry\n---\nregistry_root: other\n",
            "document",
        ),
    ] {
        fs::write(workspace.join(".flow/config.yaml"), source).expect("invalid config written");
        match load_workspace_config(&workspace).expect_err("invalid config must be rejected") {
            RuntimeError::Usage(message) => {
                assert!(message.contains(expected), "{label}: {message}")
            }
            error => panic!("{label}: {error}"),
        }
    }

    fs::write(
        workspace.join(".flow/config.yaml"),
        "stub_model: deterministic\n",
    )
    .expect("config without registry root");
    assert!(matches!(
        load_workspace_config(&workspace),
        Err(RuntimeError::Usage(message)) if message.contains("missing")
    ));

    for registry_root in ["registry", "nested/registry", "répertoire/注册表"] {
        fs::write(
            workspace.join(".flow/config.yaml"),
            format!("registry_root: {registry_root}\n"),
        )
        .expect("valid config");
        let config = load_workspace_config(&workspace).expect("config loads");
        let expected = registry_root
            .split('/')
            .fold(PathBuf::new(), |mut path, component| {
                path.push(component);
                path
            });
        assert_ne!(config.event_clock, EventClock::fixed_fixture());
        assert_eq!(config.registry_root, expected, "{registry_root}");
    }
    fs::write(
        workspace.join(".flow/config.yaml"),
        "registry_root: registry # fixture registry\n",
    )
    .expect("commented config");
    let config = load_workspace_config(&workspace).expect("commented config loads");
    assert_eq!(config.registry_root, PathBuf::from("registry"));

    fs::write(
        workspace.join(".flow/config.yaml"),
        "fixture_profile: stub-model\nregistry_root: registry\nstub_model: deterministic\n",
    )
    .expect("fixture config");
    let config = load_workspace_config(&workspace).expect("fixture config loads");
    assert_eq!(config.event_clock, EventClock::fixed_fixture());

    fs::write(
        workspace.join(".flow/config.yaml"),
        "fixture_profile: stub-model\nregistry_root: registry\n",
    )
    .expect("fixture config without stub model");
    assert!(matches!(
        load_workspace_config(&workspace),
        Err(RuntimeError::Usage(message)) if message.contains("requires stub_model")
    ));

    fs::write(
        workspace.join(".flow/config.yaml"),
        "registry_root: registry\nstub_model: deterministic\n",
    )
    .expect("stub model without fixture profile");
    assert!(matches!(
        load_workspace_config(&workspace),
        Err(RuntimeError::Usage(message)) if message.contains("requires fixture_profile")
    ));

    fs::write(
        workspace.join(".flow/config.yaml"),
        "fixture_profile: live\nregistry_root: registry\nstub_model: deterministic\n",
    )
    .expect("unsupported fixture profile");
    assert!(matches!(
        load_workspace_config(&workspace),
        Err(RuntimeError::Usage(message)) if message.contains("unsupported .flow/config.yaml fixture_profile")
    ));

    fs::write(
        workspace.join(".flow/config.yaml"),
        "fixture_profile: stub-model\nregistry_root: registry\nstub_model: live\n",
    )
    .expect("unsupported stub model");
    assert!(matches!(
        load_workspace_config(&workspace),
        Err(RuntimeError::Usage(message)) if message.contains("unsupported .flow/config.yaml stub_model")
    ));

    for registry_root in [
        ".",
        "../registry",
        "registry/./nested",
        "registry/../nested",
        r"registry\nested",
        "C:/registry",
        "NUL/registry",
        "bad:name",
    ] {
        fs::write(
            workspace.join(".flow/config.yaml"),
            format!("registry_root: {registry_root}\n"),
        )
        .expect("unsafe config");
        assert!(
            matches!(
                load_workspace_config(&workspace),
                Err(RuntimeError::Usage(message)) if message.contains("within the workspace")
            ),
            "{registry_root}"
        );
    }
    assert!(matches!(
        read_workspace_config_to_string(&workspace.join("missing-workspace")),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));

    let oversized_len = usize::try_from(MAX_WORKSPACE_CONFIG_BYTES).expect("limit fits usize") + 1;
    fs::write(
        workspace.join(".flow/config.yaml"),
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
    fs::create_dir_all(workspace.join(".flow")).expect("loop config dir");
    let outside_config = outside.join("config.yaml");
    fs::write(&outside_config, "registry_root: registry\n").expect("outside config written");
    symlink(&outside_config, workspace.join(".flow/config.yaml")).expect("config symlink");

    let err = load_workspace_config(&workspace).expect_err("config symlink must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("symlink")));
}

#[cfg(any(unix, windows))]
#[test]
fn workspace_config_rejects_linked_parent_directory() {
    let workspace = empty_workspace("workspace-config-linked-parent");
    let outside = empty_workspace("outside-workspace-config-parent");
    fs::write(outside.join("config.yaml"), "registry_root: registry\n")
        .expect("outside config written");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, workspace.join(".flow"))
        .expect("config parent symlink created");
    #[cfg(windows)]
    create_windows_junction(&workspace.join(".flow"), &outside);

    let err = load_workspace_config(&workspace).expect_err("linked config parent must fail");

    assert!(
        matches!(&err, RuntimeError::Protocol(message)
            if message.contains("symlink") || message.contains("reparse")),
        "unexpected error: {err:?}"
    );
}

#[cfg(unix)]
#[test]
fn run_loop_rejects_symlinked_log_dir_without_side_effects() {
    use std::os::unix::fs::symlink;

    let workspace = workspace_copy("smoke-loop");
    let outside = empty_workspace("outside-log");
    fs::create_dir_all(workspace.join(".flow")).expect("loop dir");
    symlink(&outside, workspace.join(LOCAL_LOG_DIR)).expect("log dir symlink");

    let err = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
        .expect_err("symlinked log dir must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("symlink")));
    assert!(!outside.join("smoke-loop.log").exists());
    assert!(
        !workspace
            .join(LOCAL_SESSION_DIR)
            .join("smoke-loop.jsonl")
            .exists()
    );
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
    symlink(&outside_target, session_dir.join("smoke-loop.jsonl")).expect("session leaf symlink");

    let err = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
        .expect_err("symlinked session leaf must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("symlink")));
    assert!(!outside_target.exists());
    assert!(
        !workspace
            .join(LOCAL_LOG_DIR)
            .join("smoke-loop.log")
            .exists()
    );
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
    assert_no_session_artifacts(&workspace, "hello-loop");
}

#[test]
fn run_loop_writes_portable_near_limit_output_leaf() {
    let workspace = workspace_copy("hello-loop");
    let leaf = "a".repeat(240);
    let target = format!("out/{leaf}");
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "out/summary.txt",
        &target,
    );

    let output = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect("portable near-limit output leaf runs");

    assert!(!output.failed, "{}", output.stdout);
    assert_eq!(
        fs::read_to_string(workspace.join(target)).expect("long output leaf readable"),
        "hello\n"
    );
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
    assert_no_session_artifacts(&workspace, "hello-loop");
}

#[test]
fn run_loop_rejects_non_file_declared_write_paths_before_side_effects() {
    for (leaf_is_directory, expected) in [(true, "must be a file"), (false, "must be a directory")]
    {
        let workspace = workspace_copy("hello-loop");
        let output_parent = workspace.join("out");
        if leaf_is_directory {
            fs::create_dir_all(output_parent.join("summary.txt"))
                .expect("directory created at write leaf");
        } else {
            if output_parent.exists() {
                fs::remove_dir_all(&output_parent).expect("fixture output directory removed");
            }
            fs::write(&output_parent, "not a directory\n").expect("file created in write ancestor");
        }

        let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
            .expect_err("non-file declared write path must fail preflight");

        assert_denied(err, core_policy::DenyReasonCode::WriteDenied, expected);
        assert_no_session_artifacts(&workspace, "hello-loop");
    }
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
    assert!(
        events
            .iter()
            .any(|event| event.event_type == EventType::ToolFailed)
    );
    assert_eq!(terminal_failure_reason(&events), Some("write_denied"));
    assert_eq!(
        fs::read_to_string(&output.session_path).expect("session log readable"),
        output.stdout
    );
    assert!(
        workspace
            .join(LOCAL_LOG_DIR)
            .join("hello-loop.log")
            .exists()
    );
}

#[test]
fn tool_started_commit_failure_prevents_own_script_side_effect() {
    struct RejectWriteStart;

    impl RuntimeEventSink for RejectWriteStart {
        fn measurement_started_at(&self) -> Option<Instant> {
            None
        }

        fn commit(
            &mut self,
            event: &EventEnvelope,
            _canonical_jsonl: &str,
            _context_manifest: Option<ContextManifestCheckpoint>,
            _measurement_started_at: Option<Instant>,
        ) -> Result<(), RuntimeError> {
            if event.event_type == EventType::ToolStarted
                && event
                    .payload
                    .get("tool_id")
                    .and_then(serde_json::Value::as_str)
                    == Some("write-summary")
            {
                return Err(RuntimeError::EventWriter(Box::new(RuntimeError::Protocol(
                    "injected tool.started commit failure".to_owned(),
                ))));
            }
            Ok(())
        }
    }

    let workspace = workspace_copy("hello-loop");
    let (registry, policy) = fixture_runtime_policy("hello-loop", "hello-loop");
    let loop_block = registry
        .loop_block("hello-loop")
        .expect("hello loop exists");
    let err = match execute_loop_with_sink(
        &workspace,
        &registry,
        &policy,
        loop_block,
        "commitfail001",
        LoopExecutionOptions::new(EventClock::fixed_fixture(), ToolSideEffectMode::ApplyAll),
        Some(&mut RejectWriteStart),
    ) {
        Err(err) => err,
        Ok(_) => panic!("tool.started commit failure must stop dispatch"),
    };

    assert!(matches!(
        err,
        RuntimeError::EventWriter(source)
            if matches!(source.as_ref(), RuntimeError::Protocol(message)
                if message.contains("tool.started"))
    ));
    assert!(!workspace.join("out/summary.txt").exists());
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
    assert_no_session_artifacts(&workspace, "hello-loop");
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
    assert_no_session_artifacts(&workspace, "hello-loop");
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
    assert_no_session_artifacts(&workspace, "hello-loop");
}

#[cfg(not(any(unix, windows)))]
#[test]
fn run_loop_replaces_hardlinked_summary_leaf_without_modifying_link_target_when_link_count_unverified()
 {
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
