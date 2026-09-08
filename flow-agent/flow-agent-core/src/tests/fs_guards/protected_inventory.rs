use crate::runtime::{
    fs_guards::{DirectoryErrorMode, verify_protected_directory_aliases},
    session_store::open_flow_agent_home_at,
};
use crate::tests::helpers::empty_workspace;
use std::{
    fs,
    os::unix::fs::{PermissionsExt as _, symlink},
};

#[test]
fn metadata_inventory_counts_distinct_covered_names_across_overlapping_roots() {
    let workspace = empty_workspace("protected-inventory");
    let home = open_flow_agent_home_at(&workspace.join("home"), true)
        .expect("home opens")
        .expect("home exists");
    let platform = open_flow_agent_home_at(&workspace.join("platform"), true)
        .expect("platform store opens")
        .expect("platform store exists");
    let nested = home
        .private_child("nested", true, DirectoryErrorMode::Protocol)
        .expect("nested store opens")
        .expect("nested store exists");
    let file = nested.path.join("data");
    fs::write(&file, b"synthetic unreadable content").expect("fixture written");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).expect("content reads disabled");
    fs::hard_link(&file, platform.path.join("stage")).expect("covered publication alias created");
    let roots = [home.clone(), platform, nested, home];
    verify_protected_directory_aliases(&roots)
        .expect("covered aliases and overlapping roots are admitted without reading content");

    let outside = workspace.join("outside");
    fs::hard_link(&file, &outside).expect("outside alias created");
    let error = verify_protected_directory_aliases(&roots).expect_err("outside alias rejected");
    assert!(
        error
            .to_string()
            .contains("aliases outside the protected set"),
        "{error}"
    );
    assert!(outside.exists(), "admission must not repair the layout");
    fs::remove_file(&outside).expect("test owner removes outside alias");
    verify_protected_directory_aliases(&roots).expect("manually repaired layout is admitted");

    symlink(&file, roots[0].path.join("link")).expect("unsupported inventory object created");
    let error = verify_protected_directory_aliases(&roots).expect_err("symlink inventory rejected");
    assert!(
        error.to_string().contains("unsupported file type"),
        "{error}"
    );
}
