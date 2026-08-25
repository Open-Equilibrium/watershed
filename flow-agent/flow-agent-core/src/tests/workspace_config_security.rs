use super::{
    helpers::{create_directory_alias, empty_workspace},
    test_support::{session_home_path, workspace_copy},
};
use crate::runtime::{
    config_io::{
        ExecutionBackend, GlobalConfig, load_global_config, require_execution_backend,
        require_fixture_execution_backend, resume_event_clock,
    },
    context::{CONTEXT_SAFETY_MARGIN, ContextModelProfile, OPERATOR_MODEL_PROFILE_ID},
    fs_guards::AnchoredWorkspace,
    instructions::read_applicable_agent_instructions,
    session::run_flow,
    session_store::open_flow_agent_home,
    types::{EmitMode, EventClock, MAX_GLOBAL_CONFIG_BYTES, RuntimeError},
};
use std::{fs, io, path::PathBuf};

fn global_config_path() -> PathBuf {
    let path = session_home_path().join("config.yaml");
    if !path.is_file() {
        crate::initialize_global_config(None).expect("isolated global Flow authority initializes");
    }
    path
}

#[test]
fn global_configuration_is_the_only_implicit_flow_authority() {
    let workspace = workspace_copy("smoke-flow");
    fs::write(
        workspace.join(".flow/config.yaml"),
        "this local config must not be parsed: [\n",
    )
    .expect("ambient local config is made invalid");
    fs::remove_dir_all(workspace.join("registry")).expect("ambient local registry is removed");
    fs::write(
        session_home_path().join("AGENTS.md"),
        "provider: forbidden\nregistry_root: forbidden\n",
    )
    .expect("global instructions are written");
    fs::write(
        workspace.join("AGENTS.md"),
        "model: forbidden\ncredentials: forbidden\n",
    )
    .expect("workspace instructions are written");
    let home = open_flow_agent_home(false, true)
        .expect("global home opens")
        .expect("global home exists");
    let anchored_workspace = AnchoredWorkspace::open(&workspace).expect("workspace anchors");

    assert_eq!(
        read_applicable_agent_instructions(&home, &anchored_workspace)
            .expect("both applicable instruction files are read"),
        "provider: forbidden\nregistry_root: forbidden\n\nmodel: forbidden\ncredentials: forbidden\n"
    );
    assert_eq!(
        load_global_config()
            .expect("instructions do not alter global configuration")
            .registry_root,
        PathBuf::from("registry")
    );

    let output = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect("the global config and registry are authoritative");

    assert!(!output.failed);
}

#[test]
fn missing_global_registry_never_falls_back_to_the_ambient_workspace_registry() {
    let workspace = workspace_copy("smoke-flow");
    assert!(workspace.join("registry/flows/smoke-flow.yaml").is_file());
    fs::remove_dir_all(session_home_path().join("registry")).expect("global registry is removed");

    let error = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("the ambient Workspace registry is not a fallback");

    assert!(error.to_string().contains("registry"), "{error}");
    assert!(!crate::tests::helpers::workspace_session_dir(&workspace).exists());
}

#[test]
fn unfinished_global_initialization_fails_before_session_mutation() {
    let workspace = workspace_copy("smoke-flow");
    fs::write(session_home_path().join(".flow-init.json"), "{}\n")
        .expect("conflicting transaction marker writes");

    let error = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("conflicting global state fails closed");

    assert!(
        error.to_string().contains("unfinished initialization"),
        "{error}"
    );
    assert!(!crate::tests::helpers::workspace_session_dir(&workspace).exists());
}

#[cfg(unix)]
#[test]
fn inaccessible_ambient_workspace_config_is_never_probed() {
    use std::os::unix::fs::PermissionsExt as _;

    let workspace = workspace_copy("smoke-flow");
    let local_config = workspace.join(".flow/config.yaml");
    fs::set_permissions(&local_config, fs::Permissions::from_mode(0o000))
        .expect("ambient config is made inaccessible");

    let output = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect("only the global Flow authority is read");

    assert!(!output.failed);
}

#[cfg(unix)]
#[test]
fn inaccessible_global_config_fails_before_session_mutation() {
    use std::os::unix::fs::PermissionsExt as _;

    let workspace = workspace_copy("smoke-flow");
    let config = global_config_path();
    let original_permissions = fs::metadata(&config)
        .expect("global config metadata reads")
        .permissions();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o000))
        .expect("global config is made inaccessible");

    let result = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl);
    fs::set_permissions(&config, original_permissions).expect("global config permissions restore");
    let error = result.expect_err("an inaccessible global config fails closed");

    assert!(
        matches!(&error, RuntimeError::Io { source, .. }
            if source.kind() == io::ErrorKind::PermissionDenied),
        "unexpected error: {error:?}"
    );
    assert!(!crate::tests::helpers::workspace_session_dir(&workspace).exists());
}

