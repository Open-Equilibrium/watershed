use super::*;
use proptest::prelude::*;
use std::{fs, path::Path};

#[test]
fn policy_compiler_matches_m1_linux_and_macos_fixtures() {
    for fixture in ["smoke-loop", "hello-loop"] {
        let registry = core_script::load_registry_root(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../loop-agent/fixtures")
                .join(fixture)
                .join("registry"),
        )
        .expect("fixture registry loads");

        let artifacts = compile_policy_artifacts(fixture, &registry, fixture)
            .expect("policy artifacts compile");
        assert_eq!(artifacts.len(), 2);
        for (artifact, file_name) in artifacts.iter().zip([
            "linux-landlock-seccomp.policy.json",
            "macos-seatbelt.policy.json",
        ]) {
            let actual = canonical_artifact_json(artifact).expect("artifact serializes");
            let expected = fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("fixtures")
                    .join(fixture)
                    .join(file_name),
            )
            .expect("expected policy fixture is readable");
            let expected_artifact: PolicyArtifact =
                serde_json::from_str(&expected).expect("expected policy fixture parses");

            assert_eq!(actual, expected, "{fixture} {file_name}");
            assert_eq!(*artifact, expected_artifact, "{fixture} {file_name}");
        }
    }
}

#[test]
fn policy_compiler_rejects_non_empty_network_allowlists_for_os_enforced_m1() {
    let mut registry = core_script::load_registry_root(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../loop-agent/fixtures/smoke-loop/registry"),
    )
    .expect("smoke-loop registry loads");
    registry.tools.get_mut("echo").expect("echo tool").network =
        core_script::NetworkPolicy::Declared {
            default: core_script::NetworkDefault::Deny,
            allow: vec![core_script::NetworkAllowEntry {
                kind: core_script::NetworkAllowKind::Cidr,
                transport: core_script::NetworkTransport::Tcp,
                cidr: "192.0.2.0/24".to_owned(),
                port: 443,
            }],
        };

    let err = compile_policy_artifact(
        "smoke-loop",
        &registry,
        "smoke-loop",
        PolicyTarget::LinuxLandlockSeccomp,
    )
    .expect_err("network allowlist is rejected");

    assert!(matches!(
        err,
        PolicyCompileError::NonEmptyNetworkAllowlist { .. }
    ));
}

#[test]
fn policy_compiler_preserves_macos_network_allowlists() {
    let mut registry = core_script::load_registry_root(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../loop-agent/fixtures/smoke-loop/registry"),
    )
    .expect("smoke-loop registry loads");
    registry.tools.get_mut("echo").expect("echo tool").network =
        core_script::NetworkPolicy::Declared {
            default: core_script::NetworkDefault::Deny,
            allow: vec![core_script::NetworkAllowEntry {
                kind: core_script::NetworkAllowKind::Cidr,
                transport: core_script::NetworkTransport::Tcp,
                cidr: "192.0.2.0/24".to_owned(),
                port: 443,
            }],
        };

    let artifact = compile_policy_artifact(
        "smoke-loop",
        &registry,
        "smoke-loop",
        PolicyTarget::MacosSeatbelt,
    )
    .expect("macOS policy artifacts may carry reviewed CIDR allowlists");

    assert_eq!(artifact.target, PolicyTarget::MacosSeatbelt);
    assert_eq!(artifact.commands[0].network.default, NetworkDefault::Deny);
    assert_eq!(
        artifact.commands[0].network.allow,
        vec![NetworkAllowEntry {
            cidr: "192.0.2.0/24".to_owned(),
            kind: NetworkAllowKind::Cidr,
            port: 443,
            transport: NetworkTransport::Tcp,
        }]
    );
}

#[test]
fn policy_compiler_rejects_unknown_predefined_commands() {
    let mut registry = core_script::load_registry_root(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../loop-agent/fixtures/smoke-loop/registry"),
    )
    .expect("smoke-loop registry loads");
    registry.tools.get_mut("echo").expect("echo tool").command =
        core_script::ToolCommand::Predefined {
            command_id: "agent-custom".to_owned(),
            argv: Vec::new(),
        };

    let err = compile_policy_artifact(
        "smoke-loop",
        &registry,
        "smoke-loop",
        PolicyTarget::LinuxLandlockSeccomp,
    )
    .expect_err("unknown predefined command must fail closed");

    assert!(err.to_string().contains("unknown trusted command"), "{err}");
}

#[test]
fn policy_compiler_rejects_tool_kind_command_shape_mismatches() {
    let mut registry = core_script::load_registry_root(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../loop-agent/fixtures/smoke-loop/registry"),
    )
    .expect("smoke-loop registry loads");
    registry.tools.get_mut("echo").expect("echo tool").tool_kind = core_script::ToolKind::OwnScript;

    let err = compile_policy_artifact(
        "smoke-loop",
        &registry,
        "smoke-loop",
        PolicyTarget::LinuxLandlockSeccomp,
    )
    .expect_err("tool kind and command shape mismatch must fail closed");

    assert!(
        err.to_string()
            .contains("tool echo command shape does not match tool_kind"),
        "{err}"
    );
}

#[test]
fn policy_artifact_rejects_forbidden_environment_allow_entries() {
    let forbidden_names = [
        "AWS_REGION",
        "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER",
        "GIT_CONFIG_GLOBAL",
        "HTTP_PROXY",
        "KUBECONFIG",
        "LD_PRELOAD",
        "MY_CREDENTIALS",
        "OPENAI_API_KEY",
        "PATH",
        "SERVICE_TOKEN",
    ];

    for name in forbidden_names {
        let artifact = policy_artifact_with_environment_allow(name);

        let err = artifact
            .validate()
            .expect_err("forbidden environment allow entry must fail validation");

        assert!(
            err.to_string().contains(name),
            "{name} should be named in {err}"
        );
    }
}

