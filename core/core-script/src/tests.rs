use super::*;
use proptest::prelude::*;
use std::path::Path;

fn registry_location(root: &Path) -> (&Path, &Path) {
    (
        root.parent().expect("registry has a parent"),
        Path::new(root.file_name().expect("registry has a name")),
    )
}

fn load_registry(root: impl AsRef<Path>) -> Result<ResolvedRegistry, RegistryError> {
    let (workspace, registry_root) = registry_location(root.as_ref());
    load_loop_registry_from_workspace(workspace, registry_root, "root")
}

fn collect_registry_files(root: &Path) -> Result<(RegistryRoot, Vec<RegistryFile>), RegistryError> {
    let (workspace, registry_root) = registry_location(root);
    let opened_root = open_registry_root(workspace, registry_root)?;
    let limits = RegistryTraversalLimits {
        max_file_bytes: MAX_REGISTRY_FILE_BYTES,
        max_total_bytes: MAX_REGISTRY_TOTAL_BYTES,
        max_entries: MAX_REGISTRY_ENTRIES,
        max_depth: MAX_REGISTRY_TRAVERSAL_DEPTH,
    };
    let mut state = RegistryTraversalState::default();
    let mut files = Vec::new();
    collect_registry_files_with_limits(
        &opened_root,
        &opened_root.dir,
        Path::new(""),
        &mut files,
        limits,
        0,
        &mut state,
    )?;
    Ok((opened_root, files))
}

proptest! {
    #[test]
    fn safe_relative_paths_accept_generated_literal_segments(
        segments in prop::collection::vec("[a-z0-9][a-z0-9_-]{0,7}", 1..8)
            .prop_filter("portable path components", |segments| {
                segments
                    .iter()
                    .all(|segment| !relative_path_has_windows_alias(segment))
            })
    ) {
        let path = segments.join("/");
        let normalized = normalize_safe_relative_path(&path);
        let child = format!("{}/leaf", path);
        let sibling = format!("{}x/leaf", path);

        prop_assert_eq!(normalized, Some(path.clone()));
        prop_assert!(relative_path_is_inside_scope(&child, &path));
        prop_assert!(!relative_path_is_inside_scope(&sibling, &path));
    }

    #[test]
    fn safe_relative_paths_reject_nonportable_components(
        prefix in prop::collection::vec("[a-z0-9][a-z0-9_-]{0,7}", 0..4),
        suffix in prop::collection::vec("[a-z0-9][a-z0-9_-]{0,7}", 0..4),
        bad in prop_oneof![
            Just(".".to_owned()),
            Just("..".to_owned()),
            Just("CON".to_owned()),
            Just("NUL.txt".to_owned()),
            Just("COM1".to_owned()),
            Just("COM¹".to_owned()),
            Just("com².txt".to_owned()),
            Just("LPT³.tar.gz".to_owned()),
            Just("trail.".to_owned()),
            Just("trail ".to_owned()),
            prop::sample::select(vec!['<', '>', ':', '"', '|', '?', '*', '\u{1}'])
                .prop_map(|character| format!("bad{character}name")),
        ],
    ) {
        let mut segments = prefix;
        segments.push(bad);
        segments.extend(suffix);
        let path = segments.join("/");

        prop_assert_eq!(normalize_safe_relative_path(&path), None);
    }

    #[test]
    fn parser_never_panics_on_arbitrary_utf8(
        source in prop::collection::vec(any::<char>(), 0..1024)
            .prop_map(|chars| chars.into_iter().collect::<String>())
    ) {
        let _ = parse_registry_block("property.yaml", &source);
    }
}

#[test]
fn registry_loader_resolves_hello_loop_refs_and_canonical_output() {
    let workspace =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../loop-agent/fixtures/hello-loop");
    let registry =
        load_loop_registry_from_workspace(&workspace, Path::new("registry"), "hello-loop")
            .expect("hello-loop registry loads");

    assert!(registry.loop_block("hello-loop").is_some());
    assert!(registry.loop_block("HelloLoop").is_some());
    assert_eq!(
        registry
            .phase_block("inspect")
            .expect("inspect phase")
            .tool_refs,
        vec!["read-file"]
    );
    assert!(registry.tool_block("ReadFile").is_some());
    assert!(registry.instruction_block("InspectInput").is_some());
    assert!(registry.connection_block("InspectData").is_some());

    let canonical = registry
        .canonical_json()
        .expect("resolved registry serializes");
    assert_eq!(
        canonical,
        registry.canonical_json().expect("canonical output repeats")
    );
    assert!(canonical.contains("\"hello-loop\""));
    assert!(canonical.contains("\"write-summary\""));
}

#[test]
fn loop_registry_retains_the_unique_transitive_definition_closure() {
    let root = temp_registry_dir("scoped-registry");
    for (path, source) in [
        (
            "root.yaml",
            "loop:\n  id: root\n  name: Root\n  phase_refs: [shared-phase]\n  subloop_refs: [child, child]\n  connection_refs: [tool-link]\n",
        ),
        (
            "child.yaml",
            "loop:\n  id: child\n  name: Child\n  phase_refs: [shared-phase]\n  subloop_refs: []\n  connection_refs: []\n",
        ),
        (
            "phase.yaml",
            "phase:\n  id: shared-phase\n  name: SharedPhase\n  instruction_refs: [shared-instruction]\n  tool_refs: []\n  steps:\n    - id: run\n      name: Run\n",
        ),
        (
            "instruction.yaml",
            "instruction:\n  id: shared-instruction\n  name: SharedInstruction\n  prompt: Shared\n",
        ),
        (
            "connection.yaml",
            "connection:\n  id: tool-link\n  name: ToolLink\n  connection_kind: data\n  from_ref: shared-instruction\n  to_ref: endpoint-tool\n",
        ),
        (
            "tool.yaml",
            "tool:\n  id: endpoint-tool\n  name: EndpointTool\n  tool_kind: predefined-command\n  command:\n    command_id: read-file\n    argv: []\n  allowed_parameters: []\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network: deny\n",
        ),
        (
            "unused.yaml",
            "instruction:\n  id: unused\n  name: Unused\n  prompt: Not retained\n",
        ),
    ] {
        std::fs::write(root.join(path), source).expect("registry block written");
    }
    let (workspace, registry_root) = registry_location(&root);

    let registry = load_loop_registry_from_workspace(workspace, registry_root, "Root")
        .expect("reachable registry loads by root name");
    let value: Value = serde_json::from_str(
        &registry
            .canonical_json()
            .expect("scoped registry canonicalizes"),
    )
    .expect("canonical registry is JSON");

    assert!(registry.loop_block("root").is_some());
    assert!(registry.loop_block("child").is_some());
    assert!(registry.tool_block("endpoint-tool").is_some());
    assert!(registry.instruction_block("unused").is_none());
    assert_eq!(
        value["loops"].as_object().map(serde_json::Map::len),
        Some(2)
    );
    assert_eq!(
        value["instructions"].as_object().map(serde_json::Map::len),
        Some(1),
        "one definition is retained when root and repeated subloops share it"
    );
}

#[test]
fn loop_registry_reports_a_missing_reachable_definition_from_its_owner() {
    let root = temp_registry_dir("scoped-registry-missing-reference");
    std::fs::write(
        root.join("root.yaml"),
        "loop:\n  id: root\n  name: Root\n  phase_refs: [missing]\n  subloop_refs: []\n  connection_refs: []\n",
    )
    .expect("root loop written");

    let error = load_registry(&root).expect_err("missing reachable phase is rejected");

    assert!(matches!(
        error,
        RegistryError::MissingReference {
            from_kind: "loop",
            from_id,
            reference_kind: "phase",
            reference,
        } if from_id == "root" && reference == "missing"
    ));
}

