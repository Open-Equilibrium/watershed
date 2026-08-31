use super::valid_policy_artifact;
use crate::{MAX_FILESYSTEM_MOUNTS, PolicyArtifact};

#[test]
fn policy_artifact_accepts_exact_workspace_mounts() {
    let mut artifact = valid_policy_artifact("filesystem-tool");
    artifact.commands[0].filesystem.read_only_mounts = vec!["workspace".to_owned()];
    artifact.commands[0].filesystem.writable_mounts = vec!["workspace/out".to_owned()];

    artifact
        .validate()
        .expect("distinct exact workspace mounts are valid");
}

#[test]
fn policy_artifact_rejects_unsafe_or_ambient_mounts() {
    for mount in ["/workspace", "C:/workspace", "workspace/../out", "other"] {
        let mut artifact = valid_policy_artifact("filesystem-tool");
        artifact.commands[0].filesystem.read_only_mounts = vec![mount.to_owned()];

        let err = artifact
            .validate()
            .expect_err("mounts must be safe and workspace-relative");

        assert_eq!(
            err.to_string(),
            format!(
                "tool filesystem-tool filesystem mount {mount:?} must be workspace or a safe path below workspace"
            )
        );
    }
}

#[test]
fn policy_artifact_rejects_duplicate_and_oversized_mount_sets() {
    let mut duplicate = valid_policy_artifact("filesystem-tool");
    duplicate.commands[0].filesystem.read_only_mounts = vec!["workspace/out".to_owned()];
    duplicate.commands[0].filesystem.writable_mounts = vec!["workspace/out".to_owned()];
    let err = duplicate
        .validate()
        .expect_err("one exact mount cannot have conflicting access modes");
    assert_eq!(
        err.to_string(),
        "tool filesystem-tool filesystem mount \"workspace/out\" is declared more than once"
    );

    let mut oversized = valid_policy_artifact("filesystem-tool");
    oversized.commands[0].filesystem.read_only_mounts = (0..MAX_FILESYSTEM_MOUNTS)
        .map(|index| format!("workspace/read-{index}"))
        .collect();
    oversized.commands[0].filesystem.writable_mounts = vec!["workspace/write".to_owned()];
    let err = oversized
        .validate()
        .expect_err("the combined exact mount set must be bounded");
    assert_eq!(
        err.to_string(),
        format!(
            "tool filesystem-tool filesystem mount count {} exceeds the maximum of {MAX_FILESYSTEM_MOUNTS}",
            MAX_FILESYSTEM_MOUNTS + 1
        )
    );
}

#[test]
fn policy_artifact_rejects_legacy_filesystem_fields() {
    let mut value = serde_json::to_value(valid_policy_artifact("filesystem-tool"))
        .expect("policy artifact serializes");
    let filesystem = value["commands"][0]["filesystem"]
        .as_object_mut()
        .expect("filesystem policy is an object");
    filesystem.remove("read_only_mounts");
    filesystem.insert("read_roots".to_owned(), serde_json::json!(["workspace"]));

    let error = serde_json::from_value::<PolicyArtifact>(value)
        .expect_err("legacy filesystem fields must fail closed");
    assert!(error.to_string().contains("read_roots"), "{error}");
}