#[test]
fn policy_artifact_rejects_malformed_environment_allow_entries() {
    let too_long = "A".repeat(65);
    for name in ["", "lowercase", "A-B", "1INVALID", too_long.as_str()] {
        let artifact = policy_artifact_with_environment_allow(name);

        let err = artifact
            .validate()
            .expect_err("malformed environment allow entry must fail validation");

        assert!(
            err.to_string().contains("^[A-Z_][A-Z0-9_]{0,63}$"),
            "{name:?} should report the environment allow grammar"
        );
    }
}

#[test]
fn policy_artifact_rejects_malformed_network_allow_entries() {
    for cidr in [
        "example.com",
        "192.0.2.42",
        "192.0.2.42/24",
        "192.0.2.0/33",
        "10.0.0.0/01",
        "2001:db8::1/32",
        "2001:DB8::/32",
    ] {
        let artifact = policy_artifact_with_network_allow(cidr, 443);

        let err = artifact
            .validate()
            .expect_err("malformed network allow entry must fail validation");

        assert!(
            err.to_string().contains(cidr),
            "{cidr:?} should be named in {err}"
        );
        assert!(
            err.to_string().contains("canonical CIDR"),
            "{cidr:?} should report the CIDR contract"
        );
    }
}

#[test]
fn policy_artifact_rejects_non_empty_linux_network_allow_entries() {
    let artifact = policy_artifact_with_network_allow("192.0.2.0/24", 443);

    let err = artifact
        .validate()
        .expect_err("linux artifacts must reject network allowlists");

    assert_eq!(
        err.to_string(),
        "tool network-tool network allow must be empty for linux-landlock-seccomp policy artifacts"
    );
}

#[test]
fn policy_artifact_rejects_zero_network_allow_port() {
    let artifact = policy_artifact_with_network_allow("192.0.2.0/24", 0);

    let err = artifact
        .validate()
        .expect_err("port zero must fail validation");

    assert_eq!(
        err.to_string(),
        "tool network-tool network allow entry 192.0.2.0/24 must use port 1-65535"
    );
}

#[test]
fn policy_artifact_rejects_unsupported_policy_version() {
    let mut artifact = valid_policy_artifact("version-tool");
    artifact.policy_version = "1".to_owned();

    let err = artifact
        .validate()
        .expect_err("unsupported policy version must fail validation");

    assert_eq!(err.to_string(), "policy_version must be fixed string \"0\"");
}

#[test]
fn policy_artifact_rejects_mismatched_command_shapes() {
    let mut predefined_runtime = valid_policy_artifact("read-file");
    predefined_runtime.commands[0].script_runtime = Some("posix-sh".to_owned());
    let err = predefined_runtime
        .validate()
        .expect_err("predefined-command must omit script_runtime");
    assert_eq!(
        err.to_string(),
        "predefined-command tool read-file must omit script_runtime"
    );

    let mut predefined_command_id = valid_policy_artifact("read-file");
    predefined_command_id.commands[0].command_id = "1-agent-read".to_owned();
    let err = predefined_command_id
        .validate()
        .expect_err("predefined-command id must follow the command id grammar");
    assert_eq!(
        err.to_string(),
        "predefined-command tool read-file command_id \"1-agent-read\" must match ^[a-z][a-z0-9_-]{0,63}$"
    );

    let mut own_script_command_id = own_script_policy_artifact("write-summary");
    own_script_command_id.commands[0].command_id = "script:other-tool".to_owned();
    let err = own_script_command_id
        .validate()
        .expect_err("own-script command_id must match tool_id");
    assert_eq!(
        err.to_string(),
        "own-script tool write-summary command_id must be script:write-summary"
    );

    let mut own_script_runtime = own_script_policy_artifact("write-summary");
    own_script_runtime.commands[0].script_runtime = None;
    let err = own_script_runtime
        .validate()
        .expect_err("own-script must declare posix-sh runtime");
    assert_eq!(
        err.to_string(),
        "own-script tool write-summary must use script_runtime posix-sh"
    );

    let mut own_script_argv = own_script_policy_artifact("write-summary");
    own_script_argv.commands[0].argv = vec!["-c".to_owned()];
    let err = own_script_argv
        .validate()
        .expect_err("own-script must not supply runner arguments");
    assert_eq!(
        err.to_string(),
        "own-script tool write-summary must omit argv"
    );
}

#[test]
fn policy_artifact_rejects_malformed_allowed_parameters() {
    let mut bad_name = valid_policy_artifact("parameter-tool");
    bad_name.commands[0].allowed_parameters[0].name = "file".to_owned();
    let err = bad_name
        .validate()
        .expect_err("parameter names must be exact flags");
    assert_eq!(
        err.to_string(),
        "tool parameter-tool parameter name \"file\" must match ^--[A-Za-z0-9][A-Za-z0-9_-]*$"
    );

    let mut string_without_constraints = valid_policy_artifact("parameter-tool");
    string_without_constraints.commands[0].allowed_parameters[1].max_length = None;
    let err = string_without_constraints
        .validate()
        .expect_err("string parameters require length and pattern constraints");
    assert_eq!(
        err.to_string(),
        "tool parameter-tool string parameter --alpha must set value_pattern and max_length"
    );

    let mut enum_without_values = valid_policy_artifact("parameter-tool");
    enum_without_values.commands[0].allowed_parameters[0]
        .allowed_values
        .clear();
    let err = enum_without_values
        .validate()
        .expect_err("enum parameters require allowed values");
    assert_eq!(
        err.to_string(),
        "tool parameter-tool enum parameter --beta must set allowed_values"
    );
}

#[test]
fn policy_artifact_rejects_non_default_protected_paths() {
    let mut artifact = valid_policy_artifact("filesystem-tool");
    artifact.commands[0].filesystem.protected_paths = vec!["**/.env".to_owned()];

    let err = artifact
        .validate()
        .expect_err("protected paths must match the SECURITY.md default set");

    assert_eq!(
        err.to_string(),
        "tool filesystem-tool filesystem protected_paths must match SECURITY.md defaults"
    );
}

