#[cfg(unix)]
use super::super::super::helpers::empty_workspace;
use super::super::super::{helpers::fixture_runtime_policy, support::assert_denied};
use crate::runtime::fixture_tools::validate_script_write_target;
#[cfg(unix)]
use crate::runtime::{
    fixture_tools::anchored_workspace_write_path, fs_guards::with_anchored_replacement_temp,
};
#[cfg(unix)]
use std::fs;

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
fn exact_write_mounts_cover_nested_and_out_of_scope_targets() {
    let (_registry, policy) = fixture_runtime_policy("hello-flow", "hello-flow");
    let command_policy = policy
        .commands
        .iter()
        .find(|command| command.tool_id == "write-summary")
        .expect("write-summary policy exists");
    assert_eq!(
        validate_script_write_target(command_policy, "out/summary.txt")
            .expect("declared write target accepted"),
        "out/summary.txt"
    );
    let mut file_scoped_policy = command_policy.clone();
    file_scoped_policy.filesystem.writable_mounts = vec!["workspace/out/summary.txt".to_owned()];
    assert_denied(
        validate_script_write_target(&file_scoped_policy, "out/summary.txt")
            .expect_err("file-scoped writes cannot reserve replacement temps"),
        core_policy::DenyReasonCode::WriteDenied,
        "replacement temp",
    );
    assert_denied(
        validate_script_write_target(command_policy, "other/summary.txt")
            .expect_err("out-of-scope write must reject"),
        core_policy::DenyReasonCode::WriteDenied,
        "lacks write scope",
    );
}
