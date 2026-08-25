use super::super::helpers::empty_workspace;
use super::support::{absent_global_home, authoring_workspace, padded_instruction};
use crate::runtime::authoring::set_create_post_validation_observer;
use crate::runtime::m11_budget_evidence::maximum_tool;
use crate::runtime::types::RuntimeError;
use crate::{create_global_registry_block, validate_global_registry};
use core_script::{
    MAX_REGISTRY_ENTRIES, MAX_REGISTRY_FILE_BYTES, MAX_REGISTRY_TOTAL_BYTES, RegistryBlock,
    ToolCommand, parse_registry_block,
};
use std::fs;

#[test]
fn authoring_definition_budget() {
    let workspace = authoring_workspace("authoring-definition-budget");
    let path = workspace.join("registry/instructions/instruction.yaml");
    let limit = usize::try_from(MAX_REGISTRY_FILE_BYTES).expect("file limit fits usize");
    fs::write(&path, padded_instruction("definition-limit", limit))
        .expect("boundary definition is written");

    validate_global_registry(None).expect("128 KiB definition is accepted");

    fs::write(&path, padded_instruction("definition-limit", limit + 1))
        .expect("excess definition is written");
    let error = validate_global_registry(None).expect_err("128 KiB plus one byte is rejected");
    assert!(error.to_string().contains("131073"), "{error}");
}

#[test]
fn authoring_registry_byte_budget() {
    let workspace = authoring_workspace("authoring-registry-byte-budget");
    let per_entry = usize::try_from(MAX_REGISTRY_TOTAL_BYTES).expect("registry limit fits usize")
        / MAX_REGISTRY_ENTRIES;
    for index in 0..MAX_REGISTRY_ENTRIES {
        let id = format!("byte-{index:04}");
        fs::write(
            workspace.join(format!("registry/instructions/{id}.yaml")),
            padded_instruction(&id, per_entry),
        )
        .expect("boundary registry entry is written");
    }

    validate_global_registry(None).expect("16 MiB registry is accepted");

    let first = workspace.join("registry/instructions/byte-0000.yaml");
    fs::write(&first, padded_instruction("byte-0000", per_entry + 1))
        .expect("excess registry byte is written");
    let error = validate_global_registry(None).expect_err("16 MiB plus one byte is rejected");
    assert!(error.to_string().contains("16777217"), "{error}");
}

#[test]
fn authoring_create_counts_the_candidate_toward_the_registry_byte_budget() {
    let workspace = authoring_workspace("authoring-create-registry-byte-budget");
    let per_entry = usize::try_from(MAX_REGISTRY_FILE_BYTES).expect("file limit fits usize");
    let entries = usize::try_from(MAX_REGISTRY_TOTAL_BYTES / MAX_REGISTRY_FILE_BYTES)
        .expect("entry count fits usize");
    for index in 0..entries {
        let id = format!("create-byte-{index:04}");
        fs::write(
            workspace.join(format!("registry/instructions/{id}.yaml")),
            padded_instruction(&id, per_entry),
        )
        .expect("boundary registry entry is written");
    }
    validate_global_registry(None).expect("full registry is accepted");

    let error = create_global_registry_block(maximum_tool())
        .expect_err("candidate beyond the aggregate registry budget is rejected");
    assert!(error.to_string().contains("16908288"), "{error}");
    assert!(!workspace.join("registry/tools/maximum-tool.yaml").exists());
}