#[test]
fn policy_artifact_rejects_protected_path_grants_outside_scope() {
    let mut artifact = valid_policy_artifact("filesystem-tool");
    artifact.commands[0].filesystem.protected_path_grants = vec!["secrets/.env".to_owned()];

    let err = artifact
        .validate()
        .expect_err("protected path grants must stay inside tool scopes");

    assert_eq!(
        err.to_string(),
        "tool filesystem-tool protected_path_grant \"secrets/.env\" must stay inside read_roots or write_roots"
    );
}

#[test]
fn policy_artifact_rejects_protected_path_grants_outside_write_scope() {
    let mut artifact = valid_policy_artifact("filesystem-tool");
    artifact.commands[0].filesystem.read_roots = vec!["workspace/in".to_owned()];
    artifact.commands[0].filesystem.write_roots = vec!["workspace/out".to_owned()];
    artifact.commands[0].filesystem.protected_path_grants =
        vec!["workspace/secrets/.env".to_owned()];

    let err = artifact
        .validate()
        .expect_err("protected path grants must stay inside declared scopes");

    assert_eq!(
        err.to_string(),
        "tool filesystem-tool protected_path_grant \"workspace/secrets/.env\" must stay inside read_roots or write_roots"
    );
}

#[test]
fn policy_artifact_accepts_read_only_protected_path_grants() {
    let mut artifact = valid_policy_artifact("filesystem-tool");
    artifact.commands[0].filesystem.read_roots = vec!["workspace/secrets".to_owned()];
    artifact.commands[0].filesystem.write_roots.clear();
    artifact.commands[0].filesystem.protected_path_grants =
        vec!["workspace/secrets/.env".to_owned()];

    artifact
        .validate()
        .expect("read-only protected path grants inside read scope are valid");
}

#[test]
fn policy_artifact_rejects_wildcard_protected_path_grants() {
    for grant in ["workspace/**", "workspace/*.env", "workspace/.env?"] {
        let mut artifact = valid_policy_artifact("filesystem-tool");
        artifact.commands[0].filesystem.protected_path_grants = vec![grant.to_owned()];

        let err = artifact
            .validate()
            .expect_err("protected path grants must be exact paths");

        assert_eq!(
            err.to_string(),
            format!(
                "tool filesystem-tool protected_path_grant {grant:?} must be an exact safe relative path"
            )
        );
    }
}

#[test]
fn policy_artifact_rejects_unsafe_protected_path_grants() {
    for grant in ["workspace/../.env", "/workspace/.env", "C:/workspace/.env"] {
        let mut artifact = valid_policy_artifact("filesystem-tool");
        artifact.commands[0].filesystem.protected_path_grants = vec![grant.to_owned()];

        let err = artifact
            .validate()
            .expect_err("protected path grants must be safe relative paths");

        assert_eq!(
            err.to_string(),
            format!(
                "tool filesystem-tool protected_path_grant {grant:?} must be a safe relative path"
            )
        );
    }
}

#[test]
fn policy_artifact_rejects_unsafe_filesystem_roots() {
    for root in ["/workspace", "C:/workspace", "workspace/../out"] {
        let mut artifact = valid_policy_artifact("filesystem-tool");
        artifact.commands[0].filesystem.read_roots = vec![root.to_owned()];
        artifact.commands[0]
            .filesystem
            .protected_path_grants
            .clear();

        let err = artifact
            .validate()
            .expect_err("read roots must be safe relative paths");

        assert_eq!(
            err.to_string(),
            format!("tool filesystem-tool filesystem root {root:?} must be a safe relative path")
        );
    }

    let mut artifact = valid_policy_artifact("filesystem-tool");
    artifact.commands[0].filesystem.write_roots = vec!["../out".to_owned()];
    artifact.commands[0]
        .filesystem
        .protected_path_grants
        .clear();

    let err = artifact
        .validate()
        .expect_err("write roots must be safe relative paths");

    assert_eq!(
        err.to_string(),
        "tool filesystem-tool filesystem root \"../out\" must be a safe relative path"
    );
}

#[test]
fn policy_artifact_rejects_phase_scope_unknown_tool_ids() {
    let mut artifact = valid_policy_artifact("read-file");
    artifact.phase_scope[0].tool_ids = vec!["missing-tool".to_owned()];

    let err = artifact
        .validate()
        .expect_err("phase scope must reference existing commands");

    assert_eq!(
        err.to_string(),
        "phase_scope inspect references unknown tool_id missing-tool"
    );
}

#[test]
fn policy_artifact_rejects_commands_missing_from_phase_scope() {
    let mut artifact = valid_policy_artifact("read-file");
    artifact
        .commands
        .push(valid_command_policy("write-summary"));

    let err = artifact
        .validate()
        .expect_err("every command must appear in phase scope");

    assert_eq!(
        err.to_string(),
        "command write-summary must appear in phase_scope"
    );
}

#[test]
fn expected_decision_fixtures_are_canonical_and_match_compiled_policies() {
    let registry = core_script::load_registry_root(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../loop-agent/fixtures/sandbox-negative/registry"),
    )
    .expect("sandbox-negative registry loads");

    for path in fixture_files("expected.json") {
        let text = fs::read_to_string(&path).expect("fixture is readable");
        assert!(text.ends_with('\n'), "{} must end with LF", path.display());

        let expected: ExpectedDecisionFixture =
            serde_json::from_str(&text).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
        assert_eq!(expected.expected, "deny");
        assert!(!expected.side_effects_allowed);
        assert_eq!(
            canonical_artifact_json(&expected).expect("canonical JSON"),
            text,
            "{} must be canonical",
            path.display()
        );

        let attempt = expected.attempt.as_object().expect("attempt is an object");
        let kind = json_string(attempt, "kind");
        assert_eq!(expected.reason_code.as_str(), expected_reason_code(kind));
        assert_attempt_shape(kind, attempt);

        let artifact = compile_policy_artifact(
            &expected.fixture_name,
            &registry,
            &expected.fixture_name,
            expected.target.clone(),
        )
        .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
        assert_eq!(artifact.fixture_name, expected.fixture_name);
        assert_eq!(artifact.source_loop_definition_id, expected.fixture_name);
        assert_eq!(artifact.target, expected.target);
        assert_attempt_denied(&artifact, attempt);
    }
}

