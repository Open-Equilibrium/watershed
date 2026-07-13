use super::*;
use proptest::prelude::*;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn collect_registry_files(dir: &Path, out: &mut Vec<RegistryFile>) -> Result<(), RegistryError> {
    let limits = RegistryTraversalLimits {
        max_file_bytes: MAX_REGISTRY_FILE_BYTES,
        max_total_bytes: MAX_REGISTRY_TOTAL_BYTES,
        max_files: MAX_REGISTRY_FILES,
        max_depth: MAX_REGISTRY_TRAVERSAL_DEPTH,
    };
    let mut state = RegistryTraversalState::default();
    collect_registry_files_with_limits(dir, dir, out, limits, 0, &mut state)
}

proptest! {
    #[test]
    fn safe_relative_paths_accept_generated_literal_segments(
        segments in prop::collection::vec("[a-z0-9][a-z0-9_-]{0,7}", 1..8)
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
    fn safe_relative_paths_reject_escape_or_windows_alias_components(
        prefix in prop::collection::vec("[a-z0-9][a-z0-9_-]{0,7}", 0..4),
        suffix in prop::collection::vec("[a-z0-9][a-z0-9_-]{0,7}", 0..4),
        bad in prop_oneof![
            Just(".".to_owned()),
            Just("..".to_owned()),
            Just("CON".to_owned()),
            Just("NUL.txt".to_owned()),
            Just("COM1".to_owned()),
            Just("trail.".to_owned()),
            Just("trail ".to_owned()),
        ],
    ) {
        let mut segments = prefix;
        segments.push(bad);
        segments.extend(suffix);
        let path = segments.join("/");

        prop_assert_eq!(normalize_safe_relative_path(&path), None);
    }

    #[test]
    fn parser_rejects_generated_unknown_top_level_blocks(
        kind in "[a-z][a-z0-9_-]{0,12}"
    ) {
        prop_assume!(!matches!(
            kind.as_str(),
            "connection" | "instruction" | "loop" | "phase" | "tool"
        ));
        let source = format!("{kind}:\n");
        let error = parse_registry_block("property.yaml", &source).expect_err("unknown kind");

        prop_assert!(
            error
                .to_string()
                .contains("unsupported registry block kind")
        );
    }
}

#[test]
fn registry_loader_resolves_hello_loop_refs_and_canonical_output() {
    let registry = load_registry_root(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../loop-agent/fixtures/hello-loop/registry"),
    )
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

    let canonical =
        canonical_resolved_registry_json(&registry).expect("resolved registry serializes");
    assert!(canonical.ends_with('\n'));
    assert_eq!(
        canonical,
        canonical_resolved_registry_json(&registry).expect("canonical output repeats")
    );
    assert!(canonical.contains("\"hello-loop\""));
    assert!(canonical.contains("\"write-summary\""));
}

#[test]
fn registry_model_resolves_all_block_kinds_and_canonical_output() {
    assert_eq!(
        serde_json::to_string(&NetworkDeny).expect("deny serializes"),
        "\"deny\""
    );
    assert_eq!(
        serde_json::from_str::<NetworkDeny>("\"deny\"").expect("deny deserializes"),
        NetworkDeny
    );
    assert!(serde_json::from_str::<NetworkDeny>("\"allow\"").is_err());

    let parsed = parse_registry_block(
        "instruction.yaml",
        "instruction:\n  id: inspect-instruction\n  name: InspectInstruction\n  prompt: Inspect\n",
    )
    .expect("instruction parses");
    let RegistryBlock::Instruction(instruction) = parsed else {
        panic!("expected instruction block");
    };

    let mut tool = own_script_tool("write-summary", "script:write-summary");
    tool.identity.name = "WriteSummary".to_owned();
    tool.write_scope = vec!["out".to_owned()];
    let phase = PhaseBlock {
        identity: BlockIdentity {
            id: "inspect-phase".to_owned(),
            name: "InspectPhase".to_owned(),
        },
        instruction_refs: vec!["InspectInstruction".to_owned()],
        tool_refs: vec!["WriteSummary".to_owned()],
        steps: vec![StepBlock {
            id: "collect".to_owned(),
            name: "Collect".to_owned(),
            connection_refs: vec!["data-link".to_owned()],
        }],
    };
    let connection = ConnectionBlock {
        identity: BlockIdentity {
            id: "data-link".to_owned(),
            name: "DataLink".to_owned(),
        },
        connection_kind: ConnectionKind::Data,
        from_ref: "WriteSummary".to_owned(),
        to_ref: "inspect-phase.collect".to_owned(),
    };
    let loop_block = LoopBlock {
        identity: BlockIdentity {
            id: "hello-loop".to_owned(),
            name: "HelloLoop".to_owned(),
        },
        phase_refs: vec!["InspectPhase".to_owned()],
        subloop_refs: Vec::new(),
        connection_refs: vec!["DataLink".to_owned()],
    };

    let registry = ResolvedRegistry::from_blocks([
        RegistryBlock::Tool(tool),
        RegistryBlock::Instruction(instruction),
        RegistryBlock::Phase(phase),
        RegistryBlock::Connection(connection),
        RegistryBlock::Loop(loop_block),
    ])
    .expect("all block kinds resolve by id or normalized name");

    assert_eq!(
        registry
            .loop_block("HelloLoop")
            .expect("loop by name")
            .identity
            .id,
        "hello-loop"
    );
    assert_eq!(
        registry
            .phase_block("inspect-phase")
            .expect("phase by id")
            .identity
            .name,
        "InspectPhase"
    );
    assert_eq!(
        registry
            .tool_block("WriteSummary")
            .expect("tool by name")
            .identity
            .id,
        "write-summary"
    );
    assert_eq!(
        registry
            .instruction_block("InspectInstruction")
            .expect("instruction by name")
            .identity
            .id,
        "inspect-instruction"
    );
    assert_eq!(
        registry
            .connection_block("DataLink")
            .expect("connection by name")
            .identity
            .id,
        "data-link"
    );

    let without_newline = registry.canonical_json().expect("registry serializes");
    assert!(!without_newline.ends_with('\n'));
    let with_newline =
        canonical_resolved_registry_json(&registry).expect("canonical registry serializes");
    assert!(with_newline.ends_with('\n'));
    assert!(with_newline.contains("\"connection_refs\":[\"DataLink\"]"));
    assert!(with_newline.contains("\"subloop_refs\":[]"));
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

    let registry = load_registry_root(root).expect("nested yml registry loads");

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

    let err = ResolvedRegistry::load_with_limits(&root, 16, 1024)
        .expect_err("oversized registry file is rejected before parsing");

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
    let mut files = Vec::new();
    collect_registry_files(&root, &mut files).expect("registry file collected");
    assert_eq!(files.len(), 1);

    let err = read_registry_file_to_string(&files[0], 16)
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
fn registry_file_reader_rejects_file_replaced_after_collection() {
    let root = temp_registry_dir("registry-file-replaced-after-collection");
    let path = root.join("instruction.yaml");
    std::fs::write(
        &path,
        "instruction:\n  id: inspect\n  name: Inspect\n  prompt: Inspect\n",
    )
    .expect("registry file written");
    let mut files = Vec::new();
    collect_registry_files(&root, &mut files).expect("registry file collected");
    assert_eq!(files.len(), 1);

    std::fs::write(
        &path,
        "instruction:\n  id: replaced\n  name: Replaced\n  prompt: Replaced\n",
    )
    .expect("registry file replaced");

    let err = read_registry_file_to_string(&files[0], MAX_REGISTRY_FILE_BYTES)
        .expect_err("replaced registry file must be rejected");

    assert!(
        matches!(err, RegistryError::UnsafePath { ref message, .. } if message.contains("changed before open")),
        "unexpected error: {err:?}"
    );
}

#[test]
fn registry_file_reader_rejects_invalid_utf8_and_identity_edges() {
    let root = temp_registry_dir("registry-file-reader-edges");
    let invalid_utf8 = root.join("invalid.yaml");
    std::fs::write(&invalid_utf8, [0xff]).expect("invalid UTF-8 registry file written");
    let mut files = Vec::new();
    collect_registry_files(&root, &mut files).expect("registry file collected");
    assert_eq!(files.len(), 1);
    assert!(matches!(
        read_registry_file_to_string(&files[0], MAX_REGISTRY_FILE_BYTES),
        Err(RegistryError::Io { source, .. }) if source.kind() == std::io::ErrorKind::InvalidData
    ));
    let existing_metadata = std::fs::symlink_metadata(&invalid_utf8).expect("file metadata");
    let missing_file = RegistryFile {
        path: root.join("missing.yaml"),
        bytes: 0,
        identity: registry_file_identity(&invalid_utf8, &existing_metadata)
            .expect("existing file identity"),
    };
    assert!(matches!(
        read_registry_file_to_string(&missing_file, MAX_REGISTRY_FILE_BYTES),
        Err(RegistryError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound
    ));

    let dir_metadata = std::fs::symlink_metadata(&root).expect("directory metadata");
    let dir_file = RegistryFile {
        path: root.clone(),
        bytes: 0,
        identity: registry_file_identity(&root, &dir_metadata).expect("directory identity"),
    };
    assert!(matches!(
        ensure_opened_registry_file_matches(&dir_file, &dir_metadata),
        Err(RegistryError::UnsafePath { message, .. }) if message.contains("symlinks")
    ));
    let mut collected = Vec::new();
    let mut state = RegistryTraversalState::default();
    assert!(matches!(
        collect_registry_files_with_limits(
            &invalid_utf8,
            &invalid_utf8,
            &mut collected,
            RegistryTraversalLimits {
                max_file_bytes: MAX_REGISTRY_FILE_BYTES,
                max_total_bytes: MAX_REGISTRY_TOTAL_BYTES,
                max_files: MAX_REGISTRY_FILES,
                max_depth: MAX_REGISTRY_TRAVERSAL_DEPTH,
            },
            0,
            &mut state,
        ),
        Err(RegistryError::UnsafePath { message, .. }) if message.contains("must be a directory")
    ));

    let first = root.join("first.yaml");
    let second = root.join("second.yaml");
    std::fs::write(
        &first,
        "instruction:\n  id: first\n  name: First\n  prompt: First\n",
    )
    .expect("first registry file written");
    std::fs::write(
        &second,
        "instruction:\n  id: second\n  name: Second\n  prompt: Second\n",
    )
    .expect("second registry file written");
    let first_metadata = std::fs::symlink_metadata(&first).expect("first metadata");
    let second_metadata = std::fs::symlink_metadata(&second).expect("second metadata");
    let first_file = RegistryFile {
        path: first.clone(),
        bytes: first_metadata.len(),
        identity: registry_file_identity(&first, &first_metadata).expect("first identity"),
    };
    assert!(matches!(
        ensure_opened_registry_file_matches(&first_file, &second_metadata),
        Err(RegistryError::UnsafePath { message, .. }) if message.contains("changed before open")
    ));
}

#[test]
fn registry_loader_rejects_total_bytes_above_read_limit() {
    let root = temp_registry_dir("registry-total-read-limit");
    let first = "instruction:\n  id: inspect-a\n  name: InspectA\n  prompt: Inspect\n";
    let second = "instruction:\n  id: inspect-b\n  name: InspectB\n  prompt: Inspect\n";
    std::fs::write(root.join("a.yaml"), first).expect("first registry file written");
    std::fs::write(root.join("b.yaml"), second).expect("second registry file written");

    let err = ResolvedRegistry::load_with_limits(
        &root,
        1024,
        u64::try_from(first.len()).expect("test length fits u64"),
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
fn registry_loader_rejects_file_counts_above_traversal_limit() {
    let root = temp_registry_dir("registry-file-count-limit");
    std::fs::write(
        root.join("a.yaml"),
        "instruction:\n  id: inspect-a\n  name: InspectA\n  prompt: Inspect\n",
    )
    .expect("first registry file written");
    std::fs::write(
        root.join("b.yaml"),
        "instruction:\n  id: inspect-b\n  name: InspectB\n  prompt: Inspect\n",
    )
    .expect("second registry file written");

    let err = ResolvedRegistry::load_with_all_limits(&root, 1024, 1024, 1, 64)
        .expect_err("registry file count is rejected during traversal");

    assert!(matches!(
        err,
        RegistryError::TraversalLimitExceeded {
            limit: "file count",
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

    let err = ResolvedRegistry::load_with_all_limits(&root, 1024, 1024, 1024, 0)
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
fn registry_errors_report_sources_and_conversions() {
    let io_error = RegistryError::Io {
        path: PathBuf::from("registry"),
        source: std::io::Error::new(std::io::ErrorKind::NotFound, "missing"),
    };
    assert_eq!(io_error.to_string(), "registry: missing");
    assert!(std::error::Error::source(&io_error).is_some());

    let unsafe_path = RegistryError::UnsafePath {
        path: PathBuf::from("registry/link"),
        message: "symlink".to_owned(),
    };
    assert_eq!(unsafe_path.to_string(), "registry/link: symlink");
    assert!(std::error::Error::source(&unsafe_path).is_none());

    let cases = [
        (
            RegistryError::InvalidBlockId("Bad".to_owned()),
            "invalid block id: Bad",
        ),
        (
            RegistryError::InvalidBlockName {
                kind: "instruction",
                id: "empty-name".to_owned(),
            },
            "instruction empty-name name must be non-empty",
        ),
        (
            RegistryError::InvalidCommandId("bad command".to_owned()),
            "invalid command id: bad command",
        ),
        (
            RegistryError::Parse {
                source_name: "bad.yaml".to_owned(),
                message: "bad shape".to_owned(),
            },
            "bad.yaml: bad shape",
        ),
        (
            RegistryError::DuplicateId {
                kind: "tool",
                id: "echo".to_owned(),
            },
            "duplicate tool id: echo",
        ),
        (
            RegistryError::AmbiguousReference {
                kind: "endpoint",
                reference: "build".to_owned(),
            },
            "ambiguous endpoint reference build matches both an id and a name",
        ),
        (
            RegistryError::MissingReference {
                from_kind: "phase",
                from_id: "inspect".to_owned(),
                reference_kind: "tool",
                reference: "missing".to_owned(),
            },
            "phase inspect references missing tool missing",
        ),
        (
            RegistryError::LoopCycle {
                loop_id: "root".to_owned(),
            },
            "loop cycle includes root",
        ),
        (
            RegistryError::LoopDepthExceeded {
                loop_id: "root".to_owned(),
                depth: 65,
                max: 64,
            },
            "loop nesting depth 65 for root exceeds max 64",
        ),
        (
            RegistryError::ReadLimitExceeded {
                path: PathBuf::from("registry/tool.yaml"),
                bytes: 17,
                max: 16,
            },
            "registry/tool.yaml: registry read size 17 bytes exceeds max 16",
        ),
        (
            RegistryError::TraversalLimitExceeded {
                path: PathBuf::from("registry/deep"),
                limit: "depth",
                observed: 65,
                max: 64,
            },
            "registry/deep: registry traversal depth 65 exceeds max 64",
        ),
    ];
    for (err, expected) in cases {
        assert_eq!(err.to_string(), expected);
        assert!(std::error::Error::source(&err).is_none());
    }

    let semantic = SemanticValidationError::ToolCommandKindMismatch {
        tool_id: "bad-tool".to_owned(),
        tool_kind: ToolKind::OwnScript,
    };
    assert_eq!(
        semantic.to_string(),
        "tool command shape does not match OwnScript: bad-tool"
    );
    let schema = SemanticValidationError::ToolSchemaViolation {
        tool_id: "bad-tool".to_owned(),
        message: "bad schema".to_owned(),
    };
    assert_eq!(
        schema.to_string(),
        "tool schema violation for bad-tool: bad schema"
    );
    let own_script = SemanticValidationError::OwnScriptCommandIdMismatch {
        command: "agent-echo".to_owned(),
        tool_id: "write-summary".to_owned(),
    };
    assert_eq!(
        own_script.to_string(),
        "own-script command must be script:<tool-id>: write-summary used agent-echo"
    );
    let invalid_cidr = SemanticValidationError::InvalidCanonicalCidr {
        cidr: "192.0.2.1/24".to_owned(),
        tool_id: "network-tool".to_owned(),
    };
    assert_eq!(
        invalid_cidr.to_string(),
        "invalid canonical CIDR for tool network-tool: 192.0.2.1/24"
    );
    let semantic_registry = RegistryError::from(schema.clone());
    assert!(std::error::Error::source(&semantic_registry).is_some());
    assert_eq!(semantic_registry.to_string(), schema.to_string());

    let canonical_registry = RegistryError::CanonicalJson(
        canonical_json(&serde_json::json!({
            "é": 1,
            "e\u{301}": 2,
        }))
        .expect_err("normalized duplicate key produces canonical error"),
    );
    assert!(canonical_registry
        .to_string()
        .contains("failed to serialize canonical registry JSON"));
    assert!(std::error::Error::source(&canonical_registry).is_some());

    let serialize_error = RegistryError::Serialize(
        serde_json::from_str::<Value>("{").expect_err("invalid json produces serde error"),
    );
    assert!(serialize_error
        .to_string()
        .contains("failed to serialize resolved registry"));
    assert!(std::error::Error::source(&serialize_error).is_some());
}

#[test]
fn registry_reference_validation_reports_each_missing_reference_shape() {
    let mut tool = own_script_tool("write-summary", "script:write-summary");
    tool.script_body = None;
    let err = validate_tool_semantics(&tool).expect_err("script body is required");
    assert!(matches!(
        err,
        SemanticValidationError::ToolSchemaViolation { message, .. }
            if message.contains("script_body")
    ));

    let mut tool = own_script_tool("write-summary", "script:write-summary");
    tool.script_body = Some("   \n".to_owned());
    let err = validate_tool_semantics(&tool).expect_err("blank script body is rejected");
    assert!(matches!(
        err,
        SemanticValidationError::ToolSchemaViolation { message, .. }
            if message.contains("non-empty")
    ));

    let mut tool = own_script_tool("write-summary", "script:write-summary");
    tool.tool_kind = ToolKind::PredefinedCommand;
    let err = validate_tool_semantics(&tool).expect_err("tool kind must match command shape");
    assert!(matches!(
        err,
        SemanticValidationError::ToolCommandKindMismatch { .. }
    ));

    let missing_instruction = ResolvedRegistry::from_blocks([RegistryBlock::Phase(PhaseBlock {
        identity: BlockIdentity {
            id: "phase".to_owned(),
            name: "Phase".to_owned(),
        },
        instruction_refs: vec!["missing-instruction".to_owned()],
        tool_refs: Vec::new(),
        steps: vec![StepBlock {
            id: "step".to_owned(),
            name: "Step".to_owned(),
            connection_refs: Vec::new(),
        }],
    })])
    .expect_err("missing instruction rejected");
    assert!(matches!(
        missing_instruction,
        RegistryError::MissingReference {
            reference_kind: "instruction",
            ..
        }
    ));

    let missing_tool = ResolvedRegistry::from_blocks([RegistryBlock::Phase(PhaseBlock {
        identity: BlockIdentity {
            id: "phase".to_owned(),
            name: "Phase".to_owned(),
        },
        instruction_refs: Vec::new(),
        tool_refs: vec!["missing-tool".to_owned()],
        steps: vec![StepBlock {
            id: "step".to_owned(),
            name: "Step".to_owned(),
            connection_refs: Vec::new(),
        }],
    })])
    .expect_err("missing tool rejected");
    assert!(matches!(
        missing_tool,
        RegistryError::MissingReference {
            reference_kind: "tool",
            ..
        }
    ));

    let invalid_step = ResolvedRegistry::from_blocks([RegistryBlock::Phase(PhaseBlock {
        identity: BlockIdentity {
            id: "phase".to_owned(),
            name: "Phase".to_owned(),
        },
        instruction_refs: Vec::new(),
        tool_refs: Vec::new(),
        steps: vec![StepBlock {
            id: "BadStep".to_owned(),
            name: "Step".to_owned(),
            connection_refs: Vec::new(),
        }],
    })])
    .expect_err("invalid step id rejected");
    assert!(matches!(invalid_step, RegistryError::InvalidBlockId(value) if value == "BadStep"));

    let missing_phase = ResolvedRegistry::from_blocks([RegistryBlock::Loop(LoopBlock {
        identity: BlockIdentity {
            id: "root".to_owned(),
            name: "Root".to_owned(),
        },
        phase_refs: vec!["missing-phase".to_owned()],
        subloop_refs: Vec::new(),
        connection_refs: Vec::new(),
    })])
    .expect_err("missing phase rejected");
    assert!(matches!(
        missing_phase,
        RegistryError::MissingReference {
            reference_kind: "phase",
            ..
        }
    ));

    let missing_loop = ResolvedRegistry::from_blocks([
        simple_phase_block("phase"),
        RegistryBlock::Loop(LoopBlock {
            identity: BlockIdentity {
                id: "root".to_owned(),
                name: "Root".to_owned(),
            },
            phase_refs: vec!["phase".to_owned()],
            subloop_refs: vec!["missing-loop".to_owned()],
            connection_refs: Vec::new(),
        }),
    ])
    .expect_err("missing loop rejected");
    assert!(matches!(
        missing_loop,
        RegistryError::MissingReference {
            reference_kind: "loop",
            ..
        }
    ));

    let missing_connection = ResolvedRegistry::from_blocks([
        simple_phase_block("phase"),
        RegistryBlock::Loop(LoopBlock {
            identity: BlockIdentity {
                id: "root".to_owned(),
                name: "Root".to_owned(),
            },
            phase_refs: vec!["phase".to_owned()],
            subloop_refs: Vec::new(),
            connection_refs: vec!["missing-connection".to_owned()],
        }),
    ])
    .expect_err("missing connection rejected");
    assert!(matches!(
        missing_connection,
        RegistryError::MissingReference {
            reference_kind: "connection",
            ..
        }
    ));

    let empty_loop = ResolvedRegistry::from_blocks([RegistryBlock::Loop(LoopBlock {
        identity: BlockIdentity {
            id: "root".to_owned(),
            name: "Root".to_owned(),
        },
        phase_refs: Vec::new(),
        subloop_refs: Vec::new(),
        connection_refs: Vec::new(),
    })])
    .expect_err("empty loop phase_refs rejected");
    assert!(empty_loop.to_string().contains("loop.phase_refs"));

    let missing_endpoint =
        ResolvedRegistry::from_blocks([RegistryBlock::Connection(ConnectionBlock {
            identity: BlockIdentity {
                id: "link".to_owned(),
                name: "Link".to_owned(),
            },
            connection_kind: ConnectionKind::Data,
            from_ref: "missing-endpoint".to_owned(),
            to_ref: "also-missing".to_owned(),
        })])
        .expect_err("missing endpoint rejected");
    assert!(matches!(
        missing_endpoint,
        RegistryError::MissingReference {
            reference_kind: "endpoint",
            ..
        }
    ));

    let missing_step_endpoint = ResolvedRegistry::from_blocks([
        RegistryBlock::Phase(PhaseBlock {
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
        }),
        RegistryBlock::Connection(ConnectionBlock {
            identity: BlockIdentity {
                id: "link".to_owned(),
                name: "Link".to_owned(),
            },
            connection_kind: ConnectionKind::Data,
            from_ref: "phase.missing-step".to_owned(),
            to_ref: "phase.step".to_owned(),
        }),
    ])
    .expect_err("missing step endpoint rejected");
    assert!(matches!(
        missing_step_endpoint,
        RegistryError::MissingReference {
            reference_kind: "step",
            ..
        }
    ));

    let step_connection_not_declared_by_loop = ResolvedRegistry::from_blocks([
        RegistryBlock::Phase(PhaseBlock {
            identity: BlockIdentity {
                id: "phase".to_owned(),
                name: "Phase".to_owned(),
            },
            instruction_refs: Vec::new(),
            tool_refs: Vec::new(),
            steps: vec![StepBlock {
                id: "step".to_owned(),
                name: "Step".to_owned(),
                connection_refs: vec!["link".to_owned()],
            }],
        }),
        RegistryBlock::Connection(ConnectionBlock {
            identity: BlockIdentity {
                id: "link".to_owned(),
                name: "Link".to_owned(),
            },
            connection_kind: ConnectionKind::Data,
            from_ref: "phase.step".to_owned(),
            to_ref: "phase.step".to_owned(),
        }),
        RegistryBlock::Loop(LoopBlock {
            identity: BlockIdentity {
                id: "root".to_owned(),
                name: "Root".to_owned(),
            },
            phase_refs: vec!["phase".to_owned()],
            subloop_refs: Vec::new(),
            connection_refs: Vec::new(),
        }),
    ])
    .expect_err("step connection must be declared by loop");
    assert!(matches!(
        step_connection_not_declared_by_loop,
        RegistryError::MissingReference {
            reference_kind: "step connection",
            ..
        }
    ));
}

#[test]
fn parser_helper_edge_cases_are_rejected_with_specific_errors() {
    fn message<T: std::fmt::Debug>(result: Result<T, RegistryError>) -> String {
        result.expect_err("expected registry error").to_string()
    }

    let declared_network = parse_registry_block(
        "network-tool.yaml",
        r#"tool:
  id: network-tool
  name: NetworkTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: --count
      value_type: integer
      required: true
      min: -5
      max: 10
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network:
    default: deny
    allow:
      - kind: cidr
        transport: udp
        cidr: 192.0.2.0/24
        port: 53
"#,
    )
    .expect("declared deny-default network parses");
    let RegistryBlock::Tool(tool) = declared_network else {
        panic!("expected tool block");
    };
    assert_eq!(tool.allowed_parameters[0].min, Some(-5));
    assert!(matches!(
        tool.network,
        NetworkPolicy::Declared { allow, .. }
            if allow[0].transport == NetworkTransport::Udp && allow[0].port == 53
    ));

    for (name, source, expected) in [
            ("unsupported-kind.yaml", "unknown:\n  id: bad\n", "unsupported registry block kind"),
            (
                "tab.yaml",
                "instruction:\n\tid: bad\n",
                "tab indentation character",
            ),
            (
                "anchor.yaml",
                "instruction:\n  id: bad\n  name: Bad\n  prompt: Inspect\n  <<: *base\n",
                "unsupported YAML syntax",
            ),
            (
                "inline-anchor.yaml",
                "instruction:\n  id: bad\n  name: &display Bad\n  prompt: Inspect\n",
                "unsupported YAML syntax",
            ),
            (
                "inline-alias.yaml",
                "instruction:\n  id: bad\n  name: Bad\n  prompt: *base\n",
                "unsupported YAML syntax",
            ),
            (
                "inline-list-alias.yaml",
                "phase:\n  id: bad\n  name: Bad\n  instruction_refs: [*inspect]\n  tool_refs: []\n  steps:\n    - id: inspect\n      name: Inspect\n",
                "unsupported YAML syntax",
            ),
            ("bad-top.yaml", "instruction: Bad\n", "top-level line"),
            (
                "two-blocks.yaml",
                "instruction:\n  id: first\n  name: First\n  prompt: Inspect\nloop:\n  id: second\n  name: Second\n  phase_refs: []\n",
                "exactly one top-level block",
            ),
            (
                "missing-network.yaml",
                "tool:\n  id: missing-network\n  name: MissingNetwork\n  tool_kind: predefined-command\n  command:\n    command_id: agent-echo\n    argv: []\n  allowed_parameters: []\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n",
                "missing tool.network",
            ),
            (
                "bad-network-scalar.yaml",
                "tool:\n  id: bad-network\n  name: BadNetwork\n  tool_kind: predefined-command\n  command:\n    command_id: agent-echo\n    argv: []\n  allowed_parameters: []\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network: allow\n",
                "unsupported network policy",
            ),
            (
                "bad-network-default.yaml",
                "tool:\n  id: bad-default\n  name: BadDefault\n  tool_kind: predefined-command\n  command:\n    command_id: agent-echo\n    argv: []\n  allowed_parameters: []\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network:\n    default: allow\n    allow: []\n",
                "unsupported network default",
            ),
            (
                "bad-network-kind.yaml",
                "tool:\n  id: bad-kind\n  name: BadKind\n  tool_kind: predefined-command\n  command:\n    command_id: agent-echo\n    argv: []\n  allowed_parameters: []\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network:\n    default: deny\n    allow:\n      - kind: host\n        transport: tcp\n        cidr: 192.0.2.0/24\n        port: 443\n",
                "unsupported network allow kind",
            ),
            (
                "bad-network-transport.yaml",
                "tool:\n  id: bad-transport\n  name: BadTransport\n  tool_kind: predefined-command\n  command:\n    command_id: agent-echo\n    argv: []\n  allowed_parameters: []\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network:\n    default: deny\n    allow:\n      - kind: cidr\n        transport: icmp\n        cidr: 192.0.2.0/24\n        port: 443\n",
                "unsupported network transport",
            ),
            (
                "bad-tool-kind.yaml",
                "tool:\n  id: bad-tool-kind\n  name: BadToolKind\n  tool_kind: custom\n  command:\n    command_id: agent-echo\n    argv: []\n  allowed_parameters: []\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network: deny\n",
                "unsupported tool_kind",
            ),
            (
                "bad-command-id.yaml",
                "tool:\n  id: bad-command-id\n  name: BadCommandId\n  tool_kind: predefined-command\n  command:\n    command_id: BadCommand\n    argv: []\n  allowed_parameters: []\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network: deny\n",
                "invalid command id",
            ),
            (
                "bad-runtime.yaml",
                "tool:\n  id: bad-runtime\n  name: BadRuntime\n  tool_kind: own-script\n  command: script:bad-runtime\n  script_runtime: python\n  script_body: echo bad\n  allowed_parameters: []\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network: deny\n",
                "unsupported script_runtime",
            ),
            (
                "bad-connection-kind.yaml",
                "connection:\n  id: link\n  name: Link\n  connection_kind: control\n  from_ref: a\n  to_ref: b\n",
                "unsupported connection_kind",
            ),
            (
                "bad-id.yaml",
                "instruction:\n  id: Bad\n  name: Bad\n  prompt: Inspect\n",
                "invalid block id",
            ),
            (
                "bad-parameter-type.yaml",
                "tool:\n  id: bad-parameter\n  name: BadParameter\n  tool_kind: predefined-command\n  command:\n    command_id: agent-echo\n    argv: []\n  allowed_parameters:\n    - name: --value\n      value_type: bytes\n      required: true\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network: deny\n",
                "unsupported parameter value_type",
            ),
            (
                "integer-with-pattern.yaml",
                "tool:\n  id: integer-with-pattern\n  name: IntegerWithPattern\n  tool_kind: predefined-command\n  command:\n    command_id: agent-echo\n    argv: []\n  allowed_parameters:\n    - name: --count\n      value_type: integer\n      required: true\n      value_pattern: '[0-9]+'\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network: deny\n",
                "integer parameters must omit value_pattern and max_length",
            ),
            (
                "integer-with-max-length.yaml",
                "tool:\n  id: integer-with-max-length\n  name: IntegerWithMaxLength\n  tool_kind: predefined-command\n  command:\n    command_id: agent-echo\n    argv: []\n  allowed_parameters:\n    - name: --count\n      value_type: integer\n      required: true\n      max_length: 4\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network: deny\n",
                "integer parameters must omit value_pattern and max_length",
            ),
            (
                "none-with-min.yaml",
                "tool:\n  id: none-with-min\n  name: NoneWithMin\n  tool_kind: predefined-command\n  command:\n    command_id: agent-echo\n    argv: []\n  allowed_parameters:\n    - name: --flag\n      value_type: none\n      required: false\n      min: 1\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network: deny\n",
                "none parameters must omit value_pattern, max_length, min, and max",
            ),
            (
                "enum-without-values.yaml",
                "tool:\n  id: enum-without-values\n  name: EnumWithoutValues\n  tool_kind: predefined-command\n  command:\n    command_id: agent-echo\n    argv: []\n  allowed_parameters:\n    - name: --mode\n      value_type: enum\n      required: true\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network: deny\n",
                "enum parameters must declare",
            ),
            (
                "bad-step-id.yaml",
                "phase:\n  id: phase\n  name: Phase\n  instruction_refs: []\n  tool_refs: []\n  steps:\n    - id: BadStep\n      name: Step\n",
                "invalid block id",
            ),
        ] {
            assert!(
                message(parse_registry_block(name, source)).contains(expected),
                "{name}"
            );
        }

    let scalar_shape = ScalarListShape {
        section: "tool",
        parent: None,
        field: "read_scope",
        field_indent: 2,
        item_indent: 4,
    };
    assert!(
        block_string_list("empty-list.yaml", "tool:\n  read_scope: []\n", scalar_shape,)
            .expect("empty list parses")
            .is_empty()
    );
    assert_eq!(
        block_string_list(
            "inline-list.yaml",
            "tool:\n  read_scope: [workspace]\n",
            scalar_shape,
        )
        .expect("inline list parses"),
        vec!["workspace"]
    );
    assert!(message(block_string_list(
        "bad-list-indent.yaml",
        "tool:\n  read_scope:\n   - workspace\n",
        scalar_shape,
    ))
    .contains("unsupported list indentation"));
    assert!(message(block_string_list(
        "missing-list.yaml",
        "tool:\n  write_scope: []\n",
        scalar_shape,
    ))
    .contains("missing tool.read_scope"));

    let nested_scalar_shape = ScalarListShape {
        section: "tool",
        parent: Some("command"),
        field: "argv",
        field_indent: 4,
        item_indent: 6,
    };
    assert_eq!(
        block_string_list(
            "nested-inline-list.yaml",
            "tool:\n  command:\n    argv: [--message]\n",
            nested_scalar_shape,
        )
        .expect("nested inline list parses"),
        vec!["--message"]
    );

    assert_eq!(
        parse_literal_block_scalar(
            "strip-block.yaml",
            "tool:\n  script_body: |-\n    echo ok\n\n",
            "tool",
            "script_body",
            "|-",
        )
        .expect("strip chomping parses"),
        "echo ok"
    );
    assert!(message(parse_literal_block_scalar(
        "missing-block.yaml",
        "tool:\n  script_body: echo ok\n",
        "tool",
        "script_body",
        "|",
    ))
    .contains("missing tool.script_body block scalar"));
    assert!(message(parse_literal_block_scalar(
        "inconsistent-block.yaml",
        "tool:\n  script_body: |\n    echo ok\n   bad\n",
        "tool",
        "script_body",
        "|",
    ))
    .contains("inconsistent indentation"));
    assert!(message(parse_literal_block_scalar(
        "empty-block.yaml",
        "tool:\n  script_body: |\n\n",
        "tool",
        "script_body",
        "|",
    ))
    .contains("must be non-empty"));

    let object_shape = ListObjectShape {
        section: "phase",
        parent: None,
        field: "steps",
        field_indent: 2,
        item_indent: 4,
        property_indent: 6,
    };
    let object = list_objects(
            "step-list.yaml",
            "phase:\n  steps:\n    - id: step\n      name: Step\n      connection_refs:\n        - link\n",
            object_shape,
        )
        .expect("object list parses");
    assert_eq!(object[0]["connection_refs"], "[\"link\"]");
    for (name, source, expected) in [
        (
            "steps-not-list.yaml",
            "phase:\n  steps: bad\n",
            "phase.steps must be a list",
        ),
        (
            "steps-property-before-item.yaml",
            "phase:\n  steps:\n      name: Step\n",
            "property appears before list item",
        ),
        (
            "steps-malformed-property.yaml",
            "phase:\n  steps:\n    - id step\n",
            "must use key: value",
        ),
        (
            "steps-empty-property.yaml",
            "phase:\n  steps:\n    - id:\n",
            "must use key: value",
        ),
        (
            "steps-duplicate-property.yaml",
            "phase:\n  steps:\n    - id: step\n      id: again\n",
            "duplicate list object property id",
        ),
        (
            "steps-bad-indent.yaml",
            "phase:\n  steps:\n     - id: step\n",
            "uses unsupported indentation",
        ),
        (
            "steps-missing.yaml",
            "phase:\n  name: Phase\n",
            "missing phase.steps",
        ),
    ] {
        assert!(
            message(list_objects(name, source, object_shape)).contains(expected),
            "{name}"
        );
    }

    for (value, expected) in [
        ("not-a-list", "must be an inline YAML list"),
        ("[\"unterminated]", "unterminated"),
        ("[,]", "empty list item"),
        ("['unterminated]", "unterminated"),
        ("[true]", "list items must be strings"),
    ] {
        assert!(
            message(parse_inline_yaml_list("inline-list.yaml", "argv", value)).contains(expected),
            "{value}"
        );
    }
    assert_eq!(
        parse_inline_yaml_list("inline-list.yaml", "argv", r#"["a,b", 'can''t']"#)
            .expect("quoted list parses"),
        vec!["a,b", "can't"]
    );

    for (value, expected) in [
        ("\"unterminated", "unterminated"),
        ("\"\\q\"", "unsupported escape"),
        ("\"\\xZ0\"", "invalid \\x escape digit"),
        ("\"\\u12\"", "incomplete \\u escape"),
        ("\"\\U00110000\"", "invalid \\U Unicode scalar"),
        ("'unterminated", "unterminated"),
        ("'bad'apostrophe'", "malformed single-quoted scalar"),
    ] {
        assert!(
            message(unquote_yaml_scalar("quoted.yaml", "field", value)).contains(expected),
            "{value}"
        );
    }

    assert!(message(parse_bool("bool.yaml", "required", "maybe")).contains("true or false"));
    assert!(message(parse_u16("port.yaml", "port", "70000")).contains("16-bit integer"));
    assert!(message(parse_i64("int.yaml", "min", "abc")).contains("64-bit integer"));

    assert!(message(parse_registry_block(
        "unknown-block.yaml",
        "endpoint:\n  id: endpoint\n  name: Endpoint\n",
    ))
    .contains("unsupported registry block kind"));
    assert!(message(required_scalar(
        "empty-scalar.yaml",
        "instruction:\n  prompt: \"\"\n",
        "instruction",
        "prompt",
    ))
    .contains("instruction.prompt must be non-empty"));
    assert!(message(optional_scalar(
        "empty-optional-scalar.yaml",
        "tool:\n  script_runtime: \"\"\n",
        "tool",
        "script_runtime",
    ))
    .contains("tool.script_runtime must be non-empty"));
    assert!(message(section_scalar_value(
        "folded-scalar.yaml",
        "instruction:\n  prompt: >\n    Folded\n",
        "instruction",
        "prompt",
    ))
    .contains("unsupported folded block scalar"));
    assert!(message(section_scalar_value(
        "missing-scalar-value.yaml",
        "instruction:\n  prompt:\n",
        "instruction",
        "prompt",
    ))
    .contains("instruction.prompt must be a scalar"));
    assert!(message(required_nested_scalar(
        "missing-nested-scalar-value.yaml",
        "tool:\n  command:\n    command_id:\n",
        "tool",
        "command",
        "command_id",
    ))
    .contains("tool.command.command_id must be a scalar"));
    assert!(message(required_nested_scalar(
        "empty-nested-scalar.yaml",
        "tool:\n  command:\n    command_id: \"\"\n",
        "tool",
        "command",
        "command_id",
    ))
    .contains("tool.command.command_id must be non-empty"));

    let nested_object_shape = ListObjectShape {
        section: "tool",
        parent: Some("network"),
        field: "allow",
        field_indent: 4,
        item_indent: 6,
        property_indent: 8,
    };
    assert!(list_objects(
        "empty-nested-objects.yaml",
        "tool:\n  network:\n    allow: []\n",
        nested_object_shape
    )
    .expect("empty nested object list parses")
    .is_empty());
    let nested_objects = list_objects(
            "nested-objects-stop-at-sibling.yaml",
            "tool:\n  network:\n    allow:\n      - kind: cidr\n        transport: tcp\n        cidr: 127.0.0.0/8\n        port: 443\n  command:\n    argv: []\n",
            nested_object_shape,
        )
        .expect("nested object list stops at sibling parent");
    assert_eq!(nested_objects.len(), 1);

    let mut current = None;
    let mut pending = Some(PendingListProperty {
        field: "connection_refs".to_owned(),
        items: vec!["link".to_owned()],
    });
    assert!(message(flush_pending_list_property(
        "orphan-pending-list.yaml",
        &mut current,
        &mut pending,
    ))
    .contains("appears before list item"));

    let object = BTreeMap::new();
    assert!(message(required_object_scalar(
        "missing-object-property.yaml",
        &object,
        "id"
    ))
    .contains("missing list object property id"));
    let object = BTreeMap::from([("id".to_owned(), String::new())]);
    assert!(message(required_object_scalar(
        "empty-object-property.yaml",
        &object,
        "id"
    ))
    .contains("list object property id must be non-empty"));
    assert!(message(reject_unexpected_object_keys(
        "unexpected-object-property.yaml",
        "phase.steps",
        &BTreeMap::from([("unexpected".to_owned(), "value".to_owned())]),
        &["id", "name"],
    ))
    .contains("unsupported phase.steps property unexpected"));
}

#[test]
fn parser_helpers_cover_duplicate_fields_and_direct_edge_branches() {
    fn message<T: std::fmt::Debug>(result: Result<T, RegistryError>) -> String {
        result.expect_err("expected registry error").to_string()
    }

    assert!(message(raw_section_field_value(
        "duplicate-section-field.yaml",
        "tool:\n  id: first\n  id: second\n",
        "tool",
        "id",
    ))
    .contains("duplicate tool.id"));
    assert!(message(raw_nested_field_value(
        "duplicate-nested-field.yaml",
        "tool:\n  command:\n    argv: []\n    argv: [--again]\n",
        "tool",
        "command",
        "argv",
    ))
    .contains("duplicate tool.command.argv"));
    assert!(message(reject_unknown_section_fields(
        "section-field-without-colon.yaml",
        "tool:\n  id\n",
        "tool",
        &["id"],
    ))
    .contains("must use key: value"));
    assert!(message(reject_unknown_nested_fields(
        "nested-field-without-colon.yaml",
        "tool:\n  command:\n    argv\n",
        "tool",
        "command",
        &["argv"],
    ))
    .contains("must use key: value"));
    assert!(message(reject_unknown_nested_fields(
        "unsupported-nested-field.yaml",
        "other:\n  command:\n    unexpected: value\n\ntool:\n  other:\n    unexpected: value\n  command:\n    unexpected: value\n",
        "tool",
        "command",
        &["argv"],
    ))
    .contains("unsupported tool.command field unexpected"));

    assert_eq!(
        raw_nested_field_value(
            "nested-field-skips-unrelated.yaml",
            "other:\n  command:\n    argv: [ignored]\n\ntool:\n  other:\n    argv: [ignored]\n  command:\n    argv: [--ok]\n",
            "tool",
            "command",
            "argv",
        )
        .expect("nested field parses"),
        Some("[--ok]".to_owned())
    );
    assert_eq!(
        parse_literal_block_scalar(
            "literal-block-edges.yaml",
            "other:\n  script_body: |\n    ignored\n\ntool:\n  name: Tool\n  script_body: |-\n    printf hi\n\n  network: deny\n",
            "tool",
            "script_body",
            "|-",
        )
        .expect("literal block parses"),
        "printf hi"
    );

    let scalar_shape = ScalarListShape {
        section: "loop",
        parent: None,
        field: "phase_refs",
        field_indent: 2,
        item_indent: 4,
    };
    assert_eq!(
        block_string_list(
            "block-list-breaks-at-next-section.yaml",
            "loop:\n  phase_refs:\n    - inspect\nphase:\n  id: inspect\n",
            scalar_shape,
        )
        .expect("block list stops at next top-level section"),
        vec!["inspect"]
    );
    let nested_scalar_shape = ScalarListShape {
        section: "tool",
        parent: Some("command"),
        field: "argv",
        field_indent: 4,
        item_indent: 6,
    };
    assert_eq!(
        block_string_list(
            "nested-block-list-skips-unrelated.yaml",
            "other:\n  command:\n    argv:\n      - ignored\n\ntool:\n  other:\n    argv:\n      - ignored\n  command:\n    argv:\n      - --ok\nnext:\n  id: after\n",
            nested_scalar_shape,
        )
        .expect("nested block list parses"),
        vec!["--ok"]
    );

    let object_shape = ListObjectShape {
        section: "phase",
        parent: None,
        field: "steps",
        field_indent: 2,
        item_indent: 4,
        property_indent: 6,
    };
    let object = list_objects(
        "pending-list-property.yaml",
        "phase:\n  steps:\n    - id: step\n      name: Step\n      connection_refs:\n        - link\nnext:\n  id: after\n",
        object_shape,
    )
    .expect("pending list property flushes before top-level break");
    assert_eq!(object[0]["connection_refs"], "[\"link\"]");
    let inline_pending_object = list_objects(
        "inline-pending-list-property.yaml",
        "phase:\n  steps:\n    - connection_refs:\n        - link\n      id: step\n      name: Step\n",
        object_shape,
    )
    .expect("pending list property may start on the item line");
    assert_eq!(inline_pending_object[0]["connection_refs"], "[\"link\"]");
    let nested_object_shape = ListObjectShape {
        section: "tool",
        parent: Some("network"),
        field: "allow",
        field_indent: 4,
        item_indent: 6,
        property_indent: 8,
    };
    let nested_object = list_objects(
        "nested-object-list-skips-unrelated.yaml",
        "other:\n  network:\n    allow:\n      - kind: cidr\n\ntool:\n  command:\n    allow:\n      - kind: ignored\n  network:\n    allow:\n      - kind: cidr\n        transport: tcp\n        cidr: 192.0.2.0/24\n        port: 443\nnext:\n  id: after\n",
        nested_object_shape,
    )
    .expect("nested object list parses");
    assert_eq!(nested_object[0]["cidr"], "192.0.2.0/24");
    assert!(message(list_objects(
        "steps-empty-field.yaml",
        "phase:\n  steps:\n    - : value\n",
        object_shape,
    ))
    .contains("must use key: value"));

    let mut item = BTreeMap::new();
    assert_eq!(
        parse_object_property("list-property.yaml", "connection_refs:", &mut item)
            .expect("empty connection_refs starts pending list"),
        Some("connection_refs".to_owned())
    );
    assert!(
        message(parse_object_property("empty-value.yaml", "id:", &mut item,))
            .contains("must use key: value")
    );
    assert!(message(push_inline_list_item(
        "malformed-quoted-list-item.yaml",
        "argv",
        &mut Vec::new(),
        "\"unterminated",
    ))
    .contains("malformed quoted scalar"));
    assert!(message(unquote_yaml_scalar(
        "dangling-double-quote-escape.yaml",
        "field",
        r#""abc\""#,
    ))
    .contains("dangling escape"));

    let mut sortable = serde_json::json!({
        "outer": [{
            "allowed_parameters": [
                {"name": "--z"},
                {"description": "missing name"},
                {"name": "--a"}
            ]
        }],
        "allowed_parameters": [
            {"name": "--b"},
            {"name": "--a"}
        ]
    });
    sort_allowed_parameters(&mut sortable);
    assert_eq!(
        sortable["allowed_parameters"]
            .as_array()
            .expect("root parameters")
            .iter()
            .map(|value| value["name"].as_str().unwrap_or(""))
            .collect::<Vec<_>>(),
        vec!["--a", "--b"]
    );
    assert_eq!(
        sortable["outer"][0]["allowed_parameters"][2]["name"],
        serde_json::json!("--z")
    );

    assert!(!is_valid_allowed_parameter_name("--"));
    assert!(!is_valid_allowed_parameter_name("value"));
    assert!(is_valid_allowed_parameter_name("--value_1"));
    assert!(value_forbids_nested_yaml_content("deny"));
    assert!(!value_forbids_nested_yaml_content("|"));
    assert!(!value_forbids_nested_yaml_content(">"));
}

#[cfg(unix)]
#[test]
fn registry_loader_rejects_symlinked_registry_entries() {
    use std::os::unix::fs::symlink;

    let root = temp_registry_dir("symlink-root");
    let outside = temp_registry_dir("symlink-outside");
    symlink(&outside, root.join("linked")).expect("registry symlink created");

    let err = load_registry_root(&root).expect_err("registry symlink must be rejected");

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

    let err = load_registry_root(&root).expect_err("registry junction must be rejected");

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

    let err = load_registry_root(&linked_root).expect_err("linked registry root must be rejected");

    assert!(
        matches!(err, RegistryError::UnsafePath { ref path, ref message }
            if path == &linked_root && (message.contains("symlink") || message.contains("reparse"))),
        "unexpected error: {err:?}"
    );
}

#[test]
fn parser_reads_own_script_body_without_relicensing_or_runtime_escape() {
    let block = parse_registry_block(
        "write-summary.yaml",
        include_str!("../../../loop-agent/fixtures/hello-loop/registry/tools/write-summary.yaml"),
    )
    .expect("write-summary parses");

    let RegistryBlock::Tool(tool) = block else {
        panic!("expected tool block");
    };

    assert_eq!(tool.script_runtime, Some(ScriptRuntime::PosixSh));
    assert_eq!(
        tool.script_body.as_deref(),
        Some("printf '%s\\n' \"$SUMMARY\" > out/summary.txt\n")
    );
}

#[test]
fn parser_rejects_unterminated_quoted_yaml_scalars() {
    for (name, source) in [
        (
            "unterminated-double-quoted-scalar.yaml",
            r#"tool:
  id: bad-quoted-tool
  name: "BadQuotedTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        ),
        (
            "unterminated-single-quoted-scalar.yaml",
            r#"tool:
  id: bad-quoted-tool
  name: 'BadQuotedTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        ),
    ] {
        let err = parse_registry_block(name, source)
            .expect_err("unterminated quoted scalar must be rejected");

        assert!(err.to_string().contains("unterminated"), "{name}: {err}");
    }
}

#[test]
fn parser_rejects_nested_content_under_scalar_yaml_fields() {
    for (name, source) in [
        (
            "scalar-network-with-nested-allow.yaml",
            r#"tool:
  id: scalar-network
  name: ScalarNetwork
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
    allow: []
"#,
        ),
        (
            "scalar-command-with-nested-command-id.yaml",
            r#"tool:
  id: scalar-command
  name: ScalarCommand
  tool_kind: own-script
  command: script:scalar-command
    command_id: agent-echo
  script_runtime: posix-sh
  script_body: |
    printf '%s\n' ok
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        ),
        (
            "nested-scalar-network-default-with-child.yaml",
            r#"tool:
  id: nested-network
  name: NestedNetwork
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network:
    default: deny
      ignored: true
    allow: []
"#,
        ),
    ] {
        let err = parse_registry_block(name, source)
            .expect_err("nested content under scalar fields must be rejected");

        assert!(err.to_string().contains("nested"), "{name}: {err}");
    }
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
fn parser_decodes_yaml_double_quoted_escape_set() {
    let block = parse_registry_block(
            "quoted-argv.yaml",
            r#"tool:
  id: quoted-argv
  name: QuotedArgv
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: ["\0", "\a", "\b", "\t", "\n", "\v", "\f", "\r", "\e", "\"", "\/", "\\", "\N", "\_", "\L", "\P", "\x41", "\u03A9", "\U0001F642"]
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        )
        .expect("YAML 1.2 double-quoted escape set parses");

    let RegistryBlock::Tool(tool) = block else {
        panic!("expected tool block");
    };
    assert_eq!(
        tool.command,
        ToolCommand::Predefined {
            command_id: "agent-echo".to_owned(),
            argv: vec![
                "\0".to_owned(),
                "\u{7}".to_owned(),
                "\u{8}".to_owned(),
                "\t".to_owned(),
                "\n".to_owned(),
                "\u{b}".to_owned(),
                "\u{c}".to_owned(),
                "\r".to_owned(),
                "\u{1b}".to_owned(),
                "\"".to_owned(),
                "/".to_owned(),
                "\\".to_owned(),
                "\u{85}".to_owned(),
                "\u{a0}".to_owned(),
                "\u{2028}".to_owned(),
                "\u{2029}".to_owned(),
                "A".to_owned(),
                "\u{03a9}".to_owned(),
                "\u{1f642}".to_owned(),
            ],
        }
    );
}

#[test]
fn parser_rejects_invalid_double_quoted_yaml_escapes() {
    let err = parse_registry_block(
        "bad-escape-script-body.yaml",
        r#"tool:
  id: bad-escape-script
  name: BadEscapeScript
  tool_kind: own-script
  command: script:bad-escape-script
  script_runtime: posix-sh
  script_body: "echo \q"
  allowed_parameters: []
  read_scope: ["workspace"]
  write_scope: ["workspace/out"]
  protected_path_grants: []
  network: deny
"#,
    )
    .expect_err("invalid quoted escape must be rejected");

    assert!(err.to_string().contains("unsupported escape"));
}

#[test]
fn parser_rejects_malformed_double_quoted_yaml_scalars() {
    let err = parse_registry_block(
        "malformed-double-quoted-scalar.yaml",
        r#"tool:
  id: malformed-double-quoted-tool
  name: "Bad"Tool"
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
    )
    .expect_err("double-quoted scalar with bare quote rejected");

    assert!(err.to_string().contains("double-quoted"));
}

#[test]
fn parser_reads_literal_block_own_script_body() {
    let block = parse_registry_block(
        "literal-script-body.yaml",
        r#"tool:
  id: literal-script
  name: LiteralScript
  tool_kind: own-script
  command: script:literal-script
  script_runtime: posix-sh
  script_body: |
    printf '%s\n' "$SUMMARY" > out/summary.txt
    echo done
  allowed_parameters: []
  read_scope: ["workspace"]
  write_scope: ["workspace/out"]
  protected_path_grants: []
  network: deny
"#,
    )
    .expect("literal block script_body parses");

    let RegistryBlock::Tool(tool) = block else {
        panic!("expected tool block");
    };
    assert_eq!(
        tool.script_body.as_deref(),
        Some("printf '%s\\n' \"$SUMMARY\" > out/summary.txt\necho done\n")
    );
}

#[test]
fn parser_preserves_literal_block_script_body_comments() {
    let block = parse_registry_block(
        "literal-script-body-comments.yaml",
        r#"tool:
  id: commented-script
  name: CommentedScript
  tool_kind: own-script
  command: script:commented-script
  script_runtime: posix-sh
  script_body: |
    #!/bin/sh
    echo ok # keep
    ---
  allowed_parameters: []
  read_scope: ["workspace"]
  write_scope: ["workspace/out"]
  protected_path_grants: []
  network: deny
"#,
    )
    .expect("literal block script comments are script source");

    let RegistryBlock::Tool(tool) = block else {
        panic!("expected tool block");
    };
    assert_eq!(
        tool.script_body.as_deref(),
        Some("#!/bin/sh\necho ok # keep\n---\n")
    );
}

#[test]
fn parser_does_not_extract_fields_from_literal_block_body() {
    let err = parse_registry_block(
        "literal-script-body-smuggle.yaml",
        r#"tool:
  id: smuggle-script
  name: SmuggleScript
  tool_kind: own-script
  command: script:smuggle-script
  script_runtime: posix-sh
  script_body: |
   read_scope: ["workspace"]
   write_scope: ["workspace/out"]
   protected_path_grants: ["workspace/.env"]
   network: deny
  allowed_parameters: []
"#,
    )
    .expect_err("literal block content must not satisfy sibling fields");

    assert!(err.to_string().contains("missing tool.read_scope"));
}

#[test]
fn parser_rejects_empty_or_misrepresented_own_script_body() {
    let err = parse_registry_block(
        "empty-script-body.yaml",
        r#"tool:
  id: empty-script
  name: EmptyScript
  tool_kind: own-script
  command: script:empty-script
  script_runtime: posix-sh
  script_body: ""
  allowed_parameters: []
  read_scope: ["workspace"]
  write_scope: ["workspace/out"]
  protected_path_grants: []
  network: deny
"#,
    )
    .expect_err("empty script body rejected");
    assert!(err.to_string().contains("script_body"));

    let err = parse_registry_block(
        "folded-script-body.yaml",
        r#"tool:
  id: folded-script
  name: FoldedScript
  tool_kind: own-script
  command: script:folded-script
  script_runtime: posix-sh
  script_body: >
    echo folded
  allowed_parameters: []
  read_scope: ["workspace"]
  write_scope: ["workspace/out"]
  protected_path_grants: []
  network: deny
"#,
    )
    .expect_err("folded script body rejected");
    assert!(err.to_string().contains("folded block scalar"));
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

    assert!(matches!(err, RegistryError::LoopCycle { .. }));
}

#[test]
fn registry_reference_validation_rejects_deep_loop_chains() {
    ResolvedRegistry::from_blocks(loop_chain_blocks(MAX_LOOP_NESTING_DEPTH))
        .expect("max loop nesting depth is accepted");

    let err = ResolvedRegistry::from_blocks(loop_chain_blocks(MAX_LOOP_NESTING_DEPTH + 1))
        .expect_err("loop nesting above the max is rejected");

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
fn registry_reference_validation_memoizes_duplicate_subloop_tails() {
    let started = Instant::now();

    ResolvedRegistry::from_blocks(duplicated_subloop_tail_blocks(25))
        .expect("duplicated acyclic subloop tail validates");

    assert!(
        started.elapsed() < Duration::from_millis(250),
        "duplicated subloop tail validation must be linear in resolved loops"
    );
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
        err,
        RegistryError::DuplicateId {
            kind: "instruction",
            ..
        }
    ));
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
fn parser_rejects_schema_invalid_empty_loop_phase_refs() {
    let err = parse_registry_block(
        "empty-loop.yaml",
        "loop:\n  id: empty-loop\n  name: EmptyLoop\n  phase_refs: []\n",
    )
    .expect_err("empty phase_refs rejected");

    assert!(err.to_string().contains("loop.phase_refs"));
}

#[test]
fn parser_rejects_schema_invalid_empty_phase_steps() {
    let err = parse_registry_block(
            "empty-phase.yaml",
            "phase:\n  id: empty-phase\n  name: EmptyPhase\n  instruction_refs: []\n  tool_refs: []\n  steps: []\n",
        )
        .expect_err("empty steps rejected");

    assert!(err.to_string().contains("phase.steps"));
}

#[test]
fn parser_rejects_duplicate_section_scalar_fields() {
    let err = parse_registry_block(
        "duplicate-write-scope.yaml",
        r#"tool:
  id: duplicate-tool
  name: DuplicateTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters: []
  read_scope: []
  write_scope: ["workspace"]
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
    )
    .expect_err("duplicate section field rejected");

    assert!(err.to_string().contains("duplicate tool.write_scope"));
}

#[test]
fn parser_rejects_duplicate_nested_scalar_fields() {
    let err = parse_registry_block(
        "duplicate-command-id.yaml",
        r#"tool:
  id: duplicate-command
  name: DuplicateCommand
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    command_id: agent-read
    argv: []
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
    )
    .expect_err("duplicate nested field rejected");

    assert!(err
        .to_string()
        .contains("duplicate tool.command.command_id"));
}

#[test]
fn parser_rejects_duplicate_section_list_fields() {
    let err = parse_registry_block(
        "duplicate-steps.yaml",
        r#"phase:
  id: duplicate-steps
  name: DuplicateSteps
  instruction_refs: []
  tool_refs: []
  steps:
    - id: first-step
      name: FirstStep
      connection_refs: []
  steps:
    - id: second-step
      name: SecondStep
      connection_refs: []
"#,
    )
    .expect_err("duplicate section list field rejected");

    assert!(err.to_string().contains("duplicate phase.steps"));
}

#[test]
fn parser_rejects_duplicate_nested_list_fields() {
    let err = parse_registry_block(
        "duplicate-network-allow.yaml",
        r#"tool:
  id: duplicate-network
  name: DuplicateNetwork
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network:
    default: deny
    allow:
      - kind: cidr
        transport: tcp
        cidr: 127.0.0.0/8
        port: 443
    allow:
      - kind: cidr
        transport: tcp
        cidr: 10.0.0.0/8
        port: 443
"#,
    )
    .expect_err("duplicate nested list field rejected");

    assert!(err.to_string().contains("duplicate tool.network.allow"));
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
fn parser_accepts_block_style_yaml_scalar_lists() {
    let block = parse_registry_block(
        "block-list-tool.yaml",
        r#"tool:
  id: block-list-tool
  name: BlockListTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv:
      - --message
      - "hello, world"
  allowed_parameters:
    - name: --mode
      value_type: enum
      required: true
      allowed_values:
        - fast
        - "safe,quoted"
  read_scope:
    - workspace
  write_scope:
    - out
  protected_path_grants:
    - out/allowed.txt
  network: deny
"#,
    )
    .expect("block-style scalar lists parse");

    let RegistryBlock::Tool(tool) = block else {
        panic!("expected tool block");
    };
    assert_eq!(
        tool.command,
        ToolCommand::Predefined {
            command_id: "agent-echo".to_owned(),
            argv: vec!["--message".to_owned(), "hello, world".to_owned()],
        }
    );
    assert_eq!(tool.read_scope, vec!["workspace"]);
    assert_eq!(tool.write_scope, vec!["out"]);
    assert_eq!(tool.protected_path_grants, vec!["out/allowed.txt"]);
    assert_eq!(
        tool.allowed_parameters[0].allowed_values,
        vec!["fast".to_owned(), "safe,quoted".to_owned()]
    );
}

#[test]
fn parser_accepts_block_style_loop_and_step_reference_lists() {
    let block = parse_registry_block(
        "block-list-loop.yaml",
        r#"loop:
  id: block-list-loop
  name: BlockListLoop
  phase_refs:
    - inspect-phase
  subloop_refs:
    - child-loop
  connection_refs:
    - data-link
"#,
    )
    .expect("loop block-style scalar lists parse");

    let RegistryBlock::Loop(loop_block) = block else {
        panic!("expected loop block");
    };
    assert_eq!(loop_block.phase_refs, vec!["inspect-phase"]);
    assert_eq!(loop_block.subloop_refs, vec!["child-loop"]);
    assert_eq!(loop_block.connection_refs, vec!["data-link"]);

    let block = parse_registry_block(
        "block-list-phase.yaml",
        r#"phase:
  id: inspect-phase
  name: InspectPhase
  instruction_refs:
    - inspect-instruction
  tool_refs:
    - block-list-tool
  steps:
    - id: inspect-step
      name: InspectStep
      connection_refs:
        - data-link
"#,
    )
    .expect("phase block-style scalar lists parse");

    let RegistryBlock::Phase(phase) = block else {
        panic!("expected phase block");
    };
    assert_eq!(phase.instruction_refs, vec!["inspect-instruction"]);
    assert_eq!(phase.tool_refs, vec!["block-list-tool"]);
    assert_eq!(phase.steps[0].connection_refs, vec!["data-link"]);
}

#[test]
fn parser_rejects_unknown_schema_fields() {
    let err = parse_registry_block(
            "unknown-field.yaml",
            "instruction:\n  id: bad-instruction\n  name: BadInstruction\n  prompt: Inspect\n  prompt_extra: ignored\n",
        )
        .expect_err("unknown field rejected");

    assert!(err.to_string().contains("unsupported instruction field"));
}

#[test]
fn parser_rejects_empty_required_schema_strings() {
    let err = parse_registry_block(
        "empty-prompt.yaml",
        "instruction:\n  id: empty-prompt\n  name: EmptyPrompt\n  prompt: \"\"\n",
    )
    .expect_err("empty prompt rejected");

    assert!(err.to_string().contains("instruction.prompt"));
}

#[test]
fn parser_rejects_plain_yaml_non_string_scalars_for_string_fields() {
    for (name, source, expected) in [
        (
            "boolean-prompt.yaml",
            "instruction:\n  id: boolean-prompt\n  name: BooleanPrompt\n  prompt: true\n",
            "instruction.prompt must be a string",
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
            "name must be a string",
        ),
    ] {
        let err = parse_registry_block(name, source)
            .expect_err("plain YAML non-string scalar must be rejected");

        assert!(err.to_string().contains(expected), "{name}: {err}");
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
fn parser_rejects_quoted_yaml_scalars_for_typed_fields() {
    for (name, source, expected) in [
        (
            "quoted-required.yaml",
            r#"tool:
  id: quoted-required
  name: QuotedRequired
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: --value
      value_type: none
      required: "true"
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
            "required",
        ),
        (
            "quoted-max-length.yaml",
            r#"tool:
  id: quoted-max-length
  name: QuotedMaxLength
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: --value
      value_type: string
      required: true
      value_pattern: "^[^/]+$"
      max_length: "64"
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
            "max_length",
        ),
        (
            "quoted-port.yaml",
            r#"tool:
  id: quoted-port
  name: QuotedPort
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network:
    default: deny
    allow:
      - kind: cidr
        transport: tcp
        cidr: 192.0.2.0/24
        port: "443"
"#,
            "port",
        ),
    ] {
        let err =
            parse_registry_block(name, source).expect_err("quoted typed scalar must be rejected");

        assert!(err.to_string().contains(expected), "{name}: {err}");
    }
}

#[test]
fn parser_rejects_schema_invalid_allowed_parameter_names() {
    let err = parse_registry_block(
        "invalid-parameter-name.yaml",
        r#"tool:
  id: invalid-parameter-name
  name: InvalidParameterName
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: file
      value_type: string
      required: true
      value_pattern: "^[^/]+$"
      max_length: 64
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
    )
    .expect_err("schema-invalid parameter name rejected");

    assert!(err.to_string().contains("allowed_parameters.name"));
}

#[test]
fn parser_enforces_allowed_parameter_schema_conditionals() {
    let err = parse_registry_block(
        "string-parameter-missing-bounds.yaml",
        r#"tool:
  id: bounded-tool
  name: BoundedTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: --message
      value_type: string
      required: true
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
    )
    .expect_err("string parameter bounds rejected");

    assert!(err.to_string().contains("value_pattern"));

    let err = parse_registry_block(
        "non-enum-allowed-values.yaml",
        r#"tool:
  id: non-enum-tool
  name: NonEnumTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: --flag
      value_type: none
      required: false
      allowed_values: [on]
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
    )
    .expect_err("non-enum allowed_values rejected");

    assert!(err.to_string().contains("allowed_values"));

    let err = parse_registry_block(
        "string-parameter-with-range.yaml",
        r#"tool:
  id: string-range-tool
  name: StringRangeTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: --message
      value_type: string
      required: true
      value_pattern: "^[^/]+$"
      max_length: 64
      min: 1
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
    )
    .expect_err("string parameter integer range rejected");

    assert!(err.to_string().contains("min"));

    let err = parse_registry_block(
        "enum-parameter-with-string-constraints.yaml",
        r#"tool:
  id: enum-string-constraints-tool
  name: EnumStringConstraintsTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: --mode
      value_type: enum
      required: true
      allowed_values: [fast]
      value_pattern: "^[a-z]+$"
      max_length: 16
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
    )
    .expect_err("enum parameter string constraints rejected");

    assert!(err.to_string().contains("value_pattern"));

    let err = parse_registry_block(
        "none-parameter-with-string-constraints.yaml",
        r#"tool:
  id: none-string-constraints-tool
  name: NoneStringConstraintsTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: --dry-run
      value_type: none
      required: false
      value_pattern: "^(true|false)$"
      max_length: 5
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
    )
    .expect_err("none parameter string constraints rejected");

    assert!(err.to_string().contains("value_pattern"));

    let err = parse_registry_block(
        "path-parameter-with-range.yaml",
        r#"tool:
  id: path-range-tool
  name: PathRangeTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: --file
      value_type: workspace-relative-path
      required: true
      value_pattern: "^[A-Za-z0-9_./-]+$"
      max_length: 128
      min: 1
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
    )
    .expect_err("workspace path parameter integer range rejected");

    assert!(err.to_string().contains("min"));

    let err = parse_registry_block(
        "integer-parameter-with-invalid-range.yaml",
        r#"tool:
  id: integer-range-tool
  name: IntegerRangeTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: --count
      value_type: integer
      required: true
      min: 10
      max: 1
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
    )
    .expect_err("integer parameter min greater than max rejected");

    assert!(err.to_string().contains("min must be <= max"));
}

#[test]
fn parser_rejects_schema_invalid_network_ports() {
    let err = parse_registry_block(
        "zero-port.yaml",
        r#"tool:
  id: network-tool
  name: NetworkTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network:
    default: deny
    allow:
      - kind: cidr
        transport: tcp
        cidr: 192.0.2.0/24
        port: 0
"#,
    )
    .expect_err("zero port rejected");

    assert!(err.to_string().contains("network.allow.port"));
}

#[test]
fn parser_preserves_commas_inside_quoted_inline_list_scalars() {
    let block = parse_registry_block(
        "comma-argv.yaml",
        r#"tool:
  id: comma-tool
  name: CommaTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: ["--expr=a,b"]
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
    )
    .expect("tool with quoted comma parses");

    let RegistryBlock::Tool(tool) = block else {
        panic!("expected tool block");
    };
    assert_eq!(
        tool.command,
        ToolCommand::Predefined {
            command_id: "agent-echo".to_owned(),
            argv: vec!["--expr=a,b".to_owned()],
        }
    );
}

#[test]
fn parser_rejects_non_string_inline_list_scalars() {
    let err = parse_registry_block(
        "numeric-argv.yaml",
        r#"tool:
  id: numeric-tool
  name: NumericTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: [1]
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
    )
    .expect_err("numeric argv item rejected");

    assert!(err.to_string().contains("argv list items must be strings"));

    let err = parse_registry_block(
        "boolean-allowed-values.yaml",
        r#"tool:
  id: boolean-values-tool
  name: BooleanValuesTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: --mode
      value_type: enum
      required: false
      allowed_values: [false]
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
    )
    .expect_err("boolean enum value rejected");

    assert!(err
        .to_string()
        .contains("allowed_values list items must be strings"));
}

#[test]
fn parser_accepts_quoted_yaml_non_string_scalars_in_string_lists() {
    let block = parse_registry_block(
        "quoted-scalar-list.yaml",
        r#"tool:
  id: quoted-scalars-tool
  name: QuotedScalarsTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: ["1", "false"]
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
    )
    .expect("quoted scalar list items parse as strings");

    let RegistryBlock::Tool(tool) = block else {
        panic!("expected tool block");
    };
    assert_eq!(
        tool.command,
        ToolCommand::Predefined {
            command_id: "agent-echo".to_owned(),
            argv: vec!["1".to_owned(), "false".to_owned()],
        }
    );
}

#[test]
fn parser_decodes_yaml_single_quoted_apostrophes() {
    let block = parse_registry_block(
        "single-quoted-scalars.yaml",
        r#"tool:
  id: single-quoted-tool
  name: 'Bob''s Tool'
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: ['Bob''s arg']
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
    )
    .expect("single-quoted scalars parse");

    let RegistryBlock::Tool(tool) = block else {
        panic!("expected tool block");
    };
    assert_eq!(tool.identity.name, "Bob's Tool");
    assert_eq!(
        tool.command,
        ToolCommand::Predefined {
            command_id: "agent-echo".to_owned(),
            argv: vec!["Bob's arg".to_owned()],
        }
    );
}

#[test]
fn parser_rejects_malformed_yaml_single_quoted_apostrophes() {
    let err = parse_registry_block(
        "malformed-single-quoted-scalar.yaml",
        r#"tool:
  id: malformed-single-quoted-tool
  name: 'Bob's Tool'
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
    )
    .expect_err("single-quoted scalar with bare apostrophe rejected");

    assert!(err.to_string().contains("single-quoted"));
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
fn canonical_json_normalizes_string_values_to_nfc() {
    let value = serde_json::json!({
        "name": "Cafe\u{301}",
        "items": ["A\u{30a}"],
    });

    assert_eq!(
        canonical_json(&value).expect("canonical JSON"),
        "{\"items\":[\"Å\"],\"name\":\"Café\"}"
    );
}

#[test]
fn canonical_json_rejects_normalized_duplicate_keys() {
    let value = serde_json::json!({
        "é": 1,
        "e\u{301}": 2,
    });

    let err = canonical_json(&value).expect_err("normalized duplicate object keys must fail");

    assert_eq!(err.to_string(), "normalized object key collision: é");
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
    assert!(parsed["$defs"]["ipv4_cidr"]["pattern"]
        .as_str()
        .expect("IPv4 CIDR pattern")
        .contains("/(3[0-2]|[12]?[0-9])"));
    assert!(parsed["$defs"]["ipv6_cidr"]["pattern"]
        .as_str()
        .expect("IPv6 CIDR pattern")
        .contains("/(12[0-8]|1[01][0-9]|[1-9]?[0-9])"));
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
        SemanticValidationError::ToolSchemaViolation { message, .. }
            if message.contains("script_runtime")
    ));

    let predefined = ToolBlock {
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
        SemanticValidationError::ToolSchemaViolation { message, .. }
            if message.contains("omit script_runtime")
    ));
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
        instruction_refs: Vec::new(),
        tool_refs: Vec::new(),
        steps: vec![StepBlock {
            id: "step".to_owned(),
            name: "Step".to_owned(),
            connection_refs: Vec::new(),
        }],
    })
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