#[test]
fn parser_rejects_oversized_names_and_definition_text() {
    let oversized_name = "x".repeat(MAX_BLOCK_NAME_CHARS + 1);
    let name_error = parse_registry_block(
        "long-name.yaml",
        &format!("instruction:\n  id: long-name\n  name: {oversized_name}\n  prompt: Valid\n"),
    )
    .expect_err("oversized block name rejected");
    assert!(name_error.to_string().contains("name"));

    let oversized_text = "x".repeat(MAX_REGISTRY_DEFINITION_BYTES + 1);
    for (source_name, source) in [
        (
            "long-prompt.yaml",
            format!(
                "instruction:\n  id: long-prompt\n  name: LongPrompt\n  prompt: {oversized_text}\n"
            ),
        ),
        (
            "long-script.yaml",
            format!(
                "tool:\n  id: long-script\n  name: LongScript\n  tool_kind: own-script\n  command: script:long-script\n  script_runtime: posix-sh\n  script_body: {oversized_text}\n  allowed_parameters: []\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network: deny\n"
            ),
        ),
    ] {
        let error = parse_registry_block(source_name, &source)
            .expect_err("oversized definition text rejected");
        assert!(error.to_string().contains("maximum"), "{error}");
    }

    let oversized_multibyte = "é".repeat(MAX_REGISTRY_DEFINITION_BYTES / 2 + 1);
    let error = parse_registry_block(
        "multibyte-prompt.yaml",
        &format!(
            "instruction:\n  id: multibyte-prompt\n  name: MultibytePrompt\n  prompt: {oversized_multibyte}\n"
        ),
    )
    .expect_err("definition limits count UTF-8 bytes");
    assert!(error.to_string().contains("maximum"), "{error}");
}

#[test]
fn loop_registry_rejects_a_definition_closure_above_its_byte_budget() {
    let root = temp_registry_dir("active-registry-limit");
    let loop_source = "loop:\n  id: root\n  name: Root\n  phase_refs: [phase]\n  subloop_refs: []\n  connection_refs: []\n";
    let phase_source = "phase:\n  id: phase\n  name: Phase\n  instruction_refs: [instruction]\n  tool_refs: []\n  steps:\n    - id: run\n      name: Run\n";
    let instruction_source =
        "instruction:\n  id: instruction\n  name: Instruction\n  prompt: Retained\n";
    for (path, source) in [
        ("loop.yaml", loop_source),
        ("phase.yaml", phase_source),
        ("instruction.yaml", instruction_source),
    ] {
        std::fs::write(root.join(path), source).expect("registry block written");
    }
    let (workspace, registry_root) = registry_location(&root);
    let max_active =
        u64::try_from(loop_source.len() + phase_source.len()).expect("test sources fit in u64");

    let error = ResolvedRegistry::load_for_loop_with_limits(
        workspace,
        registry_root,
        "root",
        1024,
        4096,
        max_active,
    )
    .expect_err("closure above active byte budget rejected");

    assert!(matches!(
        error,
        RegistryError::ReadLimitExceeded { path, bytes, max }
            if path == root && bytes > max && max == max_active
    ));
}

#[test]
fn registry_loader_enforces_workspace_boundary_and_reports_missing_workspace() {
    let workspace = temp_registry_dir("registry-workspace-boundary");
    let escaping_root = Path::new("../outside");
    let err = load_loop_registry_from_workspace(&workspace, escaping_root, "root")
        .expect_err("registry root must stay within workspace");
    assert!(
        matches!(&err, RegistryError::UnsafePath { path, .. } if path == escaping_root),
        "unexpected error: {err:?}"
    );
    assert!(err.to_string().contains("stay within the workspace"));

    let missing_workspace = workspace.join("missing-workspace");
    let err = load_loop_registry_from_workspace(&missing_workspace, Path::new("registry"), "root")
        .expect_err("missing workspace must remain an I/O failure");
    assert!(
        matches!(&err, RegistryError::Io { path, source }
            if path.as_path() == missing_workspace && source.kind() == std::io::ErrorKind::NotFound),
        "unexpected error: {err:?}"
    );
}

#[test]
fn registry_loader_accepts_nested_yaml_files_and_ignores_non_registry_files() {
    let root = temp_registry_dir("nested-registry");
    std::fs::write(root.join("README.txt"), "ignored").expect("ignored file written");
    std::fs::create_dir_all(root.join("nested")).expect("nested dir created");
    std::fs::write(
        root.join("nested").join("instruction.yml"),
        "instruction:\n  id: inspect\n  name: Inspect\n  prompt: Inspect\n",
    )
    .expect("registry file written");
    std::fs::write(
        root.join("phase.yaml"),
        "phase:\n  id: phase\n  name: Phase\n  instruction_refs: [inspect]\n  tool_refs: []\n  steps:\n    - id: run\n      name: Run\n",
    )
    .expect("phase written");
    std::fs::write(
        root.join("loop.yaml"),
        "loop:\n  id: root\n  name: Root\n  phase_refs: [phase]\n  subloop_refs: []\n  connection_refs: []\n",
    )
    .expect("loop written");

    let registry = load_registry(root).expect("nested yml registry loads");

    assert!(registry.instruction_block("Inspect").is_some());
}

#[test]
fn registry_loader_rejects_files_above_read_limit() {
    let root = temp_registry_dir("registry-file-read-limit");
    std::fs::write(
        root.join("instruction.yaml"),
        "instruction:\n  id: inspect\n  name: Inspect\n  prompt: Inspect\n",
    )
    .expect("registry file written");

    let (workspace, registry_root) = registry_location(&root);
    let err = ResolvedRegistry::load_for_loop_with_limits(
        workspace,
        registry_root,
        "root",
        16,
        1024,
        1024,
    )
    .expect_err("oversized registry file is rejected before parsing");

    assert!(err.to_string().contains("registry read size"));
    assert!(matches!(
        err,
        RegistryError::ReadLimitExceeded {
            path,
            bytes,
            max: 16,
        } if path.ends_with("instruction.yaml") && bytes > 16
    ));
}

#[test]
fn registry_file_reader_enforces_limit_before_utf8_decoding() {
    let root = temp_registry_dir("registry-bounded-file-read");
    let path = root.join("instruction.yaml");
    let mut source = vec![b'a'; 17];
    source.push(0xff);
    std::fs::write(&path, source).expect("registry file written");
    let (opened_root, files) = collect_registry_files(&root).expect("registry file collected");
    assert_eq!(files.len(), 1);

    let err = read_registry_file_to_string(&opened_root, &files[0], 16)
        .expect_err("oversized registry file is rejected before decoding trailing bytes");

    assert!(matches!(
        err,
        RegistryError::ReadLimitExceeded {
            path: error_path,
            bytes: 17,
            max: 16,
        } if error_path == path
    ));
}

#[test]
fn registry_file_reader_rejects_invalid_utf8() {
    let root = temp_registry_dir("registry-invalid-utf8");
    let invalid_utf8 = root.join("invalid.yaml");
    std::fs::write(&invalid_utf8, [0xff]).expect("invalid UTF-8 registry file written");
    let (opened_root, files) = collect_registry_files(&root).expect("registry file collected");
    assert_eq!(files.len(), 1);
    let error = read_registry_file_to_string(&opened_root, &files[0], MAX_REGISTRY_FILE_BYTES)
        .expect_err("invalid UTF-8 is rejected");
    assert!(std::error::Error::source(&error).is_some());
    assert!(error.to_string().contains("invalid.yaml"));
    assert!(matches!(
        error,
        RegistryError::Io { source, .. } if source.kind() == std::io::ErrorKind::InvalidData
    ));
}

#[test]
fn registry_file_reader_reports_leaf_removed_after_collection() {
    let root = temp_registry_dir("registry-leaf-removed");
    let path = root.join("instruction.yaml");
    std::fs::write(
        &path,
        "instruction:\n  id: inspect\n  name: Inspect\n  prompt: Inspect\n",
    )
    .expect("registry file written");
    let (opened_root, files) = collect_registry_files(&root).expect("registry file collected");
    std::fs::remove_file(&path).expect("registry file removed");

    let err = read_registry_file_to_string(&opened_root, &files[0], MAX_REGISTRY_FILE_BYTES)
        .expect_err("disappearing registry file must remain an I/O failure");
    assert!(
        matches!(&err, RegistryError::Io { path: error_path, source }
            if error_path.as_path() == path && source.kind() == std::io::ErrorKind::NotFound),
        "unexpected error: {err:?}"
    );
}

#[cfg(any(unix, windows))]
#[test]
fn registry_file_reader_rejects_ancestor_replaced_by_link_after_collection() {
    let root = temp_registry_dir("registry-ancestor-swap");
    let nested = root.join("nested");
    let outside = temp_registry_dir("registry-ancestor-swap-outside");
    std::fs::create_dir(&nested).expect("nested registry directory created");
    std::fs::write(
        nested.join("instruction.yaml"),
        "instruction:\n  id: inside\n  name: Inside\n  prompt: Inside\n",
    )
    .expect("inside registry file written");
    std::fs::write(
        outside.join("instruction.yaml"),
        "instruction:\n  id: outside\n  name: Outside\n  prompt: Outside\n",
    )
    .expect("outside registry file written");
    let (opened_root, files) = collect_registry_files(&root).expect("registry file collected");
    assert_eq!(files.len(), 1);

    std::fs::rename(&nested, root.join("retired")).expect("nested directory retired");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &nested).expect("replacement symlink created");
    #[cfg(windows)]
    create_windows_junction(&nested, &outside);

    let err = read_registry_file_to_string(&opened_root, &files[0], MAX_REGISTRY_FILE_BYTES)
        .expect_err("replacement link must not be followed");
    assert!(matches!(err, RegistryError::UnsafePath { .. }));
}