#[test]
fn global_config_helpers_reject_unsafe_registry_roots() {
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
        fs::write(global_config_path(), source).expect("invalid config written");
        match load_global_config().expect_err("invalid config must be rejected") {
            RuntimeError::Usage(message) => {
                assert!(message.contains(expected), "{label}: {message}")
            }
            error => panic!("{label}: {error}"),
        }
    }

    fs::write(global_config_path(), "stub_model: deterministic\n")
        .expect("config without registry root");
    assert!(matches!(
        load_global_config(),
        Err(RuntimeError::Usage(message)) if message.contains("missing")
    ));

    for registry_root in ["registry", "nested/registry", "répertoire/注册表"] {
        fs::write(
            global_config_path(),
            format!("registry_root: {registry_root}\n"),
        )
        .expect("valid config");
        let config = load_global_config().expect("config loads");
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
        global_config_path(),
        "registry_root: registry # authoring registry\n",
    )
    .expect("commented config");
    let config = load_global_config().expect("commented config loads");
    assert_eq!(config.registry_root, PathBuf::from("registry"));

    fs::write(
        global_config_path(),
        "fixture_profile: stub-model\nregistry_root: registry\nstub_model: deterministic\n",
    )
    .expect("fixture config");
    let config = load_global_config().expect("fixture config loads");
    assert_eq!(config.event_clock, EventClock::fixed_fixture());

    fs::write(
        global_config_path(),
        "fixture_profile: stub-model\nregistry_root: registry\n",
    )
    .expect("fixture config without stub model");
    assert!(matches!(
        load_global_config(),
        Err(RuntimeError::Usage(message)) if message.contains("requires stub_model")
    ));

    fs::write(
        global_config_path(),
        "registry_root: registry\nstub_model: deterministic\n",
    )
    .expect("stub model without fixture profile");
    assert!(matches!(
        load_global_config(),
        Err(RuntimeError::Usage(message)) if message.contains("requires fixture_profile")
    ));

    fs::write(
        global_config_path(),
        "fixture_profile: live\nregistry_root: registry\nstub_model: deterministic\n",
    )
    .expect("unsupported fixture profile");
    assert!(matches!(
        load_global_config(),
        Err(RuntimeError::Usage(message)) if message.contains("unsupported FLOW_AGENT_HOME/config.yaml fixture_profile")
    ));

    fs::write(
        global_config_path(),
        "fixture_profile: stub-model\nregistry_root: registry\nstub_model: live\n",
    )
    .expect("unsupported stub model");
    assert!(matches!(
        load_global_config(),
        Err(RuntimeError::Usage(message)) if message.contains("unsupported FLOW_AGENT_HOME/config.yaml stub_model")
    ));

    for registry_root in [
        ".",
        "config.yaml",
        "config.yaml/registry",
        "CONFIG.YAML",
        "workspaces",
        "workspaces/registry",
        ".flow-init.json/registry",
        ".flow-init.lock/registry",
        "AGENTS.md",
        "agents.md/registry",
        "../registry",
        "registry/./nested",
        "registry/../nested",
        r"registry\nested",
        "C:/registry",
        "NUL/registry",
        "bad:name",
    ] {
        fs::write(
            global_config_path(),
            format!("registry_root: {registry_root}\n"),
        )
        .expect("unsafe config");
        assert!(
            matches!(
                load_global_config(),
                Err(RuntimeError::Usage(message))
                    if message.contains("within the global Flow home")
                        || message.contains("global config file")
                        || message.contains("reserved global Flow path")
            ),
            "{registry_root}"
        );
    }

    let oversized_len = usize::try_from(MAX_GLOBAL_CONFIG_BYTES).expect("limit fits usize") + 1;
    fs::write(
        global_config_path(),
        format!("registry_root: registry\n{}", "x".repeat(oversized_len)),
    )
    .expect("oversized config written");
    assert!(matches!(
        load_global_config(),
        Err(RuntimeError::Protocol(message)) if message.contains("exceeds max")
    ));
}