#[test]
fn policy_artifact_accepts_nonempty_safe_environment_allow() {
    policy_artifact_with_environment_allow("_A1")
        .validate()
        .expect("safe nonempty environment allow entry");
}

#[test]
fn protected_path_matcher_covers_normalization_and_pattern_edges() {
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "src/main.rs",
        "workspace/src/main.rs"
    ));
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "**/.ssh/**",
        "workspace/home/user/.ssh/config"
    ));
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "**/*.env",
        "workspace/app/.env"
    ));
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "**/secret*",
        "workspace/app/secret-token"
    ));
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "**/secrets",
        "workspace/app/secrets"
    ));
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "workspace/*/id_???",
        "workspace/keys/id_rsa"
    ));
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseInsensitive,
        "**/.SSH/**",
        "workspace/home/user/.ssh/config"
    ));

    for (pattern, path) in [
        ("", "workspace/app/.env"),
        ("/absolute", "workspace/app/.env"),
        ("bad$pattern", "workspace/app/.env"),
        ("bad/**suffix", "workspace/app/.env"),
        (".", "workspace/app/.env"),
        ("..", "workspace/app/.env"),
        ("C:/secret", "workspace/app/.env"),
        ("**/.env", ""),
        ("**/.env", "/absolute"),
        ("**/.env", "workspace/$secret"),
        ("**/.env", "workspace/../secret"),
    ] {
        assert!(
            !protected_path_pattern_matches(ProtectedPathMatchMode::CaseSensitive, pattern, path),
            "{pattern:?} must not match {path:?}"
        );
    }
}

#[test]
fn default_protected_paths_have_behavioral_denial_examples() {
    let cases = [
        ("**/*.env", "workspace/app/config.env"),
        ("**/*.key", "workspace/keys/service.key"),
        ("**/*.local", "workspace/app/settings.local"),
        ("**/*.p12", "workspace/certs/client.p12"),
        ("**/*.pem", "workspace/certs/client.pem"),
        ("**/*.pfx", "workspace/certs/client.pfx"),
        ("**/.aws", "workspace/home/user/.aws"),
        ("**/.aws/**", "workspace/home/user/.aws/config"),
        ("**/.azure", "workspace/home/user/.azure"),
        ("**/.azure/**", "workspace/home/user/.azure/token"),
        ("**/.config/gcloud", "workspace/home/user/.config/gcloud"),
        (
            "**/.config/gcloud/**",
            "workspace/home/user/.config/gcloud/application_default_credentials.json",
        ),
        ("**/.config/gh", "workspace/home/user/.config/gh"),
        (
            "**/.config/gh/**",
            "workspace/home/user/.config/gh/hosts.yml",
        ),
        ("**/.docker", "workspace/home/user/.docker"),
        ("**/.docker/**", "workspace/home/user/.docker/config.json"),
        ("**/.env", "workspace/app/.env"),
        ("**/.env.*", "workspace/app/.env.production"),
        ("**/.git", "workspace/project/.git"),
        (
            "**/.git-credentials",
            "workspace/home/user/.git-credentials",
        ),
        ("**/.git/**", "workspace/project/.git/config"),
        ("**/.gnupg", "workspace/home/user/.gnupg"),
        (
            "**/.gnupg/**",
            "workspace/home/user/.gnupg/private-keys-v1.d/key",
        ),
        ("**/.kube", "workspace/home/user/.kube"),
        ("**/.kube/**", "workspace/home/user/.kube/config"),
        ("**/.loop", "workspace/project/.loop"),
        ("**/.loop/**", "workspace/project/.loop/sessions/run.jsonl"),
        ("**/.netrc", "workspace/home/user/.netrc"),
        ("**/.npmrc", "workspace/project/.npmrc"),
        ("**/.pypirc", "workspace/home/user/.pypirc"),
        ("**/.ssh", "workspace/home/user/.ssh"),
        ("**/.ssh/**", "workspace/home/user/.ssh/config"),
        ("**/credentials", "workspace/app/credentials"),
        ("**/credentials.toml", "workspace/app/credentials.toml"),
        ("**/credentials/**", "workspace/app/credentials/token"),
        ("**/id_dsa", "workspace/keys/id_dsa"),
        ("**/id_ecdsa", "workspace/keys/id_ecdsa"),
        ("**/id_ecdsa_sk", "workspace/keys/id_ecdsa_sk"),
        ("**/id_ed25519", "workspace/keys/id_ed25519"),
        ("**/id_ed25519_sk", "workspace/keys/id_ed25519_sk"),
        ("**/id_rsa", "workspace/keys/id_rsa"),
        ("**/secrets", "workspace/app/secrets"),
        ("**/secrets/**", "workspace/app/secrets/prod.txt"),
    ];

    assert_eq!(cases.len(), DEFAULT_PROTECTED_PATHS.len());
    for pattern in DEFAULT_PROTECTED_PATHS {
        assert!(
            cases
                .iter()
                .any(|(case_pattern, _path)| case_pattern == pattern),
            "missing behavioral example for {pattern}"
        );
    }
    for (pattern, path) in cases {
        assert!(
            protected_path_pattern_matches(ProtectedPathMatchMode::CaseSensitive, pattern, path),
            "{pattern} must deny {path}"
        );
    }
}

proptest! {
    #[test]
    fn protected_path_double_star_matches_any_depth(
        segments in prop::collection::vec("[a-z0-9][a-z0-9_-]{0,7}", 0..6)
            .prop_filter("portable path components", |segments| {
                segments
                    .iter()
                    .all(|segment| !core_script::relative_path_has_windows_alias(segment))
            })
    ) {
        let middle = if segments.is_empty() {
            String::new()
        } else {
            format!("{}/", segments.join("/"))
        };
        let path = format!("workspace/{middle}target.key");

        prop_assert!(protected_path_pattern_matches(
            ProtectedPathMatchMode::CaseSensitive,
            "workspace/**/target.key",
            &path
        ));
        prop_assert!(protected_path_pattern_matches(
            ProtectedPathMatchMode::CaseSensitive,
            "**/*.key",
            &path
        ));
    }

    #[test]
    fn protected_path_segment_wildcards_do_not_cross_slashes(
        middle in "[a-z0-9][a-z0-9_-]{0,7}"
    ) {
        let same_segment = format!("workspace/secret-{middle}.pem");
        let nested_segment = format!("workspace/secret-{middle}/pem");

        prop_assert!(protected_path_pattern_matches(
            ProtectedPathMatchMode::CaseSensitive,
            "workspace/secret-*.pem",
            &same_segment
        ));
        prop_assert!(!protected_path_pattern_matches(
            ProtectedPathMatchMode::CaseSensitive,
            "workspace/secret-*.pem",
            &nested_segment
        ));
    }

}

