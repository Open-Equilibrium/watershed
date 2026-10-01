use super::super::helpers::empty_workspace;
use super::super::support::run_isolated_test;
use super::support::credential;
use crate::runtime::credential_store::CredentialStore;
#[cfg(target_os = "macos")]
use crate::runtime::credential_store::{
    create_private_credential_file_for_test, macos_credential_path_has_acl_entries_for_test,
};
use std::fs;
#[cfg(target_os = "macos")]
use std::path::Path;
#[cfg(target_os = "macos")]
use std::process::Command;

#[cfg(target_os = "macos")]
fn add_macos_acl(path: &Path, entry: &str) {
    let status = Command::new("/bin/chmod")
        .args(["+a", entry])
        .arg(path)
        .status()
        .expect("macOS chmod runs");
    assert!(status.success(), "macOS extended ACL is installed");
}

#[test]
fn protected_credential_store_enforces_private_parent_and_file_modes() {
    use std::os::unix::fs::PermissionsExt as _;

    let workspace = empty_workspace("credential-store-private-modes");
    let parent = workspace.join("private");
    let path = parent.join("credentials.json");
    let store = CredentialStore::at(path.clone());
    let current = credential(900_000);
    store.replace(&current).expect("private credential stores");
    assert_eq!(
        fs::metadata(&parent)
            .expect("private parent metadata")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&path)
            .expect("private file metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("credential mode changes");
    assert!(store.read().is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .expect("credential mode restores");
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).expect("parent mode changes");
    assert!(store.read().is_err());
}

#[cfg(target_os = "macos")]
#[test]
fn protected_credential_store_rejects_an_extended_file_acl() {
    use std::os::unix::fs::PermissionsExt as _;

    let workspace = empty_workspace("credential-store-extended-file-acl");
    let path = workspace.join("private/credentials.json");
    let store = CredentialStore::at(path.clone());
    let current = credential(900_000);
    store.replace(&current).expect("private credential stores");
    add_macos_acl(&path, "everyone allow read");
    assert_eq!(
        fs::metadata(&path)
            .expect("credential metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    assert!(store.read().is_err());
    assert!(store.replace(&credential(1_000_000)).is_err());
}

#[cfg(target_os = "macos")]
#[test]
fn protected_credential_store_rejects_an_extended_parent_acl() {
    use std::os::unix::fs::PermissionsExt as _;

    let workspace = empty_workspace("credential-store-extended-parent-acl");
    let parent = workspace.join("private");
    let path = parent.join("credentials.json");
    let store = CredentialStore::at(path);
    store
        .replace(&credential(900_000))
        .expect("private credential stores");
    add_macos_acl(&parent, "everyone allow list,search");
    assert!(
        macos_credential_path_has_acl_entries_for_test(&parent)
            .expect("credential parent ACL reads")
    );
    assert_eq!(
        fs::metadata(&parent)
            .expect("credential parent metadata")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );

    assert!(store.read().is_err());
    assert!(store.replace(&credential(1_000_000)).is_err());
}

#[cfg(target_os = "macos")]
#[test]
fn protected_credential_store_removes_inherited_parent_acl_entries() {
    let workspace = empty_workspace("credential-store-inherited-parent-acl");
    add_macos_acl(
        &workspace,
        "everyone allow list,search,file_inherit,directory_inherit",
    );
    let inherited_control = workspace.join("inherited-control");
    fs::create_dir(&inherited_control).expect("inherited ACL control directory creates");
    assert!(
        macos_credential_path_has_acl_entries_for_test(&inherited_control)
            .expect("inherited control ACL reads")
    );
    fs::remove_dir(&inherited_control).expect("inherited ACL control directory removes");
    let parent = workspace.join("private");
    let path = parent.join("credentials.json");

    CredentialStore::at(path)
        .replace(&credential(900_000))
        .expect("private credential removes its inherited parent ACL");

    assert!(
        !macos_credential_path_has_acl_entries_for_test(&parent)
            .expect("credential parent ACL reads")
    );
}

#[cfg(target_os = "macos")]
#[test]
fn protected_credential_store_removes_inherited_file_acl_entries() {
    use std::os::unix::fs::PermissionsExt as _;

    let workspace = empty_workspace("credential-store-inherited-file-acl");
    let parent = workspace.join("private");
    fs::create_dir(&parent).expect("private parent creates");
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700))
        .expect("private parent mode applies");
    add_macos_acl(&parent, "everyone allow read,file_inherit");
    let path = parent.join("credentials.json");

    create_private_credential_file_for_test(&path)
        .expect("private credential removes its inherited ACL");

    assert!(!macos_credential_path_has_acl_entries_for_test(&path).expect("credential ACL reads"));
}

#[test]
fn protected_credential_store_normalizes_a_restrictive_creation_umask() {
    const CHILD_ENV: &str = "FLOW_AGENT_TEST_RESTRICTIVE_CREDENTIAL_UMASK";
    if run_isolated_test(CHILD_ENV) {
        return;
    }

    use rustix::{fs::Mode, process::umask};
    use std::os::unix::fs::PermissionsExt as _;

    struct UmaskGuard(Mode);

    impl Drop for UmaskGuard {
        fn drop(&mut self) {
            umask(self.0);
        }
    }

    let workspace = empty_workspace("credential-store-restrictive-umask");
    let existing_parent = workspace.join("existing");
    let existing_path = existing_parent.join("credentials.json");
    let existing = CredentialStore::at(existing_path.clone());
    existing
        .replace(&credential(300_000))
        .expect("initial private credential stores");

    let _guard = UmaskGuard(umask(Mode::from_bits_retain(0o200)));
    let replacement = credential(900_000);
    existing
        .replace(&replacement)
        .expect("replacement remains readable under a restrictive umask");
    assert_eq!(
        existing.read().expect("replacement reads"),
        Some(replacement)
    );
    assert_eq!(
        fs::metadata(&existing_path)
            .expect("replacement metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    let fresh_parent = workspace.join("fresh");
    let fresh_path = fresh_parent.join("credentials.json");
    CredentialStore::at(fresh_path.clone())
        .replace(&credential(1_000_000))
        .expect("fresh private credential stores under a restrictive umask");
    assert_eq!(
        fs::metadata(&fresh_parent)
            .expect("fresh private parent metadata")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&fresh_path)
            .expect("fresh private credential metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn platform_credential_store_rejects_an_untrusted_writable_configuration_ancestor() {
    const CHILD_ENV: &str = "FLOW_AGENT_TEST_UNTRUSTED_CREDENTIAL_ANCESTOR";
    if run_isolated_test(CHILD_ENV) {
        return;
    }

    use std::os::unix::fs::PermissionsExt as _;

    let workspace = empty_workspace("credential-store-untrusted-ancestor");
    let untrusted = workspace.join("untrusted");
    fs::create_dir(&untrusted).expect("untrusted ancestor creates");
    fs::set_permissions(&untrusted, fs::Permissions::from_mode(0o777))
        .expect("untrusted ancestor becomes writable");
    #[cfg(target_os = "macos")]
    let configuration = {
        unsafe { std::env::set_var("HOME", &untrusted) };
        untrusted.join("Library").join("Application Support")
    };
    #[cfg(not(target_os = "macos"))]
    let configuration = untrusted.join("configuration");
    #[cfg(not(target_os = "macos"))]
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &configuration)
    };

    let store = CredentialStore::platform_default().expect("credential path resolves");
    assert!(store.replace(&credential(900_000)).is_err());
    assert!(!configuration.join("flow-agent").exists());
}

#[test]
fn protected_credential_store_remains_bound_to_its_opened_private_parent() {
    let workspace = empty_workspace("credential-store-retained-parent");
    let parent = workspace.join("private");
    let retained = workspace.join("retained");
    let path = parent.join("credentials.json");
    let original = credential(900_000);
    let injected = credential(1_000_000);
    let updated = credential(1_100_000);
    let store = CredentialStore::at(path.clone());

    store
        .replace(&original)
        .expect("original credential stores");
    fs::rename(&parent, &retained).expect("private parent moves");
    CredentialStore::at(path.clone())
        .replace(&injected)
        .expect("replacement namespace credential stores");

    assert_eq!(
        store.read().expect("retained credential reads"),
        Some(original)
    );
    store
        .replace(&updated)
        .expect("retained credential updates");
    assert_eq!(
        store.read().expect("updated retained credential reads"),
        Some(updated.clone())
    );
    assert_eq!(
        CredentialStore::at(path)
            .read()
            .expect("replacement namespace credential reads"),
        Some(injected)
    );
    assert_eq!(
        CredentialStore::at(retained.join("credentials.json"))
            .read()
            .expect("retained namespace credential reads"),
        Some(updated)
    );
}

#[test]
fn protected_credential_store_creates_a_missing_configuration_base() {
    let workspace = empty_workspace("credential-store-missing-configuration-base");
    let base = workspace.join("configuration");
    let parent = base.join("flow-agent");
    let path = parent.join("credentials.json");
    let store = CredentialStore::at(path.clone());

    store
        .replace(&credential(900_000))
        .expect("a fresh configuration base is created");

    assert!(base.is_dir());
    assert!(parent.is_dir());
    assert!(path.is_file());
}
