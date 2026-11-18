use super::{
    flow_command,
    process::{cli_child_watchdog, wait_with_input_and_output_before, wait_with_output_before},
    test_support::{empty_workspace, session_home_path},
};
use core_script::{RegistryBlock, ToolCommand, parse_registry_block};
use std::{
    fs,
    path::Path,
    process::{Child, Output, Stdio},
};

fn flow_in(workspace: &Path, command: &str) -> Output {
    flow_command()
        .current_dir(workspace)
        .args(command.split_whitespace())
        .output()
        .expect("flow command should run")
}

fn succeed_in(workspace: &Path, command: &str) {
    let output = flow_in(workspace, command);
    assert!(
        output.status.success(),
        "{command}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn spawn_with_piped_stdin(workspace: &Path, command: &str) -> Child {
    flow_command()
        .current_dir(workspace)
        .args(command.split_whitespace())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("authoring command should start")
}

fn succeed_with_stdin(workspace: &Path, command: &str, input: &[u8]) {
    let output = wait_with_input_and_output_before(
        spawn_with_piped_stdin(workspace, command),
        input,
        cli_child_watchdog(),
    );
    assert!(
        output.status.success(),
        "{command}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_rejected(workspace: &Path, command: &str, code: i32, diagnostic: &str) {
    let output = wait_with_output_before(
        spawn_with_piped_stdin(workspace, command),
        cli_child_watchdog(),
    );
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert_eq!(output.status.code(), Some(code), "{command}: {stderr}");
    assert!(stderr.contains(diagnostic), "{command}: {stderr}");
    assert!(output.stdout.is_empty(), "{command}");
}

#[test]
fn init_create_and_validate_custom_recursive_flow() {
    let workspace = empty_workspace();
    succeed_in(&workspace, "init");
    let global_home = session_home_path();
    assert_eq!(
        fs::read_to_string(global_home.join("config.yaml")).expect("config is readable"),
        "registry_root: \"registry\"\n"
    );
    for kind in ["tools", "instructions", "phases", "flows"] {
        assert!(global_home.join("registry").join(kind).is_dir());
    }

    fs::write(
        workspace.join("string-contract.yaml"),
        "type: string\nmax_length: 4096\n",
    )
    .expect("contract written");
    fs::write(
        workspace.join("done-predicate.yaml"),
        "path: []\nequals:\n  type: string\n  value: done\n",
    )
    .expect("predicate written");
    fs::write(
        workspace.join("SYSTEM.md"),
        "Review {{project}} and return done.",
    )
    .expect("prompt written");
    fs::write(workspace.join("report.sh"), "printf '%s' \"$1\"").expect("script written");

    for command in [
        "create tool --id report --name Report --tool-kind predefined-command
            --command-id agent-report --argv --fixed --parameter --parameter-name --project
            --parameter-value-type string --parameter-required true
            --parameter-value-pattern ^[A-Za-z0-9_-]+$ --parameter-max-length 64 --end-parameter
            --parameter --parameter-name --mode --parameter-value-type enum
            --parameter-required true --parameter-allowed-value fast
            --parameter-allowed-value thorough --end-parameter
            --parameter --parameter-name --count --parameter-value-type integer
            --parameter-required false --parameter-min 1 --parameter-max 3 --end-parameter
            --parameter --parameter-name --verbose --parameter-value-type none
            --parameter-required false --end-parameter
            --parameter --parameter-name --output --parameter-value-type workspace-relative-path
            --parameter-required true --parameter-value-pattern ^[A-Za-z0-9_./-]+$
            --parameter-max-length 128 --end-parameter",
        "create tool --id script-report --name ScriptReport --tool-kind own-script
            --script-body-file report.sh",
        "create instruction --id review --name Review --prompt-file SYSTEM.md --parameter
            --parameter-name project --parameter-contract-file string-contract.yaml --end-parameter",
        "create phase --id review-leaf --name ReviewLeaf --instruction-ref review
            --tool-ref report --tool-ref script-report --output-contract-file string-contract.yaml
            --loop --loop-max-iterations 2 --loop-until-file done-predicate.yaml --end-loop",
        "create phase --id publish-leaf --name PublishLeaf --instruction-ref review
            --tool-ref report --output-contract-file string-contract.yaml",
        "create phase --id review-cycle --name ReviewCycle --phase-ref review-leaf
            --phase-ref publish-leaf --output-contract-file string-contract.yaml
            --result-from publish-leaf --transition --transition-from-phase-ref review-leaf
            --transition-to-phase-ref publish-leaf --transition-when-file done-predicate.yaml
            --end-transition",
        "create flow --id publish-subflow --name PublishSubflow --phase-ref publish-leaf",
        "create flow --id custom-review --name CustomReview --phase-ref review-leaf
            --phase-ref publish-leaf --subflow-ref publish-subflow --transition
            --transition-from-phase-ref review-leaf --transition-to-phase-ref publish-leaf
            --transition-when-file done-predicate.yaml --end-transition",
    ] {
        succeed_in(&workspace, command);
    }

    succeed_in(&workspace, "validate custom-review");
    succeed_in(&workspace, "validate");

    assert!(
        !flow_in(
            &workspace,
            "create flow --id custom-review --name Replacement --phase-ref review-cycle"
        )
        .status
        .success()
    );
    let original = fs::read_to_string(global_home.join("registry/flows/custom-review.yaml"))
        .expect("original flow remains readable");
    assert!(original.contains("CustomReview"));
    assert!(!original.contains("Replacement"));

    assert!(
        !flow_in(
            &workspace,
            "create phase --id unresolved --name Unresolved --phase-ref missing-phase
                --output-contract-file string-contract.yaml --result-from missing-phase"
        )
        .status
        .success()
    );
    assert!(!global_home.join("registry/phases/unresolved.yaml").exists());
}

#[test]
fn custom_registry_root_accepts_instruction_and_script_stdin_sources() {
    let workspace = empty_workspace();
    succeed_in(&workspace, "init --registry-root custom-registry");
    assert_eq!(
        fs::read_to_string(session_home_path().join("config.yaml")).expect("config is readable"),
        "registry_root: \"custom-registry\"\n"
    );

    succeed_with_stdin(
        &workspace,
        "create instruction --id stdin-instruction --name StdinInstruction --prompt-stdin",
        b"Review the selected project.",
    );
    succeed_with_stdin(
        &workspace,
        "create tool --id stdin-script --name StdinScript --tool-kind own-script --script-body-stdin",
        b"printf '%s' stdin-script",
    );
    fs::write(workspace.join("string-contract.yaml"), "type: string\n").expect("contract writes");
    succeed_in(
        &workspace,
        "create phase --id stdin-phase --name StdinPhase --instruction-ref stdin-instruction
            --tool-ref stdin-script --output-contract-file string-contract.yaml",
    );
    succeed_in(
        &workspace,
        "create flow --id stdin-flow --name StdinFlow --phase-ref stdin-phase",
    );
    succeed_in(&workspace, "validate stdin-flow");
    assert!(
        session_home_path()
            .join("custom-registry/flows/stdin-flow.yaml")
            .is_file()
    );
}

#[test]
fn stdin_authoring_rejects_invalid_sources_and_identities_before_reading_stdin() {
    let workspace = empty_workspace();
    succeed_in(&workspace, "init");
    fs::write(workspace.join("prompt.txt"), "Review the project.").expect("prompt writes");

    for (command, code, diagnostic) in [
        (
            "create instruction --id review --name Review --prompt-file prompt.txt --prompt-stdin",
            64,
            "duplicate --prompt-stdin",
        ),
        (
            "create instruction --id review --name Review --prompt-stdin --prompt-file prompt.txt",
            64,
            "duplicate --prompt-file",
        ),
        (
            "create tool --id report --name Report --tool-kind predefined-command
                --command-id agent-report --script-body-stdin",
            64,
            "script body is invalid for predefined-command",
        ),
        (
            "create instruction --id INVALID --name Invalid --prompt-stdin",
            65,
            "invalid_definition",
        ),
        (
            "create tool --id INVALID --name Invalid --tool-kind own-script --script-body-stdin",
            65,
            "invalid_definition",
        ),
    ] {
        assert_rejected(&workspace, command, code, diagnostic);
    }
    for kind in ["instructions", "tools"] {
        assert!(
            !session_home_path()
                .join("registry")
                .join(kind)
                .join("INVALID.yaml")
                .exists(),
            "{kind}"
        );
    }
}

#[test]
fn tool_argv_preserves_literal_help_flags_and_environment_tokens() {
    let workspace = empty_workspace();
    succeed_in(&workspace, "init");

    for (id, literal) in [
        ("long-help", "--help"),
        ("short-help", "-h"),
        ("environment", "$TOOL_INPUT"),
        ("legacy-literal", "--network"),
    ] {
        succeed_in(
            &workspace,
            &format!(
                "create tool --id {id} --name {id} --tool-kind predefined-command
                    --command-id agent-echo --argv {literal}"
            ),
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
                "--argv",
                "--script-body-stdin",
                "--parameter-max-length",
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

const LEGACY_TOOL_FLAGS: [&str; 12] = [
    "--max-concurrent-processes-and-threads",
    "--runtime-profile",
    "--read-only-mount",
    "--writable-mount",
    "--network",
    "--network-default",
    "--network-allow",
    "--network-kind",
    "--network-transport",
    "--network-cidr",
    "--network-port",
    "--end-network-allow",
];

#[test]
fn legacy_tool_flags_reject_before_stdin_and_never_publish() {
    let workspace = empty_workspace();
    succeed_in(&workspace, "init");
    let definition = session_home_path().join("registry/tools/legacy-tool.yaml");

    for flag in LEGACY_TOOL_FLAGS {
        assert_rejected(
            &workspace,
            &format!(
                "create tool --id legacy-tool --name LegacyTool --tool-kind own-script
                    --script-body-stdin {flag}"
            ),
            64,
            &format!(r#"unknown argument \"{flag}\""#),
        );
        assert!(!definition.exists(), "{flag} must not publish a definition");
    }
}

#[test]
fn authoring_public_diagnostics_and_exit_classes_are_stable() {
    let workspace = empty_workspace();
    succeed_in(&workspace, "init");

    assert_rejected(&workspace, "init", 65, "global_config_already_initialized");

    let tool = "create tool --id inspect --name Inspect --tool-kind predefined-command
        --command-id agent-echo";
    succeed_in(&workspace, tool);
    assert_rejected(&workspace, tool, 65, "definition_exists");

    assert_rejected(&workspace, "validate missing", 65, "invalid_reference");
    assert_rejected(&workspace, "validate --bogus", 64, "unknown argument");
    assert_rejected(&workspace, "init --bogus", 64, "unknown argument");
}

#[test]
fn malformed_typed_yaml_is_invalid_definition_and_never_published() {
    let workspace = empty_workspace();
    succeed_in(&workspace, "init");
    fs::write(workspace.join("valid-contract.yaml"), "type: string\n")
        .expect("valid contract should be written");
    fs::write(workspace.join("malformed.yaml"), "type: [\n")
        .expect("malformed YAML should be written");

    for (id, command) in [
        (
            "malformed-contract",
            "create phase --id malformed-contract --name MalformedContract
                --output-contract-file malformed.yaml",
        ),
        (
            "malformed-predicate",
            "create phase --id malformed-predicate --name MalformedPredicate
                --output-contract-file valid-contract.yaml --loop --loop-max-iterations 1
                --loop-until-file malformed.yaml --end-loop",
        ),
    ] {
        assert_rejected(&workspace, command, 65, "invalid_definition");
        assert!(
            !session_home_path()
                .join("registry/phases")
                .join(format!("{id}.yaml"))
                .exists(),
            "malformed typed input must not publish {id}"
        );
    }
}