#[test]
fn registry_loader_rejects_total_bytes_above_read_limit() {
    let root = temp_registry_dir("registry-total-read-limit");
    let first = "instruction:\n  id: inspect-a\n  name: InspectA\n  prompt: Inspect\n";
    let second = "instruction:\n  id: inspect-b\n  name: InspectB\n  prompt: Inspect\n";
    std::fs::write(root.join("a.yaml"), first).expect("first registry file written");
    std::fs::write(root.join("b.yaml"), second).expect("second registry file written");

    let (workspace, registry_root) = registry_location(&root);
    let err = ResolvedRegistry::load_for_loop_with_limits(
        workspace,
        registry_root,
        "root",
        1024,
        u64::try_from(first.len()).expect("test length fits u64"),
        1024,
    )
    .expect_err("registry total size is rejected before parsing all files");

    assert!(matches!(
        err,
        RegistryError::ReadLimitExceeded {
            path,
            bytes,
            max,
        } if path == root && bytes > max
    ));
}

#[test]
fn registry_loader_bounds_all_visited_entries() {
    let root = temp_registry_dir("registry-entry-count-limit");
    std::fs::write(
        root.join("a.yaml"),
        "instruction:\n  id: inspect-a\n  name: InspectA\n  prompt: Inspect\n",
    )
    .expect("registry file written");
    std::fs::write(root.join("README.txt"), "ignored").expect("non-registry entry written");

    let (workspace, registry_root) = registry_location(&root);
    let err = ResolvedRegistry::load_for_loop_with_all_limits(
        workspace,
        registry_root,
        "root",
        1024,
        RegistryTraversalLimits {
            max_file_bytes: 1024,
            max_total_bytes: 1024,
            max_entries: 1,
            max_depth: 64,
        },
    )
    .expect_err("all visited entries count toward the traversal budget");

    assert!(err.to_string().contains("registry traversal entry count"));
    assert!(matches!(
        err,
        RegistryError::TraversalLimitExceeded {
            limit: "entry count",
            observed: 2,
            max: 1,
            ..
        }
    ));
}

#[test]
fn registry_loader_rejects_directories_above_traversal_depth_limit() {
    let root = temp_registry_dir("registry-depth-limit");
    std::fs::create_dir_all(root.join("nested")).expect("nested dir created");
    std::fs::write(
        root.join("nested").join("instruction.yaml"),
        "instruction:\n  id: inspect\n  name: Inspect\n  prompt: Inspect\n",
    )
    .expect("registry file written");

    let (workspace, registry_root) = registry_location(&root);
    let err = ResolvedRegistry::load_for_loop_with_all_limits(
        workspace,
        registry_root,
        "root",
        1024,
        RegistryTraversalLimits {
            max_file_bytes: 1024,
            max_total_bytes: 1024,
            max_entries: 1024,
            max_depth: 0,
        },
    )
    .expect_err("registry traversal depth is rejected before recursion");

    assert!(matches!(
        err,
        RegistryError::TraversalLimitExceeded {
            limit: "depth",
            observed: 1,
            max: 0,
            ..
        }
    ));
}

#[test]
fn registry_reference_validation_reports_each_missing_reference_shape() {
    let mut tool = own_script_tool("write-summary", "script:write-summary");
    tool.script_body = None;
    let err = validate_tool_semantics(&tool).expect_err("script body is required");
    assert!(matches!(
        err,
        SemanticValidationError::InvalidToolDefinition { message, .. }
            if message.contains("script_body")
    ));

    let mut tool = own_script_tool("write-summary", "script:write-summary");
    tool.script_body = Some("   \n".to_owned());
    let err = validate_tool_semantics(&tool).expect_err("blank script body is rejected");
    assert!(matches!(
        err,
        SemanticValidationError::InvalidToolDefinition { message, .. }
            if message.contains("non-empty")
    ));

    let mut tool = own_script_tool("write-summary", "script:write-summary");
    tool.tool_kind = ToolKind::PredefinedCommand;
    let err = validate_tool_semantics(&tool).expect_err("tool kind must match command shape");
    assert!(err.to_string().contains("predefined-command"));
    assert!(matches!(
        err,
        SemanticValidationError::ToolCommandKindMismatch { .. }
    ));

    let mut invalid_phase = test_phase();
    invalid_phase.steps[0].id = "BadStep".to_owned();
    let invalid_step = ResolvedRegistry::from_blocks([RegistryBlock::Phase(invalid_phase)])
        .expect_err("invalid step id rejected");
    assert!(invalid_step.to_string().contains("invalid block id"));
    assert!(matches!(invalid_step, RegistryError::InvalidBlockId(value) if value == "BadStep"));

    let empty_loop = ResolvedRegistry::from_blocks([RegistryBlock::Loop(LoopBlock {
        phase_refs: Vec::new(),
        ..test_loop()
    })])
    .expect_err("empty loop phase_refs rejected");
    assert!(std::error::Error::source(&empty_loop).is_some());
    assert!(empty_loop.to_string().contains("loop.phase_refs"));

    let mut phase_with_connection = test_phase();
    phase_with_connection.steps[0]
        .connection_refs
        .push("link".to_owned());
    for (reference_kind, blocks) in [
        (
            "instruction",
            vec![RegistryBlock::Phase(PhaseBlock {
                instruction_refs: vec!["missing-instruction".to_owned()],
                ..test_phase()
            })],
        ),
        (
            "tool",
            vec![RegistryBlock::Phase(PhaseBlock {
                tool_refs: vec!["missing-tool".to_owned()],
                ..test_phase()
            })],
        ),
        (
            "phase",
            vec![RegistryBlock::Loop(LoopBlock {
                phase_refs: vec!["missing-phase".to_owned()],
                ..test_loop()
            })],
        ),
        (
            "loop",
            vec![
                simple_phase_block("phase"),
                RegistryBlock::Loop(LoopBlock {
                    subloop_refs: vec!["missing-loop".to_owned()],
                    ..test_loop()
                }),
            ],
        ),
        (
            "connection",
            vec![
                simple_phase_block("phase"),
                RegistryBlock::Loop(LoopBlock {
                    connection_refs: vec!["missing-connection".to_owned()],
                    ..test_loop()
                }),
            ],
        ),
        (
            "endpoint",
            vec![RegistryBlock::Connection(test_connection(
                "missing-endpoint",
                "also-missing",
            ))],
        ),
        (
            "step",
            vec![
                simple_phase_block("phase"),
                RegistryBlock::Connection(test_connection("phase.missing-step", "phase.step")),
            ],
        ),
        (
            "step connection",
            vec![
                RegistryBlock::Phase(phase_with_connection),
                RegistryBlock::Connection(test_connection("phase.step", "phase.step")),
                RegistryBlock::Loop(test_loop()),
            ],
        ),
    ] {
        assert_missing_reference(blocks, reference_kind);
    }
}