#[test]
fn live_global_config_requires_one_explicit_bounded_provider_and_model() {
    for (source, expected) in [
        ("registry_root: registry\n", "requires provider"),
        (
            "provider: openai-codex\nregistry_root: registry\n",
            "requires model",
        ),
        (
            "model: gpt-fixture\nprovider: openai-codex\nregistry_root: registry\n",
            "requires model_context_limit",
        ),
        (
            "model: gpt-fixture\nmodel_context_limit: 128000\nprovider: openai-codex\nregistry_root: registry\n",
            "requires output_reserve",
        ),
        (
            "model: gpt-fixture\nprovider: another\nregistry_root: registry\n",
            "unsupported FLOW_AGENT_HOME/config.yaml provider",
        ),
        (
            "model: ''\nprovider: openai-codex\nregistry_root: registry\n",
            "requires model",
        ),
    ] {
        fs::write(global_config_path(), source).expect("live config written");
        let config = load_global_config().expect("configuration parses for authoring");
        assert!(
            matches!(
                require_execution_backend(&config),
                Err(RuntimeError::Usage(message)) if message.contains(expected)
            ),
            "{source}"
        );
    }

    fs::write(
        global_config_path(),
        format!(
            "model: {}\nprovider: openai-codex\nregistry_root: registry\n",
            "m".repeat(257)
        ),
    )
    .expect("oversized model config written");
    let config = load_global_config().expect("configuration parses for authoring");
    assert!(matches!(
        require_execution_backend(&config),
        Err(RuntimeError::Usage(message)) if message.contains("at most 256 Unicode scalars")
    ));

    fs::write(
        global_config_path(),
        "model: gpt-fixture\nmodel_context_limit: 128000\noutput_reserve: 16384\nprovider: openai-codex\nregistry_root: registry\n",
    )
    .expect("productive config written");
    let config = load_global_config().expect("productive config parses");
    assert_eq!(
        require_execution_backend(&config).expect("productive backend resolves"),
        ExecutionBackend::OpenAiCodex {
            model: "gpt-fixture".to_owned(),
            model_profile: ContextModelProfile {
                context_limit: 128000,
                id: "operator-model-v0",
                output_reserve: 16384,
                safety_margin: CONTEXT_SAFETY_MARGIN,
            },
        }
    );

    fs::write(
        global_config_path(),
        "model: gpt-fixture\nmodel_context_limit: 20480\noutput_reserve: 16384\nprovider: openai-codex\nregistry_root: registry\n",
    )
    .expect("invalid productive profile written");
    let config = load_global_config().expect("configuration parses for authoring");
    assert!(matches!(
        require_execution_backend(&config),
        Err(RuntimeError::Usage(message)) if message.contains("must leave a positive input budget")
    ));
}

#[test]
fn fixture_global_config_rejects_productive_backend_fields() {
    for field in ["provider: openai-codex", "model: gpt-fixture"] {
        fs::write(
            global_config_path(),
            format!(
                "fixture_profile: stub-model\nregistry_root: registry\nstub_model: deterministic\n{field}\n"
            ),
        )
        .expect("mixed fixture config written");
        let config = load_global_config().expect("mixed fixture config parses");
        assert!(matches!(
            require_execution_backend(&config),
            Err(RuntimeError::Usage(message))
                if message.contains("fixture profiles must not declare productive")
        ));
    }
}

#[test]
fn execution_backend_helpers_preserve_fixture_and_productive_boundaries() {
    let _workspace = workspace_copy("smoke-flow");
    let fixture = load_global_config().expect("fixture config loads");
    require_fixture_execution_backend(&fixture).expect("fixture backend is accepted");

    fs::write(
        global_config_path(),
        "model: gpt-fixture\nmodel_context_limit: 128000\noutput_reserve: 16384\nprovider: openai-codex\nregistry_root: registry\n",
    )
    .expect("productive config writes");
    let productive = load_global_config().expect("productive config loads");
    assert!(matches!(
        require_fixture_execution_backend(&productive),
        Err(RuntimeError::ExecutionBackendUnavailable)
    ));

    let fixture_clock = EventClock::fixed_fixture();
    let recorded_clock = EventClock {
        base_unix_seconds: 1_700_000_000,
    };
    assert_eq!(
        resume_event_clock(&fixture, recorded_clock).expect("fixture resume clock resolves"),
        fixture_clock
    );
}