#[test]
fn policy_compile_error_messages_and_sources_cover_variants() {
    let missing_loop = PolicyCompileError::MissingLoop("missing-loop".to_owned());
    assert_eq!(
        missing_loop.to_string(),
        "policy compile references missing loop missing-loop"
    );
    assert!(std::error::Error::source(&missing_loop).is_none());

    let missing_phase = PolicyCompileError::MissingPhase("missing-phase".to_owned());
    assert_eq!(
        missing_phase.to_string(),
        "policy compile references missing phase missing-phase"
    );

    let missing_tool = PolicyCompileError::MissingTool("missing-tool".to_owned());
    assert_eq!(
        missing_tool.to_string(),
        "policy compile references missing tool missing-tool"
    );

    let depth = PolicyCompileError::LoopDepthExceeded {
        loop_id: "loop-064".to_owned(),
        depth: core_script::MAX_LOOP_NESTING_DEPTH + 1,
        max: core_script::MAX_LOOP_NESTING_DEPTH,
    };
    assert_eq!(
        depth.to_string(),
        "policy compile loop nesting depth 65 for loop-064 exceeds max 64"
    );
    assert!(std::error::Error::source(&depth).is_none());

    let network = PolicyCompileError::NonEmptyNetworkAllowlist {
        tool_id: "network-tool".to_owned(),
    };
    assert_eq!(
        network.to_string(),
        "supported policy-artifact target for tool network-tool must use a deny-all network allowlist"
    );

    let mut artifact = valid_policy_artifact("invalid-artifact");
    artifact.policy_version = "1".to_owned();
    let validation = artifact.validate().expect_err("invalid artifact");
    let invalid = PolicyCompileError::InvalidArtifact(validation);
    assert_eq!(
        invalid.to_string(),
        "policy_version must be fixed string \"0\""
    );
    assert!(std::error::Error::source(&invalid).is_some());
}

#[test]
fn policy_compile_rejects_deep_loop_chains() {
    compile_policy_artifact(
        "max-depth",
        &loop_chain_registry(core_script::MAX_LOOP_NESTING_DEPTH),
        "loop-000",
        PolicyTarget::LinuxLandlockSeccomp,
    )
    .expect("max loop nesting depth is accepted");

    let err = compile_policy_artifact(
        "too-deep",
        &loop_chain_registry(core_script::MAX_LOOP_NESTING_DEPTH + 1),
        "loop-000",
        PolicyTarget::LinuxLandlockSeccomp,
    )
    .expect_err("loop nesting above the max is rejected");

    assert!(matches!(
        err,
        PolicyCompileError::LoopDepthExceeded {
            loop_id,
            depth,
            max,
        } if loop_id == format!("loop-{:03}", core_script::MAX_LOOP_NESTING_DEPTH)
            && depth == core_script::MAX_LOOP_NESTING_DEPTH + 1
            && max == core_script::MAX_LOOP_NESTING_DEPTH
    ));
}

#[test]
fn policy_artifact_rejects_duplicate_command_tool_ids() {
    let mut artifact = valid_policy_artifact("duplicate-tool");
    artifact
        .commands
        .push(valid_command_policy("duplicate-tool"));

    let err = artifact
        .validate()
        .expect_err("duplicate command tool_id must fail validation");

    assert_eq!(err.to_string(), "duplicate command tool_id duplicate-tool");
}

#[test]
fn policy_artifact_rejects_own_script_executable_mismatch() {
    let mut artifact = own_script_policy_artifact("write-summary");
    artifact.commands[0].executable = "registry:agent-echo".to_owned();

    let err = artifact
        .validate()
        .expect_err("own-script executable mismatch must fail validation");

    assert_eq!(
        err.to_string(),
        "own-script tool write-summary executable must be runner:posix-sh"
    );
}