#[test]
fn parser_rejects_unsafe_yaml_and_unknown_fields() {
    const INSTRUCTION: &str =
        "instruction:\n  id: inspect\n  name: Inspect\n  prompt: Inspect input\n";
    let tool =
        include_str!("../../../loop-agent/fixtures/hello-loop/registry/tools/read-file.yaml");
    let phase =
        include_str!("../../../loop-agent/fixtures/hello-loop/registry/phases/inspect.yaml");
    let connection = include_str!(
        "../../../loop-agent/fixtures/hello-loop/registry/connections/inspect-data.yaml"
    );
    let loop_block =
        include_str!("../../../loop-agent/fixtures/hello-loop/registry/loops/hello-loop.yaml");
    let network_tool = tool.replace(
        "  network: deny",
        "  network:\n    default: deny\n    allow:\n      - kind: cidr\n        transport: tcp\n        cidr: 192.0.2.0/24\n        port: 443",
    );
    let cases = [
        (
            "duplicate-key.yaml",
            INSTRUCTION.replace("  prompt:", "  prompt: first\n  prompt:"),
        ),
        (
            "typed-key-collision.yaml",
            INSTRUCTION.replace("  prompt:", "  1: first\n  \"1\": second\n  prompt:"),
        ),
        (
            "anchor.yaml",
            INSTRUCTION.replace("name: Inspect", "name: &display Inspect"),
        ),
        (
            "alias.yaml",
            INSTRUCTION.replace("prompt: Inspect input", "prompt: *display"),
        ),
        (
            "merge.yaml",
            INSTRUCTION.replace("  id:", "  <<: {}\n  id:"),
        ),
        (
            "core-tag.yaml",
            INSTRUCTION.replace("prompt: Inspect input", "prompt: !!str Inspect input"),
        ),
        (
            "custom-tag.yaml",
            INSTRUCTION.replace("prompt: Inspect input", "prompt: !secret Inspect input"),
        ),
        (
            "multiple-documents.yaml",
            format!("{INSTRUCTION}---\n{INSTRUCTION}"),
        ),
        (
            "unknown-block-kind.yaml",
            "unknown:\n  id: inspect\n  name: Inspect\n".to_owned(),
        ),
        (
            "unknown-field.yaml",
            INSTRUCTION.replace("  prompt:", "  extra: true\n  prompt:"),
        ),
        (
            "unknown-nested-field.yaml",
            tool.replace("    argv: []", "    argv: []\n    extra: true"),
        ),
        (
            "unknown-parameter-field.yaml",
            tool.replace(
                "      required: true",
                "      required: true\n      extra: true",
            ),
        ),
        (
            "unknown-step-field.yaml",
            phase.replace(
                "      name: Gather",
                "      name: Gather\n      extra: true",
            ),
        ),
        (
            "unknown-network-field.yaml",
            network_tool.replace("    default:", "    extra: true\n    default:"),
        ),
        (
            "unknown-network-entry-field.yaml",
            network_tool.replace("        port:", "        extra: true\n        port:"),
        ),
        (
            "unknown-connection-field.yaml",
            connection.replace("  connection_kind:", "  extra: true\n  connection_kind:"),
        ),
        (
            "unknown-loop-field.yaml",
            loop_block.replace("  phase_refs:", "  extra: true\n  phase_refs:"),
        ),
        (
            "explicit-null.yaml",
            INSTRUCTION.replace("prompt: Inspect input", "prompt: null"),
        ),
    ];

    for (name, source) in cases {
        let error = parse_registry_block(name, &source).expect_err(name);
        assert!(error.to_string().starts_with(name), "{name}: {error}");
    }
}

#[test]
fn parser_enforces_registry_schema() {
    let tool =
        include_str!("../../../loop-agent/fixtures/hello-loop/registry/tools/read-file.yaml");
    let instruction = include_str!(
        "../../../loop-agent/fixtures/hello-loop/registry/instructions/inspect-input.yaml"
    );
    let phase =
        include_str!("../../../loop-agent/fixtures/hello-loop/registry/phases/inspect.yaml");
    let loop_block =
        include_str!("../../../loop-agent/fixtures/hello-loop/registry/loops/hello-loop.yaml");
    let cases = [
        ("invalid-id.yaml", tool.replacen("id: read-file", "id: ReadFile", 1)),
        (
            "invalid-command-id.yaml",
            tool.replace("command_id: agent-read", "command_id: AgentRead"),
        ),
        (
            "invalid-parameter-name.yaml",
            tool.replace("name: \"--file\"", "name: file"),
        ),
        (
            "duplicate-parameter-name.yaml",
            tool.replace(
                "      max_length: 128\n",
                "      max_length: 128\n    - name: \"--file\"\n      value_type: none\n      required: false\n",
            ),
        ),
        (
            "missing-string-bound.yaml",
            tool.replace("value_type: workspace-relative-path", "value_type: string")
                .replace("      max_length: 128\n", ""),
        ),
        (
            "empty-prompt.yaml",
            instruction.replace(
                "prompt: \"Read the selected input and report only deterministic facts.\"",
                "prompt: \"\"",
            ),
        ),
        (
            "empty-phase.yaml",
            phase.replace(
                "steps:\n    - id: gather\n      name: Gather\n      connection_refs: [inspect-data]",
                "steps: []",
            ),
        ),
        (
            "empty-loop.yaml",
            loop_block.replace("phase_refs: [inspect, summarize]", "phase_refs: []"),
        ),
        (
            "zero-port.yaml",
            tool.replace(
                "network: deny",
                "network:\n    default: deny\n    allow:\n      - kind: cidr\n        transport: tcp\n        cidr: 192.0.2.0/24\n        port: 0",
            ),
        ),
    ];

    for (name, source) in cases {
        let error = parse_registry_block(name, &source).expect_err(name);
        assert!(
            name != "missing-string-bound.yaml" || error.to_string().contains("value_type string"),
            "{name}: {error}"
        );
    }
}

#[test]
fn parser_enforces_document_and_depth_budgets() {
    const PREFIX: &str = "instruction:\n  id: inspect\n  name: Inspect\n  prompt: ";
    let definition = format!("{PREFIX}Inspect\n");
    let at_limit = format!(
        "{definition}#{}\n",
        "x".repeat(MAX_YAML_BYTES - definition.len() - 2)
    );
    assert_eq!(at_limit.len(), MAX_YAML_BYTES);
    assert!(parse_registry_block("at-limit.yaml", &at_limit).is_ok());

    let oversized = format!("{PREFIX}{}\n", "x".repeat(MAX_YAML_BYTES));
    assert!(parse_registry_block("oversized.yaml", &oversized).is_err());

    let nested = format!(
        "instruction:\n  id: inspect\n  name: Inspect\n  prompt: Inspect\n  extra: {}x{}\n",
        "[".repeat(MAX_YAML_DEPTH + 1),
        "]".repeat(MAX_YAML_DEPTH + 1)
    );
    assert!(parse_registry_block("deep.yaml", &nested).is_err());
}

#[cfg(unix)]
#[test]
fn registry_loader_rejects_symlinked_registry_entries() {
    use std::os::unix::fs::symlink;

    let root = temp_registry_dir("symlink-root");
    let outside = temp_registry_dir("symlink-outside");
    symlink(&outside, root.join("linked")).expect("registry symlink created");

    let err = load_registry(&root).expect_err("registry symlink must be rejected");

    assert!(
        matches!(err, RegistryError::UnsafePath { message, .. } if message.contains("symlink"))
    );
}

#[cfg(windows)]
#[test]
fn registry_loader_rejects_junction_registry_entries() {
    let root = temp_registry_dir("junction-root");
    let outside = temp_registry_dir("junction-outside");
    std::fs::write(
        outside.join("outside-tool.yaml"),
        r#"
tool:
  id: outside-tool
  name: Outside Tool
  tool_kind: own-script
  command: script:outside-tool
  script_runtime: posix-sh
  script_body: |
    echo outside
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
    )
    .expect("outside registry file written");
    create_windows_junction(&root.join("linked"), &outside);

    let err = load_registry(&root).expect_err("registry junction must be rejected");

    assert!(
        matches!(err, RegistryError::UnsafePath { ref message, .. } if message.contains("reparse")),
        "unexpected error: {err:?}"
    );
}

#[cfg(any(unix, windows))]
#[test]
fn registry_loader_rejects_linked_registry_root() {
    let parent = temp_registry_dir("linked-root-parent");
    let outside = temp_registry_dir("linked-root-target");
    let linked_root = parent.join("linked-root");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &linked_root).expect("registry root symlink created");
    #[cfg(windows)]
    create_windows_junction(&linked_root, &outside);

    let err = load_loop_registry_from_workspace(&parent, Path::new("linked-root"), "root")
        .expect_err("linked registry root must be rejected");

    assert!(
        matches!(err, RegistryError::UnsafePath { ref path, ref message }
            if path == &linked_root && (message.contains("symlink") || message.contains("reparse"))),
        "unexpected error: {err:?}"
    );
}

