#[cfg(test)]
use super::storage::nearest_existing_credential_ancestor;
use super::{
    auth_store_failure,
    platform::{
        default_credential_store_path, sync_credential_directory, verify_private_file,
        verify_private_parent,
    },
    storage::{
        StoreLock, ensure_parent, recover_abandoned_stages, replace_atomically,
        validate_durable_ancestor,
    },
    store_io,
};
#[cfg(any(unix, windows))]
use super::{
    platform::verify_private_open_file,
    storage::{recover_abandoned_stages_anchored, replace_atomically_anchored},
};
#[cfg(any(unix, windows))]
use crate::runtime::fs_guards::{
    AnchoredDir, DirectoryErrorMode, open_anchored_file_for_read, sync_anchored_directory,
};
use crate::runtime::{
    oauth_credential::{CredentialRecord, validate_credential_record},
    openai_codex::OPENAI_CODEX_PROVIDER_ID,
    types::RuntimeError,
};
use serde::{Deserialize, Serialize};
#[cfg(any(unix, windows))]
use std::{ffi::OsStr, path::Component, sync::OnceLock};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

pub(crate) const CREDENTIAL_STORE_MAX_BYTES: u64 = 256 * 1024;
const REFRESH_WINDOW_MILLISECONDS: u64 = 5 * 60 * 1_000;

fn decode_credential_document(bytes: &[u8]) -> Result<Option<CredentialRecord>, RuntimeError> {
    if bytes.len() as u64 > CREDENTIAL_STORE_MAX_BYTES {
        return Err(auth_store_failure());
    }
    let document: CredentialDocument =
        serde_json::from_slice(bytes).map_err(|_| auth_store_failure())?;
    if let Some(credential) = &document.openai_codex {
        validate_credential_record(credential)?;
    }
    Ok(document.openai_codex)
}

fn encode_credential_document(
    credential: Option<CredentialRecord>,
) -> Result<Vec<u8>, RuntimeError> {
    let bytes = serde_json::to_vec(&CredentialDocument {
        openai_codex: credential,
    })
    .map_err(|_| auth_store_failure())?;
    if bytes.len() as u64 >= CREDENTIAL_STORE_MAX_BYTES {
        return Err(auth_store_failure());
    }
    Ok(bytes)
}

pub(crate) struct CredentialStore {
    path: PathBuf,
    protect_parent: bool,
    durable_ancestor: PathBuf,
    #[cfg(any(unix, windows))]
    protected_parent: OnceLock<AnchoredDir>,
}