#[test]
fn policy_artifact_rejects_parameter_constraint_mismatches() {
    for parameter in [
        valid_parameter("--count", ParameterValueType::Integer),
        valid_parameter("--dry-run", ParameterValueType::None),
    ] {
        policy_artifact_with_parameter(parameter)
            .validate()
            .expect("valid parameter constraints");
    }

    let mut cases = Vec::new();

    let mut string_with_values = valid_parameter("--name", ParameterValueType::String);
    string_with_values.allowed_values = vec!["alice".to_owned()];
    cases.push((
        string_with_values,
        "tool parameter-tool non-enum parameter --name must omit allowed_values",
    ));

    let mut string_with_range = valid_parameter("--name", ParameterValueType::String);
    string_with_range.min = Some(1);
    cases.push((
        string_with_range,
        "tool parameter-tool string parameter --name must omit min and max",
    ));

    let mut enum_with_string_constraints = valid_parameter("--mode", ParameterValueType::Enum);
    enum_with_string_constraints.value_pattern = Some("[a-z]+".to_owned());
    enum_with_string_constraints.max_length = Some(16);
    cases.push((
            enum_with_string_constraints,
            "tool parameter-tool enum parameter --mode must omit value_pattern, max_length, min, and max",
        ));

    let mut enum_with_range = valid_parameter("--mode", ParameterValueType::Enum);
    enum_with_range.min = Some(1);
    cases.push((
            enum_with_range,
            "tool parameter-tool enum parameter --mode must omit value_pattern, max_length, min, and max",
        ));

    let mut integer_with_values = valid_parameter("--count", ParameterValueType::Integer);
    integer_with_values.allowed_values = vec!["1".to_owned()];
    cases.push((
        integer_with_values,
        "tool parameter-tool non-enum parameter --count must omit allowed_values",
    ));

    let mut integer_with_pattern = valid_parameter("--count", ParameterValueType::Integer);
    integer_with_pattern.value_pattern = Some("[0-9]+".to_owned());
    cases.push((
        integer_with_pattern,
        "tool parameter-tool integer parameter --count must omit value_pattern and max_length",
    ));

    let mut integer_with_bad_range = valid_parameter("--count", ParameterValueType::Integer);
    integer_with_bad_range.min = Some(10);
    integer_with_bad_range.max = Some(1);
    cases.push((
        integer_with_bad_range,
        "tool parameter-tool integer parameter --count min must be <= max",
    ));

    let mut none_with_values = valid_parameter("--dry-run", ParameterValueType::None);
    none_with_values.allowed_values = vec!["true".to_owned()];
    cases.push((
        none_with_values,
        "tool parameter-tool non-enum parameter --dry-run must omit allowed_values",
    ));

    let mut none_with_string_constraints = valid_parameter("--dry-run", ParameterValueType::None);
    none_with_string_constraints.value_pattern = Some("^(true|false)$".to_owned());
    none_with_string_constraints.max_length = Some(5);
    cases.push((
            none_with_string_constraints,
            "tool parameter-tool none parameter --dry-run must omit value_pattern, max_length, min, and max",
        ));

    let mut path_with_values = valid_parameter("--path", ParameterValueType::WorkspaceRelativePath);
    path_with_values.allowed_values = vec!["out/summary.txt".to_owned()];
    cases.push((
        path_with_values,
        "tool parameter-tool non-enum parameter --path must omit allowed_values",
    ));

    let mut path_with_range = valid_parameter("--path", ParameterValueType::WorkspaceRelativePath);
    path_with_range.value_pattern = Some("^[A-Za-z0-9_./-]+$".to_owned());
    path_with_range.max_length = Some(128);
    path_with_range.min = Some(1);
    cases.push((
        path_with_range,
        "tool parameter-tool workspace-relative-path parameter --path must omit min and max",
    ));

    for (parameter, expected) in cases {
        let artifact = policy_artifact_with_parameter(parameter);

        let err = artifact
            .validate()
            .expect_err("invalid parameter constraint must fail validation");

        assert_eq!(err.to_string(), expected);
    }
}

#[test]
fn policy_artifact_canonical_json_rejects_normalized_duplicate_keys() {
    let value = serde_json::json!({
        "é": 1,
        "e\u{301}": 2,
    });

    let err =
        canonical_artifact_json(&value).expect_err("normalized duplicate object keys must fail");

    assert_eq!(
        err.to_string(),
        "failed to serialize canonical policy artifact JSON: normalized object key collision: é"
    );
    assert!(std::error::Error::source(&err).is_some());
}

#[test]
fn policy_artifact_canonical_json_sorts_schema_arrays() {
    let artifact = PolicyArtifact {
        commands: vec![
            command_policy("z-tool", vec!["z", "a"], vec!["workspace/z", "workspace/a"]),
            command_policy(
                "a-tool",
                vec!["beta", "alpha"],
                vec!["workspace/b", "workspace/a"],
            ),
        ],
        fixture_name: "sort-contract".to_owned(),
        phase_scope: vec![
            PhaseScope {
                phase_id: "phase-z".to_owned(),
                tool_ids: vec!["z-tool".to_owned(), "a-tool".to_owned()],
            },
            PhaseScope {
                phase_id: "phase-a".to_owned(),
                tool_ids: vec!["z-tool".to_owned(), "a-tool".to_owned()],
            },
        ],
        policy_version: POLICY_VERSION_V0.to_owned(),
        runtime_limits: RuntimeLimits {
            headless: true,
            timeout_ms: 1000,
        },
        source_loop_definition_id: "sort-loop".to_owned(),
        target: PolicyTarget::LinuxLandlockSeccomp,
    };

    let json = canonical_artifact_json(&artifact).expect("canonical JSON");
    let canonical: PolicyArtifact =
        serde_json::from_str(&json).expect("canonical artifact deserializes");

    assert_eq!(
        canonical
            .commands
            .iter()
            .map(|command| command.tool_id.as_str())
            .collect::<Vec<_>>(),
        vec!["a-tool", "z-tool"]
    );
    assert_eq!(
        canonical.commands[0]
            .allowed_parameters
            .iter()
            .map(|param| param.name.as_str())
            .collect::<Vec<_>>(),
        vec!["--alpha", "--beta"]
    );
    assert_eq!(
        canonical.commands[0].allowed_parameters[1].allowed_values,
        vec!["alpha", "beta"]
    );
    assert_eq!(
        canonical.commands[0].filesystem.read_roots,
        vec!["workspace/a", "workspace/b"]
    );
    assert_eq!(
        canonical.commands[0].filesystem.protected_path_grants,
        vec!["workspace/a.env", "workspace/z.env"]
    );
    assert_eq!(
        canonical.commands[0].filesystem.protected_paths,
        vec!["**/.env", "**/.ssh"]
    );
    assert_eq!(
        canonical.commands[0].filesystem.write_roots,
        vec!["workspace/a-out", "workspace/z-out"]
    );
    assert_eq!(canonical.commands[0].network.allow[0].cidr, "10.0.0.0/24");
    assert_eq!(
        canonical.commands[0].environment.allow,
        vec!["LANG", "TERM"]
    );
    assert_eq!(
        canonical
            .phase_scope
            .iter()
            .map(|phase| phase.phase_id.as_str())
            .collect::<Vec<_>>(),
        vec!["phase-a", "phase-z"]
    );
    assert_eq!(canonical.phase_scope[0].tool_ids, vec!["a-tool", "z-tool"]);
}

