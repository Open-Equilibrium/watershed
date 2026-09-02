use super::{
    flow_command,
    process::{cli_child_watchdog, wait_with_input_and_output_before, wait_with_output_before},
    test_support::{empty_workspace, session_home_path},
};
use core_script::{RegistryBlock, ToolCommand, parse_registry_block};
use std::{fs, path::Path, process::Stdio};

fn initialize_default_workspace(workspace: &Path) {
    let output = flow_command()
        .current_dir(workspace)
        .arg("init")
        .output()
        .expect("workspace initialization should run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn init_create_and_validate_custom_recursive_flow() {
    let workspace = empty_workspace();
    initialize_default_workspace(&workspace);
    let global_home = session_home_path();
    assert_eq!(
        fs::read_to_string(global_home.join("config.yaml")).expect("config is readable"),
        "registry_root: \"registry\"\n"
    );
    for kind in ["tools", "instructions", "phases", "flows"] {
        assert!(global_home.join("registry").join(kind).is_dir());
    }
    let repeated_init = flow_command()
        .current_dir(&workspace)
        .arg("init")
        .output()
        .expect("repeated init should run");
    assert!(!repeated_init.status.success());

    let contract = workspace.join("string-contract.yaml");
    fs::write(&contract, "type: string\nmax_length: 4096\n").expect("contract written");
    let predicate = workspace.join("done-predicate.yaml");
    fs::write(
        &predicate,
        "path: []\nequals:\n  type: string\n  value: done\n",
    )
    .expect("predicate written");
    let prompt = workspace.join("SYSTEM.md");
    fs::write(&prompt, "Review {{project}} and return done.").expect("prompt written");
    let script = workspace.join("report.sh");
    fs::write(&script, "printf '%s' \"$1\"").expect("script written");

    let commands = [
        vec![
            "create",
            "tool",
            "--id",
            "report",
            "--name",
            "Report",
            "--tool-kind",
            "predefined-command",
            "--command-id",
            "agent-report",
            "--max-concurrent-processes-and-threads",
            "8",
            "--argv",
            "--fixed",
            "--parameter",
            "--parameter-name",
            "--project",
            "--parameter-value-type",
            "string",
            "--parameter-required",
            "true",
            "--parameter-value-pattern",
            "^[A-Za-z0-9_-]+$",
            "--parameter-max-length",
            "64",
            "--end-parameter",
            "--parameter",
            "--parameter-name",
            "--mode",
            "--parameter-value-type",
            "enum",
            "--parameter-required",
            "true",
            "--parameter-allowed-value",
            "fast",
            "--parameter-allowed-value",
            "thorough",
            "--end-parameter",
            "--parameter",
            "--parameter-name",
            "--count",
            "--parameter-value-type",
            "integer",
            "--parameter-required",
            "false",
            "--parameter-min",
            "1",
            "--parameter-max",
            "3",
            "--end-parameter",
            "--parameter",
            "--parameter-name",
            "--verbose",
            "--parameter-value-type",
            "none",
            "--parameter-required",
            "false",
            "--end-parameter",
            "--parameter",
            "--parameter-name",
            "--output",
            "--parameter-value-type",
            "workspace-relative-path",
            "--parameter-required",
            "true",
            "--parameter-value-pattern",
            "^[A-Za-z0-9_./-]+$",
            "--parameter-max-length",
            "128",
            "--end-parameter",
            "--runtime-profile",
            "host-system-read",
            "--read-only-mount",
            "workspace",
            "--writable-mount",
            "workspace/reports",
            "--network-default",
            "deny",
            "--network-allow",
            "--network-kind",
            "cidr",
            "--network-transport",
            "tcp",
            "--network-cidr",
            "127.0.0.1/32",
            "--network-port",
            "443",
            "--end-network-allow",
            "--network-allow",
            "--network-kind",
            "cidr",
            "--network-transport",
            "udp",
            "--network-cidr",
            "::1/128",
            "--network-port",
            "53",
            "--end-network-allow",
        ],
        vec![
            "create",
            "tool",
            "--id",
            "script-report",
            "--name",
            "ScriptReport",
            "--tool-kind",
            "own-script",
            "--script-body-file",
            "report.sh",
            "--max-concurrent-processes-and-threads",
            "8",
            "--network-default",
            "deny",
        ],
        vec![
            "create",
            "instruction",
            "--id",
            "review",
            "--name",
            "Review",
            "--prompt-file",
            "SYSTEM.md",
            "--parameter",
            "--parameter-name",
            "project",
            "--parameter-contract-file",
            "string-contract.yaml",
            "--end-parameter",
        ],
        vec![
            "create",
            "phase",
            "--id",
            "review-leaf",
            "--name",
            "ReviewLeaf",
            "--instruction-ref",
            "review",
            "--tool-ref",
            "report",
            "--tool-ref",
            "script-report",
            "--output-contract-file",
            "string-contract.yaml",
            "--loop",
            "--loop-max-iterations",
            "2",
            "--loop-until-file",
            "done-predicate.yaml",
            "--end-loop",
        ],
        vec![
            "create",
            "phase",
            "--id",
            "publish-leaf",
            "--name",
            "PublishLeaf",
            "--instruction-ref",
            "review",
            "--tool-ref",
            "report",
            "--output-contract-file",
            "string-contract.yaml",
        ],
        vec![
            "create",
            "phase",
            "--id",
            "review-cycle",
            "--name",
            "ReviewCycle",
            "--phase-ref",
            "review-leaf",
            "--phase-ref",
            "publish-leaf",
            "--output-contract-file",
            "string-contract.yaml",
            "--result-from",
            "publish-leaf",
            "--transition",
            "--transition-from-phase-ref",
            "review-leaf",
            "--transition-to-phase-ref",
            "publish-leaf",
            "--transition-when-file",
            "done-predicate.yaml",
            "--end-transition",
        ],
        vec![
            "create",
            "flow",
            "--id",
            "publish-subflow",
            "--name",
            "PublishSubflow",
            "--phase-ref",
            "publish-leaf",
        ],
        vec![
            "create",
            "flow",
            "--id",
            "custom-review",
            "--name",
            "CustomReview",
            "--phase-ref",
            "review-leaf",
            "--phase-ref",
            "publish-leaf",
            "--subflow-ref",
            "publish-subflow",
            "--transition",
            "--transition-from-phase-ref",
            "review-leaf",
            "--transition-to-phase-ref",
            "publish-leaf",
            "--transition-when-file",
            "done-predicate.yaml",
            "--end-transition",
        ],
    ];
    for args in commands {
        let output = flow_command()
            .current_dir(&workspace)
            .args(args)
            .output()
            .expect("create should run");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let validate = flow_command()
        .current_dir(&workspace)
        .args(["validate", "custom-review"])
        .output()
        .expect("validate should run");
    assert!(
        validate.status.success(),
        "{}",
        String::from_utf8_lossy(&validate.stderr)
    );
    let validate_all = flow_command()
        .current_dir(&workspace)
        .arg("validate")
        .output()
        .expect("full validation should run");
    assert!(
        validate_all.status.success(),
        "{}",
        String::from_utf8_lossy(&validate_all.stderr)
    );

    let duplicate = flow_command()
        .current_dir(&workspace)
        .args([
            "create",
            "flow",
            "--id",
            "custom-review",
            "--name",
            "Replacement",
            "--phase-ref",
            "review-cycle",
        ])
        .output()
        .expect("duplicate create should run");
    assert!(!duplicate.status.success());
    let original = fs::read_to_string(global_home.join("registry/flows/custom-review.yaml"))
        .expect("original flow remains readable");
    assert!(original.contains("CustomReview"));
    assert!(!original.contains("Replacement"));

    let unresolved = flow_command()
        .current_dir(&workspace)
        .args([
            "create",
            "phase",
            "--id",
            "unresolved",
            "--name",
            "Unresolved",
            "--phase-ref",
            "missing-phase",
            "--output-contract-file",
            "string-contract.yaml",
            "--result-from",
            "missing-phase",
        ])
        .output()
        .expect("unresolved create should run");
    assert!(!unresolved.status.success());
    assert!(!global_home.join("registry/phases/unresolved.yaml").exists());
}

#[test]
fn custom_registry_root_accepts_instruction_and_script_stdin_sources() {
    let workspace = empty_workspace();
    let init = flow_command()
        .current_dir(&workspace)
        .args(["init", "--registry-root", "custom-registry"])
        .output()
        .expect("custom init should run");
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    let global_home = session_home_path();
    assert_eq!(
        fs::read_to_string(global_home.join("config.yaml")).expect("config is readable"),
        "registry_root: \"custom-registry\"\n"
    );

    let instruction = flow_command()
        .current_dir(&workspace)
        .args([
            "create",
            "instruction",
            "--id",
            "stdin-instruction",
            "--name",
            "StdinInstruction",
            "--prompt-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("instruction create should start");
    let instruction = wait_with_input_and_output_before(
        instruction,
        b"Review the selected project.",
        cli_child_watchdog(),
    );
    assert!(
        instruction.status.success(),
        "{}",
        String::from_utf8_lossy(&instruction.stderr)
    );

    let tool = flow_command()
        .current_dir(&workspace)
        .args([
            "create",
            "tool",
            "--id",
            "stdin-script",
            "--name",
            "StdinScript",
            "--tool-kind",
            "own-script",
            "--script-body-stdin",
            "--max-concurrent-processes-and-threads",
            "8",
            "--network",
            "deny",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Tool create should start");
    let tool =
        wait_with_input_and_output_before(tool, b"printf '%s' stdin-script", cli_child_watchdog());
    assert!(
        tool.status.success(),
        "{}",
        String::from_utf8_lossy(&tool.stderr)
    );

    fs::write(workspace.join("string-contract.yaml"), "type: string\n").expect("contract writes");
    for args in [
        [
            "create",
            "phase",
            "--id",
            "stdin-phase",
            "--name",
            "StdinPhase",
            "--instruction-ref",
            "stdin-instruction",
            "--tool-ref",
            "stdin-script",
            "--output-contract-file",
            "string-contract.yaml",
        ]
        .as_slice(),
        [
            "create",
            "flow",
            "--id",
            "stdin-flow",
            "--name",
            "StdinFlow",
            "--phase-ref",
            "stdin-phase",
        ]
        .as_slice(),
    ] {
        let output = flow_command()
            .current_dir(&workspace)
            .args(args)
            .output()
            .expect("block create should run");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let validate = flow_command()
        .current_dir(&workspace)
        .args(["validate", "stdin-flow"])
        .output()
        .expect("validate should run");
    assert!(
        validate.status.success(),
        "{}",
        String::from_utf8_lossy(&validate.stderr)
    );
    assert!(
        session_home_path()
            .join("custom-registry/flows/stdin-flow.yaml")
            .is_file()
    );
}

#[test]
fn duplicate_prompt_source_is_rejected_without_reading_stdin() {
    let workspace = empty_workspace();
    fs::write(workspace.join("prompt.txt"), "Review the project.").expect("prompt writes");
    let child = flow_command()
        .current_dir(&workspace)
        .args([
            "create",
            "instruction",
            "--id",
            "review",
            "--name",
            "Review",
            "--prompt-file",
            "prompt.txt",
            "--prompt-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("instruction create should start");

    let output = wait_with_output_before(child, cli_child_watchdog());

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("duplicate --prompt-stdin"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn invalid_stdin_sources_are_rejected_without_reading_stdin() {
    let workspace = empty_workspace();
    fs::write(workspace.join("prompt.txt"), "Review the project.").expect("prompt writes");

    for (arguments, expected) in [
        (
            vec![
                "create",
                "instruction",
                "--id",
                "review",
                "--name",
                "Review",
                "--prompt-stdin",
                "--prompt-file",
                "prompt.txt",
            ],
            "duplicate --prompt-file",
        ),
        (
            vec![
                "create",
                "tool",
                "--id",
                "report",
                "--name",
                "Report",
                "--tool-kind",
                "predefined-command",
                "--command-id",
                "agent-report",
                "--script-body-stdin",
                "--max-concurrent-processes-and-threads",
                "8",
                "--network",
                "deny",
            ],
            "script body is invalid for predefined-command",
        ),
    ] {
        let child = flow_command()
            .current_dir(&workspace)
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("invalid authoring command should start");

        let output = wait_with_output_before(child, cli_child_watchdog());

        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn invalid_stdin_backed_identities_are_rejected_before_reading_stdin() {
    let workspace = empty_workspace();
    initialize_default_workspace(&workspace);

    for (kind, arguments) in [
        (
            "instructions",
            vec![
                "create",
                "instruction",
                "--id",
                "INVALID",
                "--name",
                "Invalid",
                "--prompt-stdin",
            ],
        ),
        (
            "tools",
            vec![
                "create",
                "tool",
                "--id",
                "INVALID",
                "--name",
                "Invalid",
                "--tool-kind",
                "own-script",
                "--script-body-stdin",
                "--max-concurrent-processes-and-threads",
                "8",
                "--network",
                "deny",
            ],
        ),
    ] {
        let child = flow_command()
            .current_dir(&workspace)
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("invalid authoring command should start");

        let output = wait_with_output_before(child, cli_child_watchdog());
        let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
        assert_eq!(output.status.code(), Some(65), "{kind}: {stderr}");
        assert!(stderr.contains("invalid_definition"), "{kind}: {stderr}");
        assert!(output.stdout.is_empty(), "{kind}");
        assert!(
            !workspace
                .join("registry")
                .join(kind)
                .join("INVALID.yaml")
                .exists()
        );
    }
}

#[test]
fn tool_argv_preserves_literal_help_flags() {
    let workspace = empty_workspace();
    initialize_default_workspace(&workspace);

    for (id, literal) in [("long-help", "--help"), ("short-help", "-h")] {
        let create = flow_command()
            .current_dir(&workspace)
            .args([
                "create",
                "tool",
                "--id",
                id,
                "--name",
                id,
                "--tool-kind",
                "predefined-command",
                "--command-id",
                "agent-echo",
                "--max-concurrent-processes-and-threads",
                "8",
                "--network",
                "deny",
                "--argv",
                literal,
            ])
            .output()
            .expect("tool creation should run");
        assert!(
            create.status.success(),
            "{}",
            String::from_utf8_lossy(&create.stderr)
        );

        let path = session_home_path().join(format!("registry/tools/{id}.yaml"));
        let source = fs::read_to_string(&path).expect("tool definition should be readable");
        let block = parse_registry_block(&path.to_string_lossy(), &source)
            .expect("tool definition should parse");
        let RegistryBlock::Tool(tool) = block else {
            panic!("created definition should be a tool");
        };
        let ToolCommand::Predefined { argv, .. } = tool.command else {
            panic!("created tool should be a predefined command");
        };
        assert_eq!(argv, [literal]);
    }
}

#[test]
fn authoring_help_exposes_each_complete_block_grammar() {
    for (kind, required_fragments) in [
        (
            "instruction",
            &[
                "--prompt-file",
                "--prompt-stdin",
                "--parameter-name",
                "--parameter-contract-file",
                "--end-parameter",
            ][..],
        ),
        (
            "phase",
            &[
                "--instruction-ref",
                "--tool-ref",
                "--phase-ref",
                "--output-contract-file",
                "--result-from",
                "--loop-max-iterations",
                "--end-loop",
                "--transition-from-phase-ref",
                "--end-transition",
            ][..],
        ),
        (
            "flow",
            &[
                "--phase-ref",
                "--subflow-ref",
                "--transition-from-phase-ref",
                "--end-transition",
            ][..],
        ),
        (
            "tool",
            &[
                "predefined-command",
                "own-script",
                "--command-id",
                "--script-body-file",
                "--parameter-value-type",
                "--end-parameter",
                "--network deny",
                "--network-allow",
                "--end-network-allow",
            ][..],
        ),
    ] {
        let output = flow_command()
            .args(["create", kind, "--help"])
            .output()
            .expect("authoring help should run");
        assert!(output.status.success(), "{kind}");
        let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
        assert!(
            stdout.starts_with(&format!("Usage:\n  flow create {kind} ")),
            "{stdout}"
        );
        for fragment in required_fragments {
            assert!(
                stdout.contains(fragment),
                "{kind} help lacks {fragment}: {stdout}"
            );
        }
        assert!(output.stderr.is_empty(), "{kind}");
    }
}

#[test]
fn authoring_public_diagnostics_and_exit_classes_are_stable() {
    fn assert_error(workspace: &Path, args: &[&str], code: i32, diagnostic: &str) {
        let output = flow_command()
            .current_dir(workspace)
            .args(args)
            .output()
            .expect("authoring command should run");
        let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

        assert_eq!(output.status.code(), Some(code), "{args:?}: {stderr}");
        assert!(stderr.contains(diagnostic), "{args:?}: {stderr}");
        assert!(output.stdout.is_empty(), "{args:?}");
    }

    let workspace = empty_workspace();
    initialize_default_workspace(&workspace);

    assert_error(
        &workspace,
        &["init"],
        65,
        "global_config_already_initialized",
    );

    let tool_args = [
        "create",
        "tool",
        "--id",
        "inspect",
        "--name",
        "Inspect",
        "--tool-kind",
        "predefined-command",
        "--command-id",
        "agent-echo",
        "--max-concurrent-processes-and-threads",
        "8",
        "--network",
        "deny",
    ];
    let created = flow_command()
        .current_dir(&workspace)
        .args(tool_args)
        .output()
        .expect("valid Tool creation should run");
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );

    assert_error(&workspace, &tool_args, 65, "definition_exists");
    assert_error(
        &workspace,
        &[
            "create",
            "tool",
            "--id",
            "INVALID",
            "--name",
            "Invalid",
            "--tool-kind",
            "predefined-command",
            "--command-id",
            "agent-echo",
            "--max-concurrent-processes-and-threads",
            "8",
            "--network",
            "deny",
        ],
        65,
        "invalid_definition",
    );
    assert_error(
        &workspace,
        &["validate", "missing"],
        65,
        "invalid_reference",
    );
    assert_error(&workspace, &["validate", "--bogus"], 64, "unknown argument");
    assert_error(&workspace, &["init", "--bogus"], 64, "unknown argument");
}

#[test]
fn malformed_typed_yaml_is_invalid_definition_and_never_published() {
    let workspace = empty_workspace();
    initialize_default_workspace(&workspace);
    fs::write(workspace.join("valid-contract.yaml"), "type: string\n")
        .expect("valid contract should be written");
    fs::write(workspace.join("malformed.yaml"), "type: [\n")
        .expect("malformed YAML should be written");

    for (id, arguments) in [
        (
            "malformed-contract",
            vec![
                "create",
                "phase",
                "--id",
                "malformed-contract",
                "--name",
                "MalformedContract",
                "--output-contract-file",
                "malformed.yaml",
            ],
        ),
        (
            "malformed-predicate",
            vec![
                "create",
                "phase",
                "--id",
                "malformed-predicate",
                "--name",
                "MalformedPredicate",
                "--output-contract-file",
                "valid-contract.yaml",
                "--loop",
                "--loop-max-iterations",
                "1",
                "--loop-until-file",
                "malformed.yaml",
                "--end-loop",
            ],
        ),
    ] {
        let output = flow_command()
            .current_dir(&workspace)
            .args(arguments)
            .output()
            .expect("invalid authoring command should run");
        let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

        assert_eq!(output.status.code(), Some(65), "{id}: {stderr}");
        assert!(stderr.contains("invalid_definition"), "{id}: {stderr}");
        assert!(output.stdout.is_empty(), "{id}");
        assert!(
            !session_home_path()
                .join("registry/phases")
                .join(format!("{id}.yaml"))
                .exists(),
            "malformed typed input must not publish {id}"
        );
    }
}