impl CredentialStore {
    #[cfg(test)]
    pub(crate) fn at(path: PathBuf) -> Self {
        let durable_ancestor = nearest_existing_credential_ancestor(&path);
        Self {
            path,
            protect_parent: false,
            durable_ancestor,
            #[cfg(any(unix, windows))]
            protected_parent: OnceLock::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn protected_at(path: PathBuf) -> Self {
        let durable_ancestor = nearest_existing_credential_ancestor(&path);
        Self {
            path,
            protect_parent: true,
            durable_ancestor,
            #[cfg(any(unix, windows))]
            protected_parent: OnceLock::new(),
        }
    }

    pub(crate) fn platform_default() -> Result<Self, RuntimeError> {
        let path = default_credential_store_path()?;
        #[cfg(windows)]
        let durable_ancestor = path
            .parent()
            .and_then(Path::parent)
            .ok_or_else(auth_store_failure)?
            .to_owned();
        #[cfg(not(windows))]
        let durable_ancestor = path
            .ancestors()
            .last()
            .ok_or_else(auth_store_failure)?
            .to_owned();
        validate_durable_ancestor(
            path.parent().ok_or_else(auth_store_failure)?,
            &durable_ancestor,
        )?;
        Ok(Self {
            path,
            protect_parent: true,
            durable_ancestor,
            #[cfg(any(unix, windows))]
            protected_parent: OnceLock::new(),
        })
    }

    pub(crate) fn read(&self) -> Result<Option<CredentialRecord>, RuntimeError> {
        #[cfg(any(unix, windows))]
        if self.protect_parent {
            let Some(parent) = self.open_protected_parent(false)? else {
                return Ok(None);
            };
            return self.read_anchored(&parent);
        }

        let Some(parent) = self.path.parent() else {
            return Err(auth_store_failure());
        };
        match fs::symlink_metadata(parent) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(store_io(parent, error)),
        }
        if self.protect_parent {
            verify_private_parent(parent)?;
        }
        match fs::symlink_metadata(&self.path) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(store_io(&self.path, error)),
        }
        if self.protect_parent {
            verify_private_file(&self.path)?;
        }
        let mut file = File::open(&self.path).map_err(|error| store_io(&self.path, error))?;
        if file
            .metadata()
            .map_err(|error| store_io(&self.path, error))?
            .len()
            > CREDENTIAL_STORE_MAX_BYTES
        {
            return Err(auth_store_failure());
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(CREDENTIAL_STORE_MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| store_io(&self.path, error))?;
        decode_credential_document(&bytes)
    }

    pub(crate) fn replace_with_clock(
        &self,
        credential: &CredentialRecord,
        now: impl FnMut() -> Duration,
        wait: impl FnMut(Duration),
    ) -> Result<(), RuntimeError> {
        validate_credential_record(credential)?;
        self.mutate_with_clock(now, wait, |_| Some(credential.clone()))
    }

    pub(crate) fn logout_with_clock(
        &self,
        now: impl FnMut() -> Duration,
        wait: impl FnMut(Duration),
    ) -> Result<bool, RuntimeError> {
        let mut removed = false;
        self.mutate_with_clock(now, wait, |current| {
            removed = current.is_some();
            None
        })?;
        Ok(removed)
    }

    pub(crate) fn resolve_with_clock(
        &self,
        now_epoch_milliseconds: u64,
        mut refresh: impl FnMut(&CredentialRecord) -> Result<CredentialRecord, RuntimeError>,
        lock_now: impl FnMut() -> Duration,
        wait: impl FnMut(Duration),
    ) -> Result<CredentialRecord, RuntimeError> {
        let current = self.read()?.ok_or_else(authentication_required)?;
        if !needs_refresh(&current, now_epoch_milliseconds) {
            return Ok(current);
        }
        #[cfg(any(unix, windows))]
        if self.protect_parent {
            let parent = self
                .open_protected_parent(true)?
                .ok_or_else(auth_store_failure)?;
            let lock_path = parent.file(self.lock_leaf()?);
            let _lock = StoreLock::acquire_anchored(&lock_path, lock_now, wait)?;
            let current = self
                .read_anchored_and_finalize(&parent)?
                .ok_or_else(authentication_required)?;
            if !needs_refresh(&current, now_epoch_milliseconds) {
                return Ok(current);
            }
            let replacement = refresh(&current)?;
            validate_credential_record(&replacement)?;
            self.write_document_anchored(&parent, Some(replacement.clone()))?;
            return Ok(replacement);
        }
        let parent = self.path.parent().ok_or_else(auth_store_failure)?;
        ensure_parent(parent, self.protect_parent, &self.durable_ancestor)?;
        let _lock = StoreLock::acquire(self.lock_path(), self.protect_parent, lock_now, wait)?;
        let current = self
            .read_locked_and_finalize(parent)?
            .ok_or_else(authentication_required)?;
        if !needs_refresh(&current, now_epoch_milliseconds) {
            return Ok(current);
        }
        let replacement = refresh(&current)?;
        validate_credential_record(&replacement)?;
        self.write_document(Some(replacement.clone()))?;
        Ok(replacement)
    }

    pub(crate) fn replace(&self, credential: &CredentialRecord) -> Result<(), RuntimeError> {
        let started = Instant::now();
        self.replace_with_clock(credential, || started.elapsed(), thread::sleep)
    }

    pub(crate) fn logout(&self) -> Result<bool, RuntimeError> {
        let started = Instant::now();
        self.logout_with_clock(|| started.elapsed(), thread::sleep)
    }

    fn mutate_with_clock(
        &self,
        now: impl FnMut() -> Duration,
        wait: impl FnMut(Duration),
        mutation: impl FnOnce(Option<&CredentialRecord>) -> Option<CredentialRecord>,
    ) -> Result<(), RuntimeError> {
        #[cfg(any(unix, windows))]
        if self.protect_parent {
            let parent = self
                .open_protected_parent(true)?
                .ok_or_else(auth_store_failure)?;
            let lock_path = parent.file(self.lock_leaf()?);
            let _lock = StoreLock::acquire_anchored(&lock_path, now, wait)?;
            let current = self.read_anchored_and_finalize(&parent)?;
            let replacement = mutation(current.as_ref());
            if let Some(credential) = &replacement {
                validate_credential_record(credential)?;
            }
            return self.write_document_anchored(&parent, replacement);
        }
        let parent = self.path.parent().ok_or_else(auth_store_failure)?;
        ensure_parent(parent, self.protect_parent, &self.durable_ancestor)?;
        let _lock = StoreLock::acquire(self.lock_path(), self.protect_parent, now, wait)?;
        let current = self.read_locked_and_finalize(parent)?;
        let replacement = mutation(current.as_ref());
        if let Some(credential) = &replacement {
            validate_credential_record(credential)?;
        }
        self.write_document(replacement)
    }

    fn read_locked_and_finalize(
        &self,
        parent: &Path,
    ) -> Result<Option<CredentialRecord>, RuntimeError> {
        recover_abandoned_stages(&self.path)?;
        let current = self.read()?;
        sync_credential_directory(parent)?;
        Ok(current)
    }

    fn write_document(&self, replacement: Option<CredentialRecord>) -> Result<(), RuntimeError> {
        let bytes = encode_credential_document(replacement)?;
        replace_atomically(&self.path, &bytes, self.protect_parent)
    }

    fn lock_path(&self) -> PathBuf {
        self.path.with_extension("lock")
    }

    #[cfg(any(unix, windows))]
    fn open_protected_parent(&self, create: bool) -> Result<Option<AnchoredDir>, RuntimeError> {
        if let Some(parent) = self.protected_parent.get() {
            parent.validate_private()?;
            return Ok(Some(parent.clone()));
        }
        let parent_path = self.path.parent().ok_or_else(auth_store_failure)?;
        let Some(parent) = open_credential_parent(parent_path, &self.durable_ancestor, create)?
        else {
            return Ok(None);
        };
        let _ = self.protected_parent.set(parent);
        let parent = self
            .protected_parent
            .get()
            .expect("credential parent is initialized");
        parent.validate_private()?;
        Ok(Some(parent.clone()))
    }

    #[cfg(any(unix, windows))]
    fn read_anchored(
        &self,
        parent: &AnchoredDir,
    ) -> Result<Option<CredentialRecord>, RuntimeError> {
        parent.validate_private()?;
        let path = parent.file(self.credential_leaf()?);
        let (mut file, metadata) = match open_anchored_file_for_read(&path) {
            Ok(opened) => opened,
            Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        verify_private_open_file(path.diagnostic_path(), &file)?;
        if metadata.len() > CREDENTIAL_STORE_MAX_BYTES {
            return Err(auth_store_failure());
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(CREDENTIAL_STORE_MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| store_io(path.diagnostic_path(), error))?;
        decode_credential_document(&bytes)
    }

    #[cfg(any(unix, windows))]
    fn read_anchored_and_finalize(
        &self,
        parent: &AnchoredDir,
    ) -> Result<Option<CredentialRecord>, RuntimeError> {
        recover_abandoned_stages_anchored(&parent.file(self.credential_leaf()?))?;
        let current = self.read_anchored(parent)?;
        sync_anchored_directory(parent)?;
        Ok(current)
    }

    #[cfg(any(unix, windows))]
    fn write_document_anchored(
        &self,
        parent: &AnchoredDir,
        replacement: Option<CredentialRecord>,
    ) -> Result<(), RuntimeError> {
        let bytes = encode_credential_document(replacement)?;
        replace_atomically_anchored(&parent.file(self.credential_leaf()?), &bytes)
    }

    #[cfg(any(unix, windows))]
    fn credential_leaf(&self) -> Result<&OsStr, RuntimeError> {
        self.path.file_name().ok_or_else(auth_store_failure)
    }

    #[cfg(any(unix, windows))]
    fn lock_leaf(&self) -> Result<PathBuf, RuntimeError> {
        self.lock_path()
            .file_name()
            .map(PathBuf::from)
            .ok_or_else(auth_store_failure)
    }
}

#[cfg(any(unix, windows))]
fn open_credential_parent(
    path: &Path,
    durable_ancestor: &Path,
    create: bool,
) -> Result<Option<AnchoredDir>, RuntimeError> {
    validate_durable_ancestor(path, durable_ancestor)?;
    let relative = path
        .strip_prefix(durable_ancestor)
        .map_err(|_| auth_store_failure())?;
    let leaves = relative
        .components()
        .map(|component| match component {
            Component::Normal(leaf) => Ok(leaf),
            _ => Err(auth_store_failure()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut current = AnchoredDir::workspace(durable_ancestor)?;
    #[cfg(unix)]
    current.validate_not_group_or_other_writable()?;
    let mut chain = vec![current.clone()];
    if leaves.is_empty() {
        current.validate_private()?;
    }
    for (index, leaf) in leaves.iter().enumerate() {
        let final_parent = index + 1 == leaves.len();
        let next = if final_parent {
            current.private_publishable_child(leaf, create, DirectoryErrorMode::Protocol)?
        } else {
            match current.child(leaf, false, DirectoryErrorMode::Protocol)? {
                Some(child) => Some(child),
                None if create => {
                    current.private_child(leaf, true, DirectoryErrorMode::Protocol)?
                }
                None => None,
            }
        };
        let Some(next) = next else {
            return Ok(None);
        };
        #[cfg(unix)]
        if !final_parent {
            next.validate_not_group_or_other_writable()?;
        }
        current = next;
        chain.push(current.clone());
    }
    if create {
        for directory in chain.iter().rev() {
            sync_anchored_directory(directory)?;
        }
    }
    Ok(Some(current))
}

fn needs_refresh(credential: &CredentialRecord, now_epoch_milliseconds: u64) -> bool {
    credential.expires <= now_epoch_milliseconds.saturating_add(REFRESH_WINDOW_MILLISECONDS)
}

fn authentication_required() -> RuntimeError {
    RuntimeError::AuthenticationRequired(format!(
        "{OPENAI_CODEX_PROVIDER_ID} authentication is required; run flow auth login {OPENAI_CODEX_PROVIDER_ID}"
    ))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CredentialDocument {
    #[serde(
        rename = "openai-codex",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    openai_codex: Option<CredentialRecord>,
}
