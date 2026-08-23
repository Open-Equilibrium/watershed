use super::valid_policy_artifact;
use crate::{PolicyArtifact, PolicyTarget, canonical_artifact_json};

#[test]
fn protected_path_grant_scope_matching_follows_the_policy_target() {
    let mut artifact = valid_policy_artifact("filesystem-tool");
    artifact.commands[0].filesystem.read_roots = vec!["Workspace/Secrets".to_owned()];
    artifact.commands[0].filesystem.write_roots.clear();
    artifact.commands[0].filesystem.protected_path_grants =
        vec!["workspace/secrets/token".to_owned()];

    artifact.target = PolicyTarget::MacosSeatbelt;
    artifact
        .validate()
        .expect("macOS protected paths use case-insensitive scope matching");

    artifact.target = PolicyTarget::LinuxLandlockSeccomp;
    artifact
        .validate()
        .expect_err("Linux protected paths use case-sensitive scope matching");
}

#[test]
fn policy_artifact_protected_path_defaults_are_order_independent() {
    let mut artifact = valid_policy_artifact("filesystem-tool");
    artifact
        .validate()
        .expect("source-order defaults are valid");

    let json = canonical_artifact_json(&artifact).expect("policy artifact canonicalizes");
    let canonical: PolicyArtifact =
        serde_json::from_str(&json).expect("canonical policy artifact deserializes");
    canonical
        .validate()
        .expect("canonical-order defaults remain valid");

    artifact.commands[0].filesystem.protected_paths.reverse();
    artifact
        .validate()
        .expect("reordered complete defaults remain valid");

    artifact.commands[0].filesystem.protected_paths[0] =
        artifact.commands[0].filesystem.protected_paths[1].clone();
    artifact
        .validate()
        .expect_err("a duplicate replacing one default must fail validation");
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
    artifact.commands[0].filesystem.protected_path_grants = vec!["workspace/secrets/**".to_owned()];

    let err = artifact
        .validate()
        .expect_err("protected path patterns must overlap declared scopes");

    assert_eq!(
        err.to_string(),
        "tool filesystem-tool protected_path_grant \"workspace/secrets/**\" must stay inside read_roots or write_roots"
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
fn policy_artifact_accepts_safe_wildcard_protected_path_grants() {
    let mut artifact = valid_policy_artifact("filesystem-tool");
    artifact.commands[0].filesystem.protected_path_grants = vec!["workspace/**".to_owned()];

    artifact
        .validate()
        .expect("a safe protected path pattern inside declared scopes is valid");
}

#[test]
fn policy_artifact_rejects_grants_that_extend_above_their_scope() {
    let mut artifact = valid_policy_artifact("filesystem-tool");
    artifact.commands[0].filesystem.read_roots = vec!["workspace/private".to_owned()];
    artifact.commands[0].filesystem.write_roots.clear();

    for grant in ["workspace/**", "workspace/*"] {
        artifact.commands[0].filesystem.protected_path_grants = vec![grant.to_owned()];
        artifact
            .validate()
            .expect_err("a protected path grant must stay inside its declared scope");
    }

    artifact.commands[0].filesystem.protected_path_grants = vec![
        "workspace/private/**".to_owned(),
        "workspace/private/.env".to_owned(),
    ];
    artifact
        .validate()
        .expect("contained protected path grants are valid");
}

#[test]
fn policy_artifact_rejects_unsafe_protected_path_grants() {
    for grant in [
        "workspace/../.env",
        "workspace/**suffix",
        "/workspace/.env",
        "C:/workspace/.env",
    ] {
        let mut artifact = valid_policy_artifact("filesystem-tool");
        artifact.commands[0].filesystem.protected_path_grants = vec![grant.to_owned()];

        let err = artifact
            .validate()
            .expect_err("protected path grants must be safe relative paths");

        assert_eq!(
            err.to_string(),
            format!(
                "tool filesystem-tool protected_path_grant {grant:?} must be a safe relative path or pattern"
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