fn command_policy(
    tool_id: &str,
    allowed_values: Vec<&str>,
    read_roots: Vec<&str>,
) -> CommandPolicy {
    CommandPolicy {
        allowed_parameters: vec![
            AllowedParameterPolicy {
                name: "--beta".to_owned(),
                required: false,
                max: None,
                max_length: None,
                min: None,
                value_pattern: None,
                value_type: ParameterValueType::Enum,
                allowed_values: allowed_values
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect(),
            },
            AllowedParameterPolicy {
                name: "--alpha".to_owned(),
                required: true,
                max: None,
                max_length: Some(128),
                min: None,
                value_pattern: Some("[a-z]+".to_owned()),
                value_type: ParameterValueType::String,
                allowed_values: Vec::new(),
            },
        ],
        argv: vec!["--second".to_owned(), "--first".to_owned()],
        command_id: format!("{tool_id}-command"),
        environment: EnvironmentPolicy {
            allow: vec!["TERM".to_owned(), "LANG".to_owned()],
            default: EnvironmentDefault::Clear,
        },
        executable: format!("/bin/{tool_id}"),
        filesystem: FilesystemPolicy {
            protected_path_grants: vec!["workspace/z.env".to_owned(), "workspace/a.env".to_owned()],
            protected_paths: vec!["**/.ssh".to_owned(), "**/.env".to_owned()],
            read_roots: read_roots.iter().map(|root| (*root).to_owned()).collect(),
            write_roots: vec!["workspace/z-out".to_owned(), "workspace/a-out".to_owned()],
        },
        network: NetworkPolicy {
            allow: vec![
                NetworkAllowEntry {
                    cidr: "10.0.1.0/24".to_owned(),
                    kind: NetworkAllowKind::Cidr,
                    port: 443,
                    transport: NetworkTransport::Udp,
                },
                NetworkAllowEntry {
                    cidr: "10.0.0.0/24".to_owned(),
                    kind: NetworkAllowKind::Cidr,
                    port: 80,
                    transport: NetworkTransport::Tcp,
                },
            ],
            default: NetworkDefault::Deny,
        },
        script_runtime: None,
        tool_id: tool_id.to_owned(),
        tool_kind: ToolKind::PredefinedCommand,
    }
}

fn policy_artifact_with_environment_allow(name: &str) -> PolicyArtifact {
    let mut artifact = valid_policy_artifact("environment-tool");
    artifact.commands[0].environment.allow = vec![name.to_owned()];
    artifact
}

fn policy_artifact_with_network_allow(cidr: &str, port: u16) -> PolicyArtifact {
    let mut artifact = valid_policy_artifact("network-tool");
    artifact.commands[0].network.allow = vec![NetworkAllowEntry {
        cidr: cidr.to_owned(),
        kind: NetworkAllowKind::Cidr,
        port,
        transport: NetworkTransport::Tcp,
    }];
    artifact
}

fn valid_policy_artifact(tool_id: &str) -> PolicyArtifact {
    PolicyArtifact {
        commands: vec![valid_command_policy(tool_id)],
        fixture_name: format!("{tool_id}-fixture"),
        phase_scope: vec![PhaseScope {
            phase_id: "inspect".to_owned(),
            tool_ids: vec![tool_id.to_owned()],
        }],
        policy_version: POLICY_VERSION_V0.to_owned(),
        runtime_limits: RuntimeLimits {
            headless: true,
            timeout_ms: 1000,
        },
        source_loop_definition_id: format!("{tool_id}-loop"),
        target: PolicyTarget::LinuxLandlockSeccomp,
    }
}

fn loop_chain_registry(depth: usize) -> core_script::ResolvedRegistry {
    let loops = (0..depth)
        .map(|index| {
            let id = format!("loop-{index:03}");
            (
                id.clone(),
                core_script::LoopBlock {
                    identity: core_script::BlockIdentity {
                        id,
                        name: format!("Loop {index:03}"),
                    },
                    phase_refs: Vec::new(),
                    subloop_refs: (index + 1 < depth)
                        .then(|| format!("loop-{:03}", index + 1))
                        .into_iter()
                        .collect(),
                    connection_refs: Vec::new(),
                },
            )
        })
        .collect();
    core_script::ResolvedRegistry {
        connections: BTreeMap::new(),
        instructions: BTreeMap::new(),
        loops,
        phases: BTreeMap::new(),
        tools: BTreeMap::new(),
    }
}

fn valid_command_policy(tool_id: &str) -> CommandPolicy {
    let mut command = command_policy(tool_id, vec!["a"], vec!["workspace"]);
    command.filesystem.write_roots = vec!["workspace".to_owned()];
    command.filesystem.protected_paths = DEFAULT_PROTECTED_PATHS
        .iter()
        .map(|path| (*path).to_owned())
        .collect();
    command.network.allow.clear();
    command
}

fn policy_artifact_with_parameter(parameter: AllowedParameterPolicy) -> PolicyArtifact {
    let mut artifact = valid_policy_artifact("parameter-tool");
    artifact.commands[0].allowed_parameters = vec![parameter];
    artifact
}

fn valid_parameter(name: &str, value_type: ParameterValueType) -> AllowedParameterPolicy {
    let mut parameter = AllowedParameterPolicy {
        name: name.to_owned(),
        required: false,
        max: None,
        max_length: None,
        min: None,
        value_pattern: None,
        value_type,
        allowed_values: Vec::new(),
    };
    match &parameter.value_type {
        ParameterValueType::String => {
            parameter.value_pattern = Some("[a-z]+".to_owned());
            parameter.max_length = Some(64);
        }
        ParameterValueType::Enum => {
            parameter.allowed_values = vec!["fast".to_owned()];
        }
        ParameterValueType::Integer
        | ParameterValueType::None
        | ParameterValueType::WorkspaceRelativePath => {}
    }
    parameter
}