#[test]
fn parser_handles_block_script_bodies_and_requires_content() {
    let fixture =
        include_str!("../../../loop-agent/fixtures/hello-loop/registry/tools/write-summary.yaml");
    let literal = fixture.replace(
        "    printf '%s\\n' \"$SUMMARY\" > out/summary.txt\n",
        "    #!/bin/sh\n    write_scope: [\"literal-only\"]\n    ---\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt\n",
    );
    let block = parse_registry_block("literal-script-body.yaml", &literal)
        .expect("literal script source is opaque to registry parsing");

    let RegistryBlock::Tool(tool) = block else {
        panic!("expected tool block");
    };
    assert_eq!(tool.script_runtime, Some(ScriptRuntime::PosixSh));
    assert_eq!(
        tool.script_body.as_deref(),
        Some(
            "#!/bin/sh\nwrite_scope: [\"literal-only\"]\n---\nprintf '%s\\n' \"$SUMMARY\" > out/summary.txt\n"
        )
    );

    let duplicate = literal.replacen(
        "  write_scope: [\"workspace/out\"]\n",
        "  write_scope: [\"workspace/out\"]\n  write_scope: []\n",
        1,
    );
    let err = parse_registry_block("real-duplicate.yaml", &duplicate)
        .expect_err("a real sibling field remains a duplicate");
    assert!(err.to_string().starts_with("real-duplicate.yaml"));

    let body = "  script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt\n";
    for (name, source) in [
        ("missing-script-body.yaml", fixture.replace(body, "")),
        (
            "empty-script-body.yaml",
            fixture.replace(body, "  script_body: \"\"\n"),
        ),
    ] {
        assert!(parse_registry_block(name, &source).is_err(), "{name}");
    }

    let folded = fixture.replacen("  script_body: |", "  script_body: >", 1);
    parse_registry_block("folded-script-body.yaml", &folded)
        .expect("standard folded YAML scalars are accepted");
}

#[test]
fn parser_decodes_yaml_double_quoted_escapes() {
    let block = parse_registry_block(
        "quoted-script-body.yaml",
        r#"tool:
  id: quoted-script
  name: QuotedScript
  tool_kind: own-script
  command: script:quoted-script
  script_runtime: posix-sh
  script_body: "printf '%s\n' \"$SUMMARY\" > out/summary.txt"
  allowed_parameters: []
  read_scope: ["workspace"]
  write_scope: ["workspace/out"]
  protected_path_grants: []
  network: deny
"#,
    )
    .expect("YAML 1.2 double-quoted escapes parse");

    let RegistryBlock::Tool(tool) = block else {
        panic!("expected tool block");
    };
    assert_eq!(
        tool.script_body.as_deref(),
        Some("printf '%s\n' \"$SUMMARY\" > out/summary.txt")
    );
}

#[test]
fn connection_endpoints_reject_cross_kind_ambiguous_references() {
    let err = ResolvedRegistry::from_blocks([
        RegistryBlock::Tool(own_script_tool("build", "script:build")),
        RegistryBlock::Phase(PhaseBlock {
            identity: BlockIdentity {
                id: "build".to_owned(),
                name: "BuildPhase".to_owned(),
            },
            instruction_refs: Vec::new(),
            tool_refs: Vec::new(),
            steps: vec![StepBlock {
                id: "step".to_owned(),
                name: "Step".to_owned(),
                connection_refs: Vec::new(),
            }],
        }),
        RegistryBlock::Connection(ConnectionBlock {
            identity: BlockIdentity {
                id: "ambiguous-endpoint".to_owned(),
                name: "AmbiguousEndpoint".to_owned(),
            },
            connection_kind: ConnectionKind::Data,
            from_ref: "build".to_owned(),
            to_ref: "build.step".to_owned(),
        }),
    ])
    .expect_err("cross-kind endpoint ambiguity rejected");

    assert!(matches!(
        err,
        RegistryError::AmbiguousReference {
            kind: "endpoint",
            reference
        } if reference == "build"
    ));
}

#[test]
fn connection_endpoint_resolves_dotted_block_name_before_step_syntax() {
    let mut tool = own_script_tool("read-file", "script:read-file");
    tool.identity.name = "Read.File".to_owned();

    let registry = ResolvedRegistry::from_blocks([
        RegistryBlock::Tool(tool),
        RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: "sink".to_owned(),
                name: "Sink".to_owned(),
            },
            prompt: "Consume".to_owned(),
        }),
        RegistryBlock::Connection(ConnectionBlock {
            identity: BlockIdentity {
                id: "dotted-endpoint".to_owned(),
                name: "DottedEndpoint".to_owned(),
            },
            connection_kind: ConnectionKind::Data,
            from_ref: "Read.File".to_owned(),
            to_ref: "sink".to_owned(),
        }),
    ])
    .expect("dotted exact endpoint name resolves before phase.step syntax");

    assert_eq!(
        registry
            .tool_block("Read.File")
            .expect("dotted name resolves")
            .identity
            .id,
        "read-file"
    );
}

#[test]
fn registry_reference_validation_rejects_loop_cycles() {
    let err = ResolvedRegistry::from_blocks([
        simple_phase_block("phase"),
        RegistryBlock::Loop(LoopBlock {
            identity: BlockIdentity {
                id: "a".to_owned(),
                name: "A".to_owned(),
            },
            phase_refs: vec!["phase".to_owned()],
            subloop_refs: vec!["b".to_owned()],
            connection_refs: Vec::new(),
        }),
        RegistryBlock::Loop(LoopBlock {
            identity: BlockIdentity {
                id: "b".to_owned(),
                name: "B".to_owned(),
            },
            phase_refs: vec!["phase".to_owned()],
            subloop_refs: vec!["a".to_owned()],
            connection_refs: Vec::new(),
        }),
    ])
    .expect_err("cycle rejected");

    assert!(err.to_string().contains("loop cycle"));
    assert!(matches!(err, RegistryError::LoopCycle { .. }));
}

#[test]
fn registry_reference_validation_rejects_deep_loop_chains() {
    ResolvedRegistry::from_blocks(loop_chain_blocks(MAX_LOOP_NESTING_DEPTH))
        .expect("max loop nesting depth is accepted");

    let err = ResolvedRegistry::from_blocks(loop_chain_blocks(MAX_LOOP_NESTING_DEPTH + 1))
        .expect_err("loop nesting above the max is rejected");

    assert!(std::error::Error::source(&err).is_none());
    assert!(err.to_string().contains("loop nesting depth"));
    assert!(matches!(
        err,
        RegistryError::LoopDepthExceeded {
            loop_id,
            depth,
            max,
        } if loop_id == format!("loop-{MAX_LOOP_NESTING_DEPTH:03}")
            && depth == MAX_LOOP_NESTING_DEPTH + 1
            && max == MAX_LOOP_NESTING_DEPTH
    ));
}

#[test]
fn registry_reference_validation_counts_shared_subloop_tails_per_path() {
    let mut blocks = loop_chain_blocks(MAX_LOOP_NESTING_DEPTH);
    blocks.push(RegistryBlock::Loop(LoopBlock {
        identity: BlockIdentity {
            id: "zz-parent".to_owned(),
            name: "Parent".to_owned(),
        },
        phase_refs: vec!["chain-phase".to_owned()],
        subloop_refs: vec!["loop-000".to_owned()],
        connection_refs: Vec::new(),
    }));

    let err = ResolvedRegistry::from_blocks(blocks)
        .expect_err("shared subloop tail still counts against parent depth");

    assert!(matches!(
        err,
        RegistryError::LoopDepthExceeded {
            depth,
            max,
            ..
        } if depth == MAX_LOOP_NESTING_DEPTH + 1 && max == MAX_LOOP_NESTING_DEPTH
    ));
}

#[test]
fn registry_accepts_duplicate_subloop_tails_within_depth() {
    ResolvedRegistry::from_blocks(duplicated_subloop_tail_blocks(25))
        .expect("duplicated acyclic subloop tail validates");
}

#[test]
fn registry_rejects_ambiguous_same_kind_id_name_references() {
    let err = ResolvedRegistry::from_blocks([
        RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: "alpha".to_owned(),
                name: "Alpha".to_owned(),
            },
            prompt: "first".to_owned(),
        }),
        RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: "beta".to_owned(),
                name: "alpha".to_owned(),
            },
            prompt: "second".to_owned(),
        }),
    ])
    .expect_err("ambiguous same-kind id/name reference rejected");

    assert!(err.to_string().contains("ambiguous instruction reference"));
    assert!(matches!(
        err,
        RegistryError::AmbiguousReference {
            kind: "instruction",
            reference,
        } if reference == "alpha"
    ));
}