#[test]
fn authoring_registry_entry_budget() {
    let workspace = authoring_workspace("authoring-registry-entry-budget");
    for index in 0..MAX_REGISTRY_ENTRIES {
        let id = format!("entry-{index:04}");
        fs::write(
            workspace.join(format!("registry/instructions/{id}.yaml")),
            padded_instruction(&id, 128),
        )
        .expect("boundary registry entry is written");
    }

    validate_global_registry(None).expect("1,024 definitions are accepted");
    let error = create_global_registry_block(maximum_tool())
        .expect_err("authoring cannot publish definition 1,025");
    assert!(error.to_string().contains("entry count"), "{error}");
    assert!(!workspace.join("registry/tools/maximum-tool.yaml").exists());

    fs::write(
        workspace.join("registry/instructions/entry-excess.yaml"),
        padded_instruction("entry-excess", 128),
    )
    .expect("excess registry entry is written");
    let error = validate_global_registry(None).expect_err("definition 1,025 is rejected");
    assert!(error.to_string().contains("1025"), "{error}");
}

#[test]
fn authoring_transaction_roundtrip() {
    let _workspace = authoring_workspace("authoring-transaction-roundtrip");
    let block = maximum_tool();

    let path = create_global_registry_block(block.clone()).expect("Tool is published");

    let bytes = fs::read(&path).expect("published Tool is readable");
    assert_eq!(
        bytes.len(),
        usize::try_from(MAX_REGISTRY_FILE_BYTES).unwrap()
    );
    let source = String::from_utf8(bytes).expect("published Tool is UTF-8");
    assert_eq!(
        parse_registry_block("maximum-tool.yaml", &source).expect("published Tool reloads"),
        block
    );
    validate_global_registry(None).expect("published registry validates");
}

#[test]
fn authoring_publishes_each_approved_block_kind_and_validates_a_selected_flow() {
    let _workspace = authoring_workspace("authoring-all-block-kinds");
    let definitions = [
        (
            "tool.yaml",
            "tool:\n  id: inspect\n  name: Inspect\n  tool_kind: predefined-command\n  command:\n    command_id: agent-echo\n    argv: [inspect]\n  allowed_parameters: []\n  read_scope: [workspace]\n  write_scope: []\n  protected_path_grants: []\n  network: deny\n",
        ),
        (
            "instruction.yaml",
            "instruction:\n  id: review\n  name: Review\n  prompt: Review the project.\n",
        ),
        (
            "phase.yaml",
            "phase:\n  id: review\n  name: Review\n  instruction_refs: [review]\n  tool_refs: [inspect]\n  output:\n    type: string\n",
        ),
        (
            "flow.yaml",
            "flow:\n  id: review\n  name: Review\n  phase_refs: [review]\n  subflow_refs: []\n",
        ),
    ];

    let mut published = Vec::new();
    for (source_name, source) in definitions {
        let block = parse_registry_block(source_name, source).expect("definition parses");
        published.push(create_global_registry_block(block).expect("approved block kind publishes"));
    }
    assert_eq!(
        published
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        ["inspect.yaml", "review.yaml", "review.yaml", "review.yaml"]
    );
    validate_global_registry(Some("review")).expect("selected authored Flow closure validates");
    let error =
        validate_global_registry(Some("missing")).expect_err("unknown selected Flow is rejected");
    assert!(
        error
            .to_string()
            .contains("registry root references missing flow missing"),
        "{error}"
    );
}

#[test]
fn authoring_uninitialized_global_config_uses_runtime_exit_class() {
    absent_global_home();
    let workspace = empty_workspace("authoring-uninitialized-global-config");
    let error = create_global_registry_block(maximum_tool())
        .expect_err("uninitialized global config is rejected");

    assert_eq!(error.exit_code(), 65);
    assert!(
        error
            .to_string()
            .contains("global Flow config is not initialized")
    );
    assert!(!workspace.join(".flow").exists());
}