#[test]
fn productive_model_validation_counts_unicode_scalars_and_rejects_controls() {
    let config = |model: &str| GlobalConfig {
        event_clock: EventClock::wall_clock(),
        model: Some(model.to_owned()),
        model_context_limit: Some(128000),
        output_reserve: Some(16384),
        provider: Some("openai-codex".to_owned()),
        registry_root: PathBuf::from("registry"),
        stub_model_fixture_profile: false,
    };

    assert!(matches!(
        require_execution_backend(&config("")),
        Err(RuntimeError::Usage(message)) if message.contains("at least one Unicode scalar")
    ));
    assert!(matches!(
        require_execution_backend(&config("model\nname")),
        Err(RuntimeError::Usage(message)) if message.contains("control characters")
    ));
    assert_eq!(
        require_execution_backend(&config(&"🦀".repeat(256)))
            .expect("256 Unicode scalars are accepted"),
        ExecutionBackend::OpenAiCodex {
            model: "🦀".repeat(256),
            model_profile: ContextModelProfile {
                context_limit: 128000,
                id: OPERATOR_MODEL_PROFILE_ID,
                output_reserve: 16384,
                safety_margin: CONTEXT_SAFETY_MARGIN,
            },
        }
    );
}

#[test]
fn non_fixture_global_config_fails_closed_before_runtime_side_effects() {
    let workspace = workspace_copy("hello-flow");
    fs::write(global_config_path(), "registry_root: registry\n").expect("global config written");

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("live execution requires an explicit provider and model");

    assert!(err.to_string().contains("requires provider"), "{err}");
    assert_eq!(err.exit_code(), 64);
    assert!(!workspace.join("out/summary.txt").exists());
    assert!(
        !crate::tests::helpers::workspace_session_dir(&workspace).exists(),
        "backend rejection must precede session persistence"
    );
}

#[test]
fn global_config_reports_non_regular_config_leaf() {
    let _workspace = workspace_copy("hello-flow");
    let config_path = global_config_path();
    fs::remove_file(&config_path).expect("fixture config removed");
    fs::create_dir(&config_path).expect("config path replaced with directory");

    let err = load_global_config().expect_err("config directory must fail");

    assert!(
        matches!(&err, RuntimeError::Protocol(message)
            if message.contains("must be a file") && !message.contains("symlink")),
        "unexpected error: {err:?}"
    );
}

#[test]
fn global_config_missing_fails_closed() {
    let workspace = empty_workspace("missing-global-config");

    let err = load_global_config().expect_err("missing global config must fail");

    assert!(
        matches!(&err, RuntimeError::Io { source, .. } if source.kind() == io::ErrorKind::NotFound),
        "unexpected error: {err:?}"
    );
    assert!(!crate::tests::helpers::workspace_session_dir(&workspace).exists());
}

#[cfg(unix)]
#[test]
fn global_config_rejects_symlinked_config_file() {
    use std::os::unix::fs::symlink;

    let outside = empty_workspace("outside-global-config");
    let outside_config = outside.join("config.yaml");
    fs::write(&outside_config, "registry_root: registry\n").expect("outside config written");
    let config_path = global_config_path();
    fs::remove_file(&config_path).expect("initialized config removes");
    symlink(&outside_config, &config_path).expect("config symlink");

    let err = load_global_config().expect_err("config symlink must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("symlink")));
}

#[cfg(any(unix, windows))]
#[test]
fn global_config_rejects_hardlinked_config_file() {
    let _workspace = workspace_copy("hello-flow");
    let outside = empty_workspace("outside-global-config-hardlink");
    let config_path = global_config_path();
    let outside_config = outside.join("config.yaml");
    fs::write(&outside_config, "registry_root: registry\n").expect("outside config written");
    fs::remove_file(&config_path).expect("fixture config removed");
    fs::hard_link(&outside_config, &config_path).expect("config hard link created");

    let err = load_global_config().expect_err("hard-linked config must fail");

    assert!(
        matches!(&err, RuntimeError::Protocol(message) if message.contains("hard-linked")),
        "unexpected error: {err:?}"
    );
}

#[cfg(any(unix, windows))]
#[test]
fn global_config_rejects_linked_home_directory() {
    let outside = empty_workspace("outside-global-config-home");
    fs::write(outside.join("config.yaml"), "registry_root: registry\n")
        .expect("outside config written");
    create_directory_alias(&session_home_path(), &outside);

    let err = load_global_config().expect_err("linked config parent must fail");

    assert!(
        matches!(&err, RuntimeError::Protocol(message)
            if message.contains("symlink") || message.contains("reparse")),
        "unexpected error: {err:?}"
    );
}
