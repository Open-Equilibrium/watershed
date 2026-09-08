use core_policy::{
    EnvironmentDefault, PolicyArtifact, canonical_artifact_json, compile_policy_artifact,
};
use core_script::{ResolvedRegistry, parse_registry_block};
use serde_json::{Value, json};

fn invocation_policy() -> Value {
    json!({
        "commands": [{
            "allowed_parameters": [{"name": "--count", "required": true, "value_type": "integer", "min": 1, "max": 3}],
            "argv": ["literal"],
            "command_id": "agent-echo",
            "environment": {"allow": [], "default": "clear"},
            "executable": "registry:agent-echo",
            "tool_id": "echo",
            "tool_kind": "predefined-command"
        }],
        "phase_scope": [{"phase_id": "say", "tool_ids": ["echo"]}],
        "policy_version": "0",
        "runtime_limits": {"headless": true, "timeout_ms": 30000},
        "source_flow_definition_id": "root"
    })
}

#[test]
fn invocation_contract_compiles_availability_parameters_environment_and_runtime_limits() {
    let blocks = [
        "tool:\n  id: echo\n  name: Echo\n  tool_kind: predefined-command\n  command:\n    command_id: agent-echo\n    argv: [literal]\n  allowed_parameters:\n    - name: --count\n      value_type: integer\n      required: true\n      min: 1\n      max: 3\n",
        "phase:\n  id: say\n  name: Say\n  instruction_refs: []\n  tool_refs: [echo]\n  output:\n    type: string\n",
        "flow:\n  id: root\n  name: Root\n  phase_refs: [say]\n  subflow_refs: []\n",
    ].map(|source| parse_registry_block("invocation.yaml", source).expect("invocation definition parses"));
    let registry = ResolvedRegistry::from_blocks(blocks).expect("invocation registry resolves");
    let policy = compile_policy_artifact(&registry, "root").expect("invocation policy compiles");
    policy
        .validate()
        .expect("compiled invocation contract validates");
    assert_eq!(
        policy.commands[0].environment.default,
        EnvironmentDefault::Clear
    );
    let expected: PolicyArtifact =
        serde_json::from_value(invocation_policy()).expect("invocation artifact parses");
    assert_eq!(policy, expected);
    let canonical = canonical_artifact_json(&policy).expect("policy canonicalizes");
    assert_eq!(
        serde_json::from_str::<PolicyArtifact>(&canonical).expect("canonical policy parses"),
        policy
    );
}

#[test]
fn invocation_contract_rejects_legacy_policy_requirements() {
    let legacy: serde_json::Map<String, Value> =
        serde_json::from_str(include_str!("../fixtures/legacy-policy-requirements.json"))
            .expect("legacy security requirements fixture parses");
    for (field, value) in legacy {
        let mut policy = invocation_policy();
        if field == "target" {
            policy[&field] = value;
        } else {
            policy["commands"][0][&field] = value;
        }
        let error = serde_json::from_value::<PolicyArtifact>(policy)
            .expect_err("obsolete security requirements cannot be silently ignored");
        let message = error.to_string();
        assert!(
            message.contains("unknown field") && message.contains(&field),
            "{field}: {message}"
        );
    }
}