#[cfg(any(unix, windows))]
#[test]
fn authoring_rejects_a_hardlinked_global_config_before_publication() {
    let workspace = authoring_workspace("authoring-hardlinked-config");
    let outside = empty_workspace("authoring-hardlinked-config-outside");
    let config_path = workspace.join("config.yaml");
    let outside_config = outside.join("config.yaml");
    fs::write(&outside_config, "registry_root: registry\n").expect("outside config writes");
    fs::remove_file(&config_path).expect("global config removes");
    fs::hard_link(&outside_config, &config_path).expect("global config hard links");

    let error = create_global_registry_block(maximum_tool())
        .expect_err("hardlinked config is rejected before authoring publication");

    assert!(
        matches!(&error, RuntimeError::Protocol(message) if message.contains("hard-linked")),
        "unexpected error: {error:?}"
    );
    assert!(!workspace.join("registry/tools/maximum-tool.yaml").exists());
}

#[test]
fn authoring_publish_is_atomic_for_missing_duplicate_and_oversized_definitions() {
    let workspace = authoring_workspace("authoring-publish-atomicity");
    fs::remove_dir(workspace.join("registry/tools")).expect("Tool directory is removed");
    let block = maximum_tool();
    let error = create_global_registry_block(block.clone())
        .expect_err("missing kind directory is rejected");
    assert_eq!(error.exit_code(), 65);
    assert!(error.to_string().contains("does not exist"), "{error}");
    assert!(!workspace.join("registry/tools/maximum-tool.yaml").exists());

    fs::create_dir(workspace.join("registry/tools")).expect("Tool directory is restored");
    create_global_registry_block(block.clone()).expect("first publication succeeds");
    let persisted = fs::read(workspace.join("registry/tools/maximum-tool.yaml"))
        .expect("published definition is readable");
    let error = create_global_registry_block(block.clone())
        .expect_err("duplicate publication never overwrites");
    assert!(matches!(
        error,
        RuntimeError::DefinitionExists {
            definition_kind: "tool",
            ref definition_id,
            ..
        } if definition_id == "maximum-tool"
    ));
    assert_eq!(
        fs::read(workspace.join("registry/tools/maximum-tool.yaml"))
            .expect("definition remains readable"),
        persisted
    );

    let mut oversized = block;
    let RegistryBlock::Tool(tool) = &mut oversized else {
        unreachable!("fixture is a Tool")
    };
    let ToolCommand::Predefined { argv, .. } = &mut tool.command else {
        unreachable!("fixture is predefined")
    };
    argv[0].push('x');
    let error = create_global_registry_block(oversized)
        .expect_err("generated definition byte budget is enforced before publication");
    assert!(
        error.to_string().contains("generated registry definition"),
        "{error}"
    );
}

#[test]
fn concurrent_authoring_serializes_registry_validation_and_publication() {
    let _workspace = authoring_workspace("authoring-concurrent-publication");
    let mut first = maximum_tool();
    let mut second = first.clone();
    for (block, id) in [(&mut first, "first"), (&mut second, "second")] {
        let RegistryBlock::Tool(tool) = block else {
            unreachable!("fixture is a Tool")
        };
        tool.identity.id = id.to_owned();
        tool.identity.name = "SharedName".to_owned();
        let ToolCommand::Predefined { argv, .. } = &mut tool.command else {
            unreachable!("fixture is predefined")
        };
        argv[0] = "echo".to_owned();
    }

    let (start_sender, start_receiver) = std::sync::mpsc::channel();
    let (done_sender, done_receiver) = std::sync::mpsc::channel();
    let second_create = std::thread::spawn(move || {
        start_receiver.recv().expect("second create starts");
        let result = create_global_registry_block(second);
        let _ = done_sender.send(());
        result
    });
    set_create_post_validation_observer(move || {
        start_sender.send(()).expect("concurrent create starts");
        let _ = done_receiver.recv_timeout(std::time::Duration::from_secs(2));
    });

    let first_result = create_global_registry_block(first);
    let second_result = second_create.join().expect("second create joins");
    assert_eq!(
        usize::from(first_result.is_ok()) + usize::from(second_result.is_ok()),
        1,
        "only one duplicate-name definition may publish: first={first_result:?}, second={second_result:?}"
    );
    validate_global_registry(None).expect("the concurrently updated registry remains unambiguous");
}
