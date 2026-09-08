use crate::runtime::{
    fs_guards::{AnchoredDir, DirectoryErrorMode, verify_protected_aliases},
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
    verify_protected_aliases(&roots, &[])
        .expect("covered aliases and overlapping roots are admitted without reading content");

    let outside = workspace.join("outside");
    fs::hard_link(&file, &outside).expect("outside alias created");
    let error = verify_protected_aliases(&roots, &[]).expect_err("outside alias rejected");
    assert!(
        error
            .to_string()
            .contains("aliases outside the protected set"),
        "{error}"
    );
    assert!(outside.exists(), "admission must not repair the layout");
    fs::remove_file(&outside).expect("test owner removes outside alias");
    verify_protected_aliases(&roots, &[]).expect("manually repaired layout is admitted");

    let installed_path = workspace.join("installed");
    fs::create_dir(&installed_path).expect("synthetic installation created");
    fs::set_permissions(&installed_path, fs::Permissions::from_mode(0o755))
        .expect("installation is not a private store");
    let installed = AnchoredDir::workspace(&installed_path).expect("installation anchored");
    let unrelated = installed.path.join("unrelated");
    fs::write(&unrelated, b"unselected binary").expect("unrelated image written");
    fs::hard_link(&unrelated, workspace.join("unrelated-alias"))
        .expect("unrelated outside alias created");
    symlink(&unrelated, installed.path.join("unrelated-link"))
        .expect("unrelated installation symlink created");

    for parent in [&installed, &roots[2]] {
        let image = parent.file("executor");
        fs::write(&image.path, b"synthetic installed image").expect("image written");
        let opened = fs::File::open(&image.path).expect("image retained");
        fs::set_permissions(&image.path, fs::Permissions::from_mode(0o000))
            .expect("image reads and execution disabled");
        verify_protected_aliases(&roots, &[(&image, &opened)])
            .expect("a selected image is admitted without reading or executing it");

        let home_alias = roots[0].file("selected-executor");
        fs::hard_link(&image.path, &home_alias.path).expect("protected home alias created");
        let repeated = image.clone();
        let references = [
            (&image, &opened),
            (&repeated, &opened),
            (&home_alias, &opened),
        ];
        for images in [&references[..1], &references[..]] {
            verify_protected_aliases(&roots, images)
                .expect("selected names and protected home aliases are counted once");
            fs::hard_link(&image.path, &outside).expect("outside image alias created");
            let error = verify_protected_aliases(&roots, images)
                .expect_err("outside image alias rejected even with repeated references");
            assert!(
                error
                    .to_string()
                    .contains("aliases outside the protected set"),
                "{error}"
            );
            assert!(outside.exists(), "admission must not repair image aliases");
            fs::remove_file(&outside).expect("test owner removes outside image alias");
            verify_protected_aliases(&roots, images)
                .expect("manually repaired image aliases are admitted");
        }

        fs::remove_file(&image.path).expect("test owner removes selected name");
        assert!(
            verify_protected_aliases(&roots, &[(&image, &opened)]).is_err(),
            "a missing selected name must fail despite its retained image and home alias"
        );
        fs::write(&image.path, b"replacement image").expect("selected name replaced");
        let replacement = fs::File::open(&image.path).expect("replacement retained");
        verify_protected_aliases(&roots, &[(&image, &replacement)])
            .expect("replacement layout has no outside aliases");
        assert!(
            verify_protected_aliases(&roots, &[(&image, &opened)]).is_err(),
            "a replaced selected name must not match the previously retained image"
        );
        fs::remove_file(&image.path).expect("test owner removes replacement");
        fs::remove_file(&home_alias.path).expect("test owner removes retained image alias");
    }

    symlink(&file, roots[0].path.join("link")).expect("unsupported inventory object created");
    let error = verify_protected_aliases(&roots, &[]).expect_err("symlink inventory rejected");
    assert!(
        error.to_string().contains("unsupported file type"),
        "{error}"
    );
}