#[test]
fn registry_rejects_duplicate_ids_and_ids_that_shadow_names() {
    let duplicate_id = ResolvedRegistry::from_blocks([
        RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: "same".to_owned(),
                name: "First".to_owned(),
            },
            prompt: "first".to_owned(),
        }),
        RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: "same".to_owned(),
                name: "Second".to_owned(),
            },
            prompt: "second".to_owned(),
        }),
    ])
    .expect_err("duplicate block ids must fail");

    assert!(duplicate_id.to_string().contains("duplicate instruction"));
    assert!(matches!(
        duplicate_id,
        RegistryError::DuplicateId {
            kind: "instruction",
            id,
        } if id == "same"
    ));

    let id_shadowing_name = ResolvedRegistry::from_blocks([
        RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: "first".to_owned(),
                name: "alias".to_owned(),
            },
            prompt: "first".to_owned(),
        }),
        RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: "alias".to_owned(),
                name: "Second".to_owned(),
            },
            prompt: "second".to_owned(),
        }),
    ])
    .expect_err("ids must not shadow existing names");

    assert!(matches!(
        id_shadowing_name,
        RegistryError::AmbiguousReference {
            kind: "instruction",
            reference,
        } if reference == "alias"
    ));
}

#[test]
fn registry_rejects_normalized_duplicate_names() {
    let err = ResolvedRegistry::from_blocks([
        RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: "composed".to_owned(),
                name: "Café".to_owned(),
            },
            prompt: "Inspect".to_owned(),
        }),
        RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: "decomposed".to_owned(),
                name: "Cafe\u{301}".to_owned(),
            },
            prompt: "Inspect".to_owned(),
        }),
    ])
    .expect_err("canonically equivalent names are duplicates");

    assert!(matches!(
        &err,
        RegistryError::DuplicateName {
            kind: "instruction",
            name,
        } if name == "Cafe\u{301}"
    ));
    assert_eq!(err.to_string(), "duplicate instruction name: Cafe\u{301}");
}

#[test]
fn registry_rejects_programmatic_invalid_identity_ids() {
    let err = ResolvedRegistry::from_blocks([RegistryBlock::Instruction(InstructionBlock {
        identity: BlockIdentity {
            id: "../bad".to_owned(),
            name: "Bad".to_owned(),
        },
        prompt: "Inspect".to_owned(),
    })])
    .expect_err("programmatic block ids must follow registry id rules");

    assert!(matches!(
        err,
        RegistryError::InvalidBlockId(id) if id == "../bad"
    ));
}

#[test]
fn registry_rejects_programmatic_empty_identity_names() {
    let err = ResolvedRegistry::from_blocks([RegistryBlock::Instruction(InstructionBlock {
        identity: BlockIdentity {
            id: "empty-name".to_owned(),
            name: String::new(),
        },
        prompt: "Inspect".to_owned(),
    })])
    .expect_err("programmatic block names must be non-empty");

    assert_eq!(
        err.to_string(),
        "instruction empty-name name must be non-empty"
    );
}

#[test]
fn registry_resolves_normalized_name_references() {
    let registry = ResolvedRegistry::from_blocks([
        RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: "inspect".to_owned(),
                name: "Café".to_owned(),
            },
            prompt: "Inspect".to_owned(),
        }),
        RegistryBlock::Phase(PhaseBlock {
            identity: BlockIdentity {
                id: "phase".to_owned(),
                name: "Phase".to_owned(),
            },
            instruction_refs: vec!["Cafe\u{301}".to_owned()],
            tool_refs: Vec::new(),
            steps: Vec::new(),
        }),
    ])
    .expect("canonically equivalent name reference resolves");

    assert_eq!(
        registry
            .instruction_block("Cafe\u{301}")
            .expect("decomposed reference resolves")
            .identity
            .id,
        "inspect"
    );
}

#[test]
fn registry_rejects_duplicate_phase_step_ids() {
    let err = ResolvedRegistry::from_blocks([RegistryBlock::Phase(PhaseBlock {
        identity: BlockIdentity {
            id: "phase".to_owned(),
            name: "Phase".to_owned(),
        },
        instruction_refs: Vec::new(),
        tool_refs: Vec::new(),
        steps: vec![
            StepBlock {
                id: "attempt".to_owned(),
                name: "Attempt".to_owned(),
                connection_refs: Vec::new(),
            },
            StepBlock {
                id: "attempt".to_owned(),
                name: "Retry".to_owned(),
                connection_refs: Vec::new(),
            },
        ],
    })])
    .expect_err("duplicate phase-local step ids must fail");

    assert!(matches!(
        err,
        RegistryError::DuplicateId {
            kind: "step",
            id,
        } if id == "phase.attempt"
    ));
}

#[test]
fn registry_rejects_duplicate_phase_tool_refs() {
    let err = ResolvedRegistry::from_blocks([
        RegistryBlock::Tool(own_script_tool("echo", "script:echo")),
        RegistryBlock::Phase(PhaseBlock {
            identity: BlockIdentity {
                id: "phase".to_owned(),
                name: "Phase".to_owned(),
            },
            instruction_refs: Vec::new(),
            tool_refs: vec!["echo".to_owned(), "echo".to_owned()],
            steps: Vec::new(),
        }),
    ])
    .expect_err("duplicate phase tool references must fail");

    assert!(matches!(
        err,
        RegistryError::DuplicateId {
            kind: "phase tool reference",
            id,
        } if id == "phase.echo"
    ));
}

#[test]
fn parser_defaults_optional_loop_reference_lists() {
    let block = parse_registry_block(
        "minimal-loop.yaml",
        "loop:\n  id: minimal-loop\n  name: MinimalLoop\n  phase_refs: [phase-a]\n",
    )
    .expect("minimal loop parses");

    let RegistryBlock::Loop(loop_block) = block else {
        panic!("expected loop block");
    };
    assert!(loop_block.subloop_refs.is_empty());
    assert!(loop_block.connection_refs.is_empty());
}

#[test]
fn parser_accepts_yaml_comments_and_discards_formatting_comments() {
    let block = parse_registry_block(
            "commented-loop.yaml",
            "# leading comment\nloop: # block comment\n  id: commented-loop # field comment\n  name: \"Hash # Loop\"\n  phase_refs: [phase-a] # inline list comment\n",
        )
        .expect("comments are ignored outside quoted scalars");

    let RegistryBlock::Loop(loop_block) = block else {
        panic!("expected loop block");
    };
    assert_eq!(loop_block.identity.name, "Hash # Loop");
    assert_eq!(loop_block.phase_refs, vec!["phase-a"]);
}

#[test]
fn parser_accepts_standard_yaml_layout_and_quoted_tabs() {
    for (name, source, expected_prompt) in [
        (
            "quoted-tab.yaml",
            "instruction:\n  id: quoted-tab\n  name: QuotedTab\n  prompt: \"left\tright\"\n",
            "left\tright",
        ),
        (
            "three-space-indent.yaml",
            "instruction:\n   id: three-space-indent\n   name: ThreeSpaceIndent\n   prompt: Inspect\n",
            "Inspect",
        ),
    ] {
        let RegistryBlock::Instruction(instruction) =
            parse_registry_block(name, source).expect("valid YAML 1.2 parses")
        else {
            panic!("expected instruction block");
        };
        assert_eq!(instruction.prompt, expected_prompt);
    }
}

#[test]
fn parser_rejects_plain_yaml_non_string_scalars_for_string_fields() {
    for (name, source) in [
        (
            "boolean-prompt.yaml",
            "instruction:\n  id: boolean-prompt\n  name: BooleanPrompt\n  prompt: true\n",
        ),
        (
            "null-step-name.yaml",
            r#"phase:
  id: null-step-name
  name: NullStepName
  instruction_refs: []
  tool_refs: []
  steps:
    - id: inspect-step
      name: null
"#,
        ),
    ] {
        let err = parse_registry_block(name, source)
            .expect_err("plain YAML non-string scalar must be rejected");

        assert!(err.to_string().starts_with(name), "{name}: {err}");
    }

    let block = parse_registry_block(
            "quoted-boolean-prompt.yaml",
            "instruction:\n  id: quoted-boolean-prompt\n  name: QuotedBooleanPrompt\n  prompt: \"true\"\n",
        )
        .expect("quoted YAML scalar remains a string");

    let RegistryBlock::Instruction(instruction) = block else {
        panic!("expected instruction block");
    };
    assert_eq!(instruction.prompt, "true");
}

