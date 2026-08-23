use super::{
    helpers::{create_directory_alias, empty_workspace},
    test_support::workspace_copy,
};
use crate::runtime::{
    config_io::{
        ExecutionBackend, WorkspaceConfig, classify_workspace_config_open_error,
        load_workspace_config, read_workspace_config_to_string, require_execution_backend,
        require_fixture_execution_backend, resume_event_clock,
    },
    context::{CONTEXT_SAFETY_MARGIN, ContextModelProfile, OPERATOR_MODEL_PROFILE_ID},
    session::run_flow,
    types::{EmitMode, EventClock, MAX_WORKSPACE_CONFIG_BYTES, RuntimeError},
};
use std::{fs, io, path::PathBuf};

#[test]
fn workspace_config_helpers_reject_unsafe_registry_roots() {
    let workspace = empty_workspace("workspace-config-helpers");
    fs::create_dir_all(workspace.join(".flow")).expect("flow config dir");
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
        "registry_root: registry # authoring registry\n",
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
        ".flow",
        ".flow/registry",
        ".FLOW",
        ".Flow/registry",
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
                Err(RuntimeError::Usage(message))
                    if message.contains("within the workspace") || message.contains("must not overlap .flow")
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

#[test]
fn live_workspace_requires_one_explicit_bounded_provider_and_model() {
    let workspace = empty_workspace("workspace-live-provider-config");
    fs::create_dir_all(workspace.join(".flow")).expect("flow config dir");

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
            "unsupported .flow/config.yaml provider",
        ),
        (
            "model: ''\nprovider: openai-codex\nregistry_root: registry\n",
            "requires model",
        ),
    ] {
        fs::write(workspace.join(".flow/config.yaml"), source).expect("live config written");
        let config = load_workspace_config(&workspace).expect("configuration parses for authoring");
        assert!(
            matches!(
                require_execution_backend(&config),
                Err(RuntimeError::Usage(message)) if message.contains(expected)
            ),
            "{source}"
        );
    }

    fs::write(
        workspace.join(".flow/config.yaml"),
        format!(
            "model: {}\nprovider: openai-codex\nregistry_root: registry\n",
            "m".repeat(257)
        ),
    )
    .expect("oversized model config written");
    let config = load_workspace_config(&workspace).expect("configuration parses for authoring");
    assert!(matches!(
        require_execution_backend(&config),
        Err(RuntimeError::Usage(message)) if message.contains("at most 256 Unicode scalars")
    ));

    fs::write(
        workspace.join(".flow/config.yaml"),
        "model: gpt-fixture\nmodel_context_limit: 128000\noutput_reserve: 16384\nprovider: openai-codex\nregistry_root: registry\n",
    )
    .expect("productive config written");
    let config = load_workspace_config(&workspace).expect("productive config parses");
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
        workspace.join(".flow/config.yaml"),
        "model: gpt-fixture\nmodel_context_limit: 20480\noutput_reserve: 16384\nprovider: openai-codex\nregistry_root: registry\n",
    )
    .expect("invalid productive profile written");
    let config = load_workspace_config(&workspace).expect("configuration parses for authoring");
    assert!(matches!(
        require_execution_backend(&config),
        Err(RuntimeError::Usage(message)) if message.contains("must leave a positive input budget")
    ));
}

#[test]
fn fixture_workspace_rejects_productive_backend_fields() {
    let workspace = empty_workspace("workspace-fixture-backend-config");
    fs::create_dir_all(workspace.join(".flow")).expect("flow config dir");

    for field in ["provider: openai-codex", "model: gpt-fixture"] {
        fs::write(
            workspace.join(".flow/config.yaml"),
            format!(
                "fixture_profile: stub-model\nregistry_root: registry\nstub_model: deterministic\n{field}\n"
            ),
        )
        .expect("mixed fixture config written");
        let config = load_workspace_config(&workspace).expect("mixed fixture config parses");
        assert!(matches!(
            require_execution_backend(&config),
            Err(RuntimeError::Usage(message))
                if message.contains("fixture profiles must not declare productive")
        ));
    }
}

