use super::valid_policy_artifact;
use crate::PolicyArtifact;

#[test]
fn policy_artifact_accepts_explicit_environment_allow_entries_regardless_of_semantics() {
    let explicit_names = [
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

    for name in explicit_names {
        policy_artifact_with_environment_allow(name)
            .validate()
            .unwrap_or_else(|error| {
                panic!("explicit environment allow entry {name} failed: {error}")
            });
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
fn policy_artifact_accepts_syntactically_valid_environment_allow() {
    policy_artifact_with_environment_allow("_A1")
        .validate()
        .expect("syntactically valid environment allow entry");
}

fn policy_artifact_with_environment_allow(name: &str) -> PolicyArtifact {
    let mut artifact = valid_policy_artifact("environment-tool");
    artifact.commands[0].environment.allow = vec![name.to_owned()];
    artifact
}
