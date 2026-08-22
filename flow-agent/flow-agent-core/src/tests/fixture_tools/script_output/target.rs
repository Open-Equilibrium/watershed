#[cfg(unix)]
use super::super::super::helpers::empty_workspace;
use super::super::super::{
    helpers::fixture_runtime_policy, support::assert_denied, test_support::workspace_copy,
};
use crate::runtime::{
    execution_plan::runtime_protected_path_match_mode,
    fixture_tools::{validate_script_write_target, write_script_output},
    fs_guards::{AnchoredDir, replacement_temp_path},
};
#[cfg(unix)]
use crate::runtime::{
    fixture_tools::anchored_workspace_write_path, fs_guards::with_anchored_replacement_temp,
};
use std::{fs, path::Path};

#[cfg(unix)]
#[test]
fn publish_stays_bound_to_the_opened_target_directory() {
    use std::{io::Write as _, os::unix::fs::symlink};

    let workspace = empty_workspace("script-directory-swap");
    let outside = empty_workspace("script-directory-swap-outside");
    fs::create_dir(workspace.join("out")).expect("output directory created");
    fs::write(workspace.join("out/result.txt"), "old").expect("original output written");
    fs::write(outside.join("result.txt"), "outside").expect("outside output written");
    let target = anchored_workspace_write_path(&workspace, "out/result.txt", true)
        .expect("target resolves")
        .expect("target parent exists");
    let moved_output = workspace.join("out-opened");

    with_anchored_replacement_temp(&target, None, |temp, mut file| {
        file.write_all(b"new").expect("temp output written");
        drop(file);
        fs::rename(workspace.join("out"), &moved_output).expect("output directory moved");
        symlink(&outside, workspace.join("out")).expect("replacement output symlink created");
        temp.rename_to(&target)
    })
    .expect("anchored replacement succeeds");

    assert_eq!(
        fs::read_to_string(outside.join("result.txt")).expect("outside output readable"),
        "outside"
    );
    assert_eq!(
        fs::read_to_string(moved_output.join("result.txt")).expect("output readable"),
        "new"
    );
}

#[test]
fn atomic_replacement_rejects_protected_temp_path() {
    let workspace = workspace_copy("hello-flow");
    fs::create_dir(workspace.join(".git")).expect("protected target parent created");
    let target = workspace.join(".git/allowed.txt");
    let anchored_workspace = AnchoredDir::workspace(&workspace).expect("workspace anchors");
    let (_registry, policy) = fixture_runtime_policy("hello-flow", "hello-flow");
    let mut write_policy = policy
        .commands
        .iter()
        .find(|command| command.tool_id == "write-summary")
        .expect("write-summary policy exists")
        .clone();
    write_policy.filesystem.write_roots = vec!["workspace".to_owned()];
    write_policy.filesystem.protected_path_grants = vec!["workspace/.git/allowed.txt".to_owned()];
    let temp_path = replacement_temp_path(Path::new(".git/allowed.txt"), 0)
        .expect("replacement temp path is valid");
    let match_mode = runtime_protected_path_match_mode(&policy.target);
    assert_eq!(
        validate_script_write_target(match_mode, &write_policy, ".git/allowed.txt")
            .expect("final protected target has an exact grant"),
        ".git/allowed.txt"
    );

    let err = write_script_output(
        &anchored_workspace,
        ".git/allowed.txt",
        b"new\n",
        match_mode,
        &write_policy,
    )
    .expect_err("protected replacement temp must reject before creation");

    assert_denied(
        err,
        core_policy::DenyReasonCode::ProtectedPathDenied,
        "protected path",
    );
    assert!(!workspace.join(temp_path).exists());
    assert!(!target.exists());
}

#[test]
fn scope_and_pattern_helpers_cover_grants_and_wildcards() {
    let (_registry, policy) = fixture_runtime_policy("hello-flow", "hello-flow");
    let command_policy = policy
        .commands
        .iter()
        .find(|command| command.tool_id == "write-summary")
        .expect("write-summary policy exists");
    let match_mode = runtime_protected_path_match_mode(&policy.target);
    assert_eq!(
        validate_script_write_target(match_mode, command_policy, "out/summary.txt")
            .expect("declared write target accepted"),
        "out/summary.txt"
    );
    let mut file_scoped_policy = command_policy.clone();
    file_scoped_policy.filesystem.write_roots = vec!["workspace/out/summary.txt".to_owned()];
    assert_denied(
        validate_script_write_target(match_mode, &file_scoped_policy, "out/summary.txt")
            .expect_err("file-scoped writes cannot reserve replacement temps"),
        core_policy::DenyReasonCode::WriteDenied,
        "replacement temp",
    );
    assert_denied(
        validate_script_write_target(match_mode, command_policy, "other/summary.txt")
            .expect_err("out-of-scope write must reject"),
        core_policy::DenyReasonCode::WriteDenied,
        "lacks write scope",
    );

    let mut broad_policy = command_policy.clone();
    broad_policy.filesystem.write_roots = vec!["workspace".to_owned()];
    assert_denied(
        validate_script_write_target(match_mode, &broad_policy, ".ssh/id_rsa")
            .expect_err("ungranted protected path must reject"),
        core_policy::DenyReasonCode::ProtectedPathDenied,
        "protected path",
    );
    broad_policy.filesystem.protected_path_grants = vec!["workspace/.ssh/**".to_owned()];
    assert_eq!(
        validate_script_write_target(match_mode, &broad_policy, ".ssh/id_rsa")
            .expect("explicit protected grant accepted"),
        ".ssh/id_rsa"
    );
    broad_policy.filesystem.protected_path_grants = vec!["workspace/??.pem".to_owned()];
    assert_denied(
        validate_script_write_target(match_mode, &broad_policy, "é.pem")
            .expect_err("two-character grant must not authorize one Unicode scalar"),
        core_policy::DenyReasonCode::ProtectedPathDenied,
        "protected path",
    );
}