#[test]
fn execution_backend_helpers_preserve_fixture_and_productive_boundaries() {
    let workspace = workspace_copy("smoke-flow");
    let fixture = load_workspace_config(&workspace).expect("fixture config loads");
    require_fixture_execution_backend(&fixture).expect("fixture backend is accepted");

    fs::write(
        workspace.join(".flow/config.yaml"),
        "model: gpt-fixture\nmodel_context_limit: 128000\noutput_reserve: 16384\nprovider: openai-codex\nregistry_root: registry\n",
    )
    .expect("productive config writes");
    let productive = load_workspace_config(&workspace).expect("productive config loads");
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
    let config = |model: &str| WorkspaceConfig {
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
fn non_fixture_workspace_fails_closed_before_runtime_side_effects() {
    let workspace = workspace_copy("hello-flow");
    fs::write(
        workspace.join(".flow/config.yaml"),
        "registry_root: registry\n",
    )
    .expect("normal workspace config written");

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
fn workspace_config_reports_non_regular_config_leaf() {
    let workspace = workspace_copy("hello-flow");
    let config_path = workspace.join(".flow/config.yaml");
    fs::remove_file(&config_path).expect("fixture config removed");
    fs::create_dir(&config_path).expect("config path replaced with directory");

    let err = load_workspace_config(&workspace).expect_err("config directory must fail");

    assert!(
        matches!(&err, RuntimeError::Protocol(message)
            if message.contains("regular file") && !message.contains("symlink")),
        "unexpected error: {err:?}"
    );
}

#[test]
fn workspace_config_reports_non_directory_flow_parent() {
    let workspace = workspace_copy("hello-flow");
    let flow_path = workspace.join(".flow");
    fs::remove_dir_all(&flow_path).expect("fixture flow directory removed");
    fs::write(&flow_path, b"not a directory").expect("flow path replaced with a file");

    let err = load_workspace_config(&workspace).expect_err("flow parent file must fail");

    assert!(
        matches!(&err, RuntimeError::Protocol(message) if message.contains("directory")),
        "unexpected error: {err:?}"
    );
}

#[test]
fn workspace_config_preserves_unrelated_open_errors() {
    let workspace = workspace_copy("hello-flow");
    let flow_path = workspace.join(".flow");
    let config_path = flow_path.join("config.yaml");
    let workspace_dir =
        cap_std::fs::Dir::open_ambient_dir(&workspace, cap_std::ambient_authority())
            .expect("workspace opens");
    let flow_dir = workspace_dir
        .open_dir(".flow")
        .expect("flow directory opens");

    let err = classify_workspace_config_open_error(
        &flow_dir,
        "config.yaml",
        config_path.clone(),
        io::Error::new(io::ErrorKind::PermissionDenied, "injected access failure"),
        "file",
    );

    assert!(matches!(
        err,
        RuntimeError::Io { path, source }
            if path == config_path && source.kind() == io::ErrorKind::PermissionDenied
    ));
}

#[cfg(unix)]
#[test]
fn workspace_config_rejects_symlinked_config_file() {
    use std::os::unix::fs::symlink;

    let workspace = empty_workspace("workspace-config-symlink");
    let outside = empty_workspace("outside-workspace-config");
    fs::create_dir_all(workspace.join(".flow")).expect("flow config dir");
    let outside_config = outside.join("config.yaml");
    fs::write(&outside_config, "registry_root: registry\n").expect("outside config written");
    symlink(&outside_config, workspace.join(".flow/config.yaml")).expect("config symlink");

    let err = load_workspace_config(&workspace).expect_err("config symlink must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("symlink")));
}

#[cfg(any(unix, windows))]
#[test]
fn workspace_config_rejects_hardlinked_config_file() {
    let workspace = workspace_copy("hello-flow");
    let outside = empty_workspace("outside-workspace-config-hardlink");
    let config_path = workspace.join(".flow/config.yaml");
    let outside_config = outside.join("config.yaml");
    fs::write(&outside_config, "registry_root: registry\n").expect("outside config written");
    fs::remove_file(&config_path).expect("fixture config removed");
    fs::hard_link(&outside_config, &config_path).expect("config hard link created");

    let err = load_workspace_config(&workspace).expect_err("hard-linked config must fail");

    assert!(
        matches!(&err, RuntimeError::Protocol(message) if message.contains("hard-linked")),
        "unexpected error: {err:?}"
    );
}

#[cfg(any(unix, windows))]
#[test]
fn workspace_config_rejects_linked_parent_directory() {
    let workspace = empty_workspace("workspace-config-linked-parent");
    let outside = empty_workspace("outside-workspace-config-parent");
    fs::write(outside.join("config.yaml"), "registry_root: registry\n")
        .expect("outside config written");
    create_directory_alias(&workspace.join(".flow"), &outside);

    let err = load_workspace_config(&workspace).expect_err("linked config parent must fail");

    assert!(
        matches!(&err, RuntimeError::Protocol(message)
            if message.contains("symlink") || message.contains("reparse")),
        "unexpected error: {err:?}"
    );
}