#[test]
fn parser_rejects_malformed_or_quoted_typed_scalars() {
    let parameter_tool =
        include_str!("../../../loop-agent/fixtures/hello-loop/registry/tools/read-file.yaml");
    let network_tool = include_str!(
        "../../../loop-agent/fixtures/sandbox-negative/registry/tools/network-tool.yaml"
    )
    .replace(
        "  network: deny\n",
        "  network:\n    default: deny\n    allow:\n      - kind: cidr\n        transport: tcp\n        cidr: 192.0.2.0/24\n        port: 443\n",
    );
    let integer_parameter = parameter_tool.replace(
        "      value_type: workspace-relative-path\n      required: true\n      value_pattern: \"^[A-Za-z0-9_./-]+$\"\n      max_length: 128",
        "      value_type: integer\n      required: true\n      min: nope",
    );

    for (name, source) in [
        (
            "quoted-required.yaml",
            parameter_tool.replace("required: true", "required: \"true\""),
        ),
        (
            "quoted-max-length.yaml",
            parameter_tool.replace("max_length: 128", "max_length: \"128\""),
        ),
        (
            "quoted-port.yaml",
            network_tool.replace("port: 443", "port: \"443\""),
        ),
        (
            "malformed-required.yaml",
            parameter_tool.replace("required: true", "required: maybe"),
        ),
        (
            "uppercase-required.yaml",
            parameter_tool.replace("required: true", "required: True"),
        ),
        ("malformed-min.yaml", integer_parameter),
        (
            "malformed-port.yaml",
            network_tool.replace("port: 443", "port: nope"),
        ),
    ] {
        let err = parse_registry_block(name, &source)
            .expect_err("schema-typed scalars must use their declared representation");

        assert!(err.to_string().starts_with(name), "{name}: {err}");
    }
}

#[test]
fn ids_follow_v0_token_rules() {
    assert!(is_valid_block_id("hello-loop"));
    assert!(is_valid_block_id("read_file_1"));
    assert!(!is_valid_block_id(""));
    assert!(!is_valid_block_id("HelloLoop"));
    assert!(!is_valid_block_id("../hello"));

    assert!(is_valid_command_id("agent-read"));
    assert!(!is_valid_command_id("1-agent-read"));
    assert!(!is_valid_command_id("agent.read"));
}

