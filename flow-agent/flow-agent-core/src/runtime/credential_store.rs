mod lifecycle;
mod platform;
mod storage;

use crate::runtime::types::RuntimeError;
use std::{io, path::Path};

pub(crate) use lifecycle::CredentialStore;
#[cfg(any(all(target_os = "linux", target_arch = "x86_64"), all(test, windows)))]
pub(crate) use platform::default_credential_store_path;

#[cfg(test)]
pub(crate) use lifecycle::CREDENTIAL_STORE_MAX_BYTES;
#[cfg(all(test, target_os = "macos"))]
pub(crate) use platform::{
    create_private_credential_file_for_test, macos_credential_path_has_acl_entries_for_test,
};
#[cfg(all(test, windows))]
pub(crate) use platform::{
    set_windows_credential_world_access_for_test,
    windows_credential_directory_is_current_user_only_for_test,
    windows_credential_file_is_current_user_only_for_test,
};
#[cfg(test)]
pub(crate) use storage::{
    CREDENTIAL_LOCK_DEADLINE, StoreLock, credential_staging_path_for_test,
    set_credential_protection_error_for_test,
};

fn store_io(path: &Path, source: io::Error) -> RuntimeError {
    RuntimeError::Io {
        path: path.to_owned(),
        source,
    }
}

fn auth_store_failure() -> RuntimeError {
    RuntimeError::Protocol("authentication credential store failure".to_owned())
}