fn own_script_policy_artifact(tool_id: &str) -> PolicyArtifact {
    let mut artifact = valid_policy_artifact(tool_id);
    artifact.commands[0].command_id = format!("script:{tool_id}");
    artifact.commands[0].executable = "runner:posix-sh".to_owned();
    artifact.commands[0].script_runtime = Some("posix-sh".to_owned());
    artifact.commands[0].tool_kind = ToolKind::OwnScript;
    artifact
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct ExpectedDecisionFixture {
    attempt: Value,
    expected: String,
    fixture_name: String,
    reason_code: DenyReasonCode,
    side_effects_allowed: bool,
    target: PolicyTarget,
}

fn expected_reason_code(kind: &str) -> &'static str {
    match kind {
        "write" => "write_denied",
        "network" => "network_denied",
        "environment" => "environment_denied",
        "tool_out_of_phase" => "tool_out_of_phase",
        "protected_path" => "protected_path_denied",
        "symlink_escape" => "symlink_escape_denied",
        "interpreter_escape" => "interpreter_escape_denied",
        _ => panic!("unknown attempt kind {kind}"),
    }
}

fn assert_attempt_shape(kind: &str, attempt: &serde_json::Map<String, Value>) {
    let operation = || json_string(attempt, "operation");
    let fields = match kind {
        "write" => match operation() {
            "write" | "create" => &["kind", "operation", "path", "tool_id"][..],
            "rename" => &["from_path", "kind", "operation", "to_path", "tool_id"],
            other => panic!("unsupported write operation {other}"),
        },
        "network" => &["destination", "kind", "port", "tool_id", "transport"],
        "environment" => &["kind", "name", "tool_id"],
        "tool_out_of_phase" => &["kind", "phase_id", "tool_id"],
        "protected_path" => match operation() {
            "read" | "write" | "create" | "execute" => {
                &["kind", "operation", "path", "tool_id"][..]
            }
            "rename" => &["from_path", "kind", "operation", "to_path", "tool_id"],
            other => panic!("unsupported protected-path operation {other}"),
        },
        "symlink_escape" => &[
            "kind",
            "operation",
            "path",
            "symlink_path",
            "symlink_target",
            "tool_id",
        ],
        "interpreter_escape" => &["argv", "executable", "kind", "tool_id"],
        _ => panic!("unknown attempt kind {kind}"),
    };
    assert_eq!(
        attempt.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        fields.iter().copied().collect(),
        "unexpected {kind} attempt fields"
    );
    for field in fields
        .iter()
        .filter(|field| **field != "argv" && **field != "port")
    {
        json_string(attempt, field);
    }
}

fn assert_attempt_denied(artifact: &PolicyArtifact, attempt: &serde_json::Map<String, Value>) {
    let kind = json_string(attempt, "kind");
    let tool_id = json_string(attempt, "tool_id");
    if kind == "tool_out_of_phase" {
        let phase_id = json_string(attempt, "phase_id");
        assert!(artifact.phase_scope.iter().any(|phase| {
            phase.phase_id == phase_id && !phase.tool_ids.iter().any(|id| id == tool_id)
        }));
        return;
    }

    let command = artifact
        .commands
        .iter()
        .find(|command| command.tool_id == tool_id)
        .expect("attempt tool has a compiled command");
    match kind {
        "write" => assert!(attempt_paths(attempt).into_iter().any(|path| {
            let Some(path) = workspace_path(path) else {
                return true;
            };
            !command.filesystem.write_roots.iter().any(|root| {
                core_script::normalize_safe_relative_path(root)
                    .is_some_and(|root| core_script::relative_path_is_inside_scope(&path, &root))
            })
        })),
        "network" => {
            assert!(command.network.allow.is_empty());
            let port = attempt
                .get("port")
                .and_then(Value::as_u64)
                .expect("network port is an integer");
            assert!(port > 0 && u16::try_from(port).is_ok());
            serde_json::from_value::<NetworkTransport>(attempt["transport"].clone())
                .expect("network transport is valid");
        }
        "environment" => assert!(
            !command
                .environment
                .allow
                .iter()
                .any(|name| name == json_string(attempt, "name"))
        ),
        "protected_path" => {
            let mode = protected_path_match_mode_for_policy_target(&artifact.target);
            assert!(attempt_paths(attempt).into_iter().any(|path| {
                let Some(path) = workspace_path(path) else {
                    return false;
                };
                !command
                    .filesystem
                    .protected_path_grants
                    .iter()
                    .any(|grant| workspace_path(grant).as_deref() == Some(path.as_str()))
                    && command
                        .filesystem
                        .protected_paths
                        .iter()
                        .any(|pattern| protected_path_pattern_matches(mode, pattern, &path))
            }));
        }
        "symlink_escape" => assert!(
            json_string(attempt, "symlink_target").starts_with('/')
                || core_script::normalize_safe_relative_path(json_string(
                    attempt,
                    "symlink_target"
                ))
                .is_none()
        ),
        "interpreter_escape" => {
            let argv = serde_json::from_value::<Vec<String>>(attempt["argv"].clone())
                .expect("interpreter argv is valid");
            assert!(
                command.executable != json_string(attempt, "executable") || command.argv != argv
            );
        }
        _ => unreachable!(),
    }
}

fn json_string<'a>(object: &'a serde_json::Map<String, Value>, field: &str) -> &'a str {
    object
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{field} must be a string"))
}

fn attempt_paths(attempt: &serde_json::Map<String, Value>) -> Vec<&str> {
    ["path", "from_path", "to_path"]
        .into_iter()
        .filter_map(|field| attempt.get(field).and_then(Value::as_str))
        .collect()
}

fn workspace_path(path: &str) -> Option<String> {
    let path = core_script::normalize_safe_relative_path(path)?;
    Some(if path == "workspace" || path.starts_with("workspace/") {
        path
    } else {
        format!("workspace/{path}")
    })
}

fn fixture_files(suffix: &str) -> Vec<std::path::PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut files = Vec::new();
    collect_fixture_files(&root, suffix, &mut files);
    files.sort();
    assert!(!files.is_empty(), "expected at least one {suffix} fixture");
    files
}

fn collect_fixture_files(dir: &Path, suffix: &str, out: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|err| panic!("{}: {err}", dir.display())) {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_fixture_files(&path, suffix, out);
        } else if path.to_string_lossy().ends_with(suffix) {
            out.push(path);
        }
    }
}