#[test]
fn registry_schema_is_checked_in_json() {
    let parsed = registry_schema();

    assert_eq!(
        parsed["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(
        parsed["$id"],
        "https://open-equilibrium.org/watershed/schemas/script/v0/registry-block.schema.json"
    );
}

#[test]
fn registry_schema_publishes_name_and_definition_limits() {
    let schema = registry_schema();

    assert_eq!(
        schema["$defs"]["block_name"]["maxLength"],
        MAX_BLOCK_NAME_CHARS
    );
    assert_eq!(
        schema["$defs"]["instruction"]["properties"]["prompt"]["maxLength"],
        MAX_REGISTRY_DEFINITION_BYTES
    );
    assert_eq!(
        schema["$defs"]["tool"]["properties"]["script_body"]["maxLength"],
        MAX_REGISTRY_DEFINITION_BYTES
    );
}

#[test]
fn registry_schema_concrete_blocks_own_full_shapes() {
    let parsed = registry_schema();

    for definition in ["connection", "instruction", "loop", "phase", "tool"] {
        let block = &parsed["$defs"][definition];
        assert_eq!(block["additionalProperties"], false, "{definition}");
        assert!(block["properties"]["id"].is_object(), "{definition}");
        assert!(block["properties"]["name"].is_object(), "{definition}");
    }

    for definition in ["connection", "instruction", "loop", "phase"] {
        assert!(
            parsed["$defs"][definition]["allOf"].is_null(),
            "{definition} must not compose identity through allOf"
        );
    }
}

#[test]
fn registry_schema_ties_tool_kind_to_command_shape() {
    let parsed = registry_schema();
    let tool_rules = parsed["$defs"]["tool"]["allOf"]
        .as_array()
        .expect("tool shape rules");

    assert!(tool_rules.iter().any(|rule| {
        rule["if"]["properties"]["tool_kind"]["const"] == "predefined-command"
            && rule["then"]["properties"]["command"]["$ref"] == "#/$defs/predefined_command"
            && rule["then"]["not"]["anyOf"].is_array()
    }));
    assert!(tool_rules.iter().any(|rule| {
        rule["if"]["properties"]["tool_kind"]["const"] == "own-script"
            && rule["then"]["properties"]["command"]["$ref"] == "#/$defs/own_script_command"
            && rule["then"]["required"].as_array().is_some_and(|items| {
                items.contains(&serde_json::json!("script_runtime"))
                    && items.contains(&serde_json::json!("script_body"))
            })
    }));
}

#[test]
fn registry_schema_bounds_string_and_enum_parameters() {
    let parsed = registry_schema();
    let parameter_rules = parsed["$defs"]["allowed_parameter"]["allOf"]
        .as_array()
        .expect("allowed parameter rules");

    assert!(parameter_rules.iter().any(|rule| {
        rule["if"]["properties"]["value_type"]["const"] == "string"
            && rule["then"]["required"].as_array().is_some_and(|items| {
                items.contains(&serde_json::json!("value_pattern"))
                    && items.contains(&serde_json::json!("max_length"))
            })
            && schema_rule_forbids_required_field(&rule["then"], "min")
            && schema_rule_forbids_required_field(&rule["then"], "max")
    }));
    assert!(parameter_rules.iter().any(|rule| {
        rule["if"]["properties"]["value_type"]["const"] == "enum"
            && rule["then"]["required"]
                .as_array()
                .is_some_and(|items| items.contains(&serde_json::json!("allowed_values")))
            && schema_rule_forbids_required_field(&rule["then"], "value_pattern")
            && schema_rule_forbids_required_field(&rule["then"], "max_length")
            && schema_rule_forbids_required_field(&rule["then"], "min")
            && schema_rule_forbids_required_field(&rule["then"], "max")
            && rule["else"]["not"]["required"]
                .as_array()
                .is_some_and(|items| items.contains(&serde_json::json!("allowed_values")))
    }));
    assert!(parameter_rules.iter().any(|rule| {
        rule["if"]["properties"]["value_type"]["const"] == "integer"
            && schema_rule_forbids_required_field(&rule["then"], "value_pattern")
            && schema_rule_forbids_required_field(&rule["then"], "max_length")
    }));
    assert!(parameter_rules.iter().any(|rule| {
        rule["if"]["properties"]["value_type"]["const"] == "none"
            && schema_rule_forbids_required_field(&rule["then"], "value_pattern")
            && schema_rule_forbids_required_field(&rule["then"], "max_length")
            && schema_rule_forbids_required_field(&rule["then"], "min")
            && schema_rule_forbids_required_field(&rule["then"], "max")
    }));
    assert!(parameter_rules.iter().any(|rule| {
        rule["if"]["properties"]["value_type"]["const"] == "workspace-relative-path"
            && schema_rule_forbids_required_field(&rule["then"], "min")
            && schema_rule_forbids_required_field(&rule["then"], "max")
    }));
}

#[test]
fn registry_schema_constrains_network_allow_to_cidr() {
    let parsed = registry_schema();
    let cidr_shape = &parsed["$defs"]["cidr_allow"]["properties"]["cidr"];
    let cidr_refs = cidr_shape["$ref"]
        .as_str()
        .expect("network allow cidr uses shared CIDR definition");

    assert_eq!(cidr_refs, "#/$defs/cidr");
    assert_eq!(parsed["$defs"]["ipv4_cidr"]["type"], "string");
    assert_eq!(parsed["$defs"]["ipv6_cidr"]["type"], "string");
    assert!(
        parsed["$defs"]["ipv4_cidr"]["pattern"]
            .as_str()
            .expect("IPv4 CIDR pattern")
            .contains("/(3[0-2]|[12]?[0-9])")
    );
    assert!(
        parsed["$defs"]["ipv6_cidr"]["pattern"]
            .as_str()
            .expect("IPv6 CIDR pattern")
            .contains("/(12[0-8]|1[01][0-9]|[1-9]?[0-9])")
    );
}

#[test]
fn cidr_contract_rejects_hostnames_and_malformed_values() {
    for cidr in [
        "0.0.0.0/0",
        "192.0.2.0/24",
        "192.0.2.42/32",
        "::/0",
        "2001:db8::/32",
        "::1/128",
    ] {
        assert!(is_valid_canonical_cidr(cidr), "{cidr}");
    }

    for cidr in [
        "example.com",
        "*.corp",
        "https://example.com",
        "192.0.2.42",
        "192.0.2.42/24",
        "192.0.2.0/33",
        "2001:db8::1/32",
        "2001:db8::/129",
        "2001:0db8::/32",
        "2001:DB8::/32",
        "10.0.0.0/-1",
        "10.0.0.0/foo",
        "10.0.0.0/01",
    ] {
        assert!(!is_valid_canonical_cidr(cidr), "{cidr}");
    }
}

#[test]
fn semantic_validation_requires_own_script_command_to_match_tool_id() {
    let mut tool = own_script_tool("write-summary", "script:other-tool");

    let err = validate_tool_semantics(&tool).expect_err("mismatched script id rejected");

    assert!(err.to_string().contains("script:<tool-id>"));
    assert_eq!(
        err,
        SemanticValidationError::OwnScriptCommandIdMismatch {
            command: "script:other-tool".to_owned(),
            tool_id: "write-summary".to_owned(),
        }
    );

    tool.command = ToolCommand::OwnScript("script:write-summary".to_owned());
    validate_tool_semantics(&tool).expect("matching script id accepted");
}

#[test]
fn semantic_validation_enforces_tool_kind_specific_script_fields() {
    let mut missing_runtime = own_script_tool("write-summary", "script:write-summary");
    missing_runtime.script_runtime = None;

    let err =
        validate_tool_semantics(&missing_runtime).expect_err("own-script runtime is required");

    assert!(matches!(
        err,
        SemanticValidationError::InvalidToolDefinition { message, .. }
            if message.contains("script_runtime")
    ));

    let mut predefined = ToolBlock {
        allowed_parameters: Vec::new(),
        command: ToolCommand::Predefined {
            command_id: "agent-echo".to_owned(),
            argv: Vec::new(),
        },
        identity: BlockIdentity {
            id: "echo".to_owned(),
            name: "Echo".to_owned(),
        },
        network: NetworkPolicy::Deny(NetworkDeny),
        protected_path_grants: Vec::new(),
        read_scope: Vec::new(),
        script_body: Some("echo unexpected".to_owned()),
        script_runtime: None,
        tool_kind: ToolKind::PredefinedCommand,
        write_scope: Vec::new(),
    };

    let err =
        validate_tool_semantics(&predefined).expect_err("predefined tools must omit script fields");

    assert!(matches!(
        err,
        SemanticValidationError::InvalidToolDefinition { message, .. }
            if message.contains("omit script_runtime")
    ));

    predefined.tool_kind = ToolKind::OwnScript;
    let err = validate_tool_semantics(&predefined).expect_err("command shape must match tool kind");
    assert!(err.to_string().contains("own-script"));
}

#[test]
fn semantic_validation_rejects_noncanonical_network_cidr() {
    let mut tool = own_script_tool("network-tool", "script:network-tool");
    tool.network = NetworkPolicy::Declared {
        allow: vec![NetworkAllowEntry {
            cidr: "192.0.2.42/24".to_owned(),
            kind: NetworkAllowKind::Cidr,
            port: 443,
            transport: NetworkTransport::Tcp,
        }],
        default: NetworkDefault::Deny,
    };

    let err = validate_tool_semantics(&tool).expect_err("host-bit CIDR rejected");

    assert!(err.to_string().contains("invalid canonical CIDR"));
    assert_eq!(
        err,
        SemanticValidationError::InvalidCanonicalCidr {
            cidr: "192.0.2.42/24".to_owned(),
            tool_id: "network-tool".to_owned(),
        }
    );

    if let NetworkPolicy::Declared { allow, .. } = &mut tool.network {
        allow[0].cidr = "192.0.2.0/24".to_owned();
    }
    validate_registry_block_semantics(&RegistryBlock::Tool(tool)).expect("canonical CIDR accepted");
}

fn own_script_tool(id: &str, command: &str) -> ToolBlock {
    ToolBlock {
        allowed_parameters: Vec::new(),
        command: ToolCommand::OwnScript(command.to_owned()),
        identity: BlockIdentity {
            id: id.to_owned(),
            name: "TestTool".to_owned(),
        },
        network: NetworkPolicy::Deny(NetworkDeny),
        protected_path_grants: Vec::new(),
        read_scope: Vec::new(),
        script_body: Some("echo ok".to_owned()),
        script_runtime: Some(ScriptRuntime::PosixSh),
        tool_kind: ToolKind::OwnScript,
        write_scope: Vec::new(),
    }
}

fn simple_phase_block(id: &str) -> RegistryBlock {
    RegistryBlock::Phase(PhaseBlock {
        identity: BlockIdentity {
            id: id.to_owned(),
            name: format!("Phase {id}"),
        },
        ..test_phase()
    })
}

fn test_phase() -> PhaseBlock {
    PhaseBlock {
        identity: BlockIdentity {
            id: "phase".to_owned(),
            name: "Phase".to_owned(),
        },
        instruction_refs: Vec::new(),
        tool_refs: Vec::new(),
        steps: vec![StepBlock {
            id: "step".to_owned(),
            name: "Step".to_owned(),
            connection_refs: Vec::new(),
        }],
    }
}

fn test_loop() -> LoopBlock {
    LoopBlock {
        identity: BlockIdentity {
            id: "root".to_owned(),
            name: "Root".to_owned(),
        },
        phase_refs: vec!["phase".to_owned()],
        subloop_refs: Vec::new(),
        connection_refs: Vec::new(),
    }
}

fn test_connection(from_ref: &str, to_ref: &str) -> ConnectionBlock {
    ConnectionBlock {
        identity: BlockIdentity {
            id: "link".to_owned(),
            name: "Link".to_owned(),
        },
        connection_kind: ConnectionKind::Data,
        from_ref: from_ref.to_owned(),
        to_ref: to_ref.to_owned(),
    }
}

fn assert_missing_reference(blocks: Vec<RegistryBlock>, expected: &str) {
    let error = ResolvedRegistry::from_blocks(blocks).expect_err("missing reference rejected");
    assert!(error.to_string().contains("references missing"));
    match error {
        RegistryError::MissingReference { reference_kind, .. } => {
            assert_eq!(reference_kind, expected)
        }
        error => panic!("expected missing {expected} reference, got {error}"),
    }
}

fn loop_chain_blocks(depth: usize) -> Vec<RegistryBlock> {
    let mut blocks = vec![simple_phase_block("chain-phase")];
    blocks.extend((0..depth).map(|index| RegistryBlock::Loop(loop_chain_block(index, depth))));
    blocks
}

fn loop_chain_block(index: usize, depth: usize) -> LoopBlock {
    LoopBlock {
        identity: BlockIdentity {
            id: format!("loop-{index:03}"),
            name: format!("Loop {index:03}"),
        },
        phase_refs: vec!["chain-phase".to_owned()],
        subloop_refs: (index + 1 < depth)
            .then(|| format!("loop-{:03}", index + 1))
            .into_iter()
            .collect(),
        connection_refs: Vec::new(),
    }
}

fn duplicated_subloop_tail_blocks(depth: usize) -> Vec<RegistryBlock> {
    let mut blocks = vec![simple_phase_block("dup-phase")];
    blocks.extend((0..depth).map(|index| {
        let child = (index + 1 < depth).then(|| format!("dup-loop-{:03}", index + 1));
        RegistryBlock::Loop(LoopBlock {
            identity: BlockIdentity {
                id: format!("dup-loop-{index:03}"),
                name: format!("DupLoop {index:03}"),
            },
            phase_refs: vec!["dup-phase".to_owned()],
            subloop_refs: child
                .into_iter()
                .flat_map(|child| [child.clone(), child])
                .collect(),
            connection_refs: Vec::new(),
        })
    }));
    blocks
}

fn registry_schema() -> serde_json::Value {
    serde_json::from_str(include_str!("../schemas/registry-block.schema.json"))
        .expect("schema is valid JSON")
}

fn schema_rule_forbids_required_field(rule: &Value, field: &str) -> bool {
    rule["not"]["anyOf"].as_array().is_some_and(|entries| {
        entries.iter().any(|entry| {
            entry["required"]
                .as_array()
                .is_some_and(|items| items.contains(&serde_json::json!(field)))
        })
    })
}

fn temp_registry_dir(label: &str) -> std::path::PathBuf {
    let target = std::env::temp_dir().join(format!(
        "watershed-core-script-{label}-{}",
        std::process::id()
    ));
    if target.exists() {
        std::fs::remove_dir_all(&target).expect("stale temp registry removed");
    }
    std::fs::create_dir_all(&target).expect("temp registry created");
    target
}

#[cfg(windows)]
fn create_windows_junction(link: &Path, target: &Path) {
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .expect("mklink command runs");
    assert!(
        output.status.success(),
        "junction creation failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
