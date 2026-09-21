use super::super::super::helpers::empty_workspace;
use crate::runtime::fixture_tools::compile_own_script_operations;
use crate::runtime::{
    fixture_tools::anchored_workspace_write_path, fs_guards::with_anchored_replacement_temp,
};
use std::fs;

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
        temp.rename_to(&target.leaf)
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
fn fixture_write_targets_accept_workspace_relative_outputs() {
    for target in ["out/summary.txt", "other/summary.txt", "summary.txt"] {
        let write = compile_own_script_operations(&format!("printf 'hello\\n' > {target}"))
            .expect("literal workspace-relative fixture output accepted")
            .expect("fixture plans an output");
        assert_eq!(write.target, target);
        assert_eq!(write.contents, b"hello\n");
    }
}
