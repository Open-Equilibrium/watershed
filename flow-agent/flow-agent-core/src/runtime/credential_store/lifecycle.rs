#[cfg(test)]
use super::storage::nearest_existing_credential_ancestor;
use super::{
    auth_store_failure,
    platform::{default_credential_store_path, verify_private_open_file},
    storage::{
        StoreLock, recover_abandoned_stages_anchored, replace_atomically_anchored,
        validate_durable_ancestor,
    },
    store_io,
};
use crate::runtime::fs_guards::{
    AnchoredDir, DirectoryErrorMode, open_anchored_file_for_read, sync_anchored_directory,
};
use crate::runtime::{
    endpoint_credential::{
        EndpointCredential, OPENAI_COMPATIBLE_PROVIDER_ID, validate_endpoint_credential,
    },
    oauth_credential::{CredentialRecord, validate_credential_record},
    openai_codex::OPENAI_CODEX_PROVIDER_ID,
    types::RuntimeError,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    ffi::OsStr,
    io::{self, Read},
    path::{Component, Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub(crate) const CREDENTIAL_STORE_MAX_BYTES: u64 = 256 * 1024;
const REFRESH_WINDOW_MILLISECONDS: u64 = 5 * 60 * 1_000;

/// One provider slot's record, or why it does not decode or validate. Only a document that passed
/// every file-safety check is decoded, so a locked mutation may replace an undecodable slot.
type SlotResult<T> = Result<Option<T>, RuntimeError>;

/// Both slots, each decoded on its own so that a broken slot does not hide the other one.
struct DecodedDocument {
    openai_codex: SlotResult<CredentialRecord>,
    openai_compatible: SlotResult<EndpointCredential>,
}

impl DecodedDocument {
    fn absent() -> Self {
        Self {
            openai_codex: Ok(None),
            openai_compatible: Ok(None),
        }
    }

    fn undecodable() -> Self {
        Self {
            openai_codex: Err(authentication_required(OPENAI_CODEX_PROVIDER_ID)),
            openai_compatible: Err(authentication_required(OPENAI_COMPATIBLE_PROVIDER_ID)),
        }
    }

    /// The document a locked mutation writes: both decoded slots, while a slot that does not
    /// decode is dropped instead of kept.
    fn preserved(&self) -> CredentialDocument {
        CredentialDocument {
            openai_codex: self.openai_codex.as_ref().ok().and_then(Clone::clone),
            openai_compatible: self.openai_compatible.as_ref().ok().and_then(Clone::clone),
        }
    }
}

/// The one slot a locked mutation changes; the other slot is preserved.
enum Slot {
    OpenAiCodex(Option<CredentialRecord>),
    OpenAiCompatible(Option<EndpointCredential>),
}

fn decode_credential_document(bytes: &[u8]) -> DecodedDocument {
    let Ok(document) = serde_json::from_slice::<StoredDocument>(bytes) else {
        return DecodedDocument::undecodable();
    };
    DecodedDocument {
        openai_codex: decode_slot(
            document.openai_codex,
            OPENAI_CODEX_PROVIDER_ID,
            validate_credential_record,
        ),
        openai_compatible: decode_slot(
            document.openai_compatible,
            OPENAI_COMPATIBLE_PROVIDER_ID,
            validate_endpoint_credential,
        ),
    }
}

fn decode_slot<T: DeserializeOwned>(
    value: Option<serde_json::Value>,
    provider_id: &str,
    validate: impl FnOnce(&T) -> Result<(), RuntimeError>,
) -> SlotResult<T> {
    let Some(value) = value else {
        return Ok(None);
    };
    let credential: T =
        serde_json::from_value(value).map_err(|_| authentication_required(provider_id))?;
    validate(&credential).map_err(|_| authentication_required(provider_id))?;
    Ok(Some(credential))
}

fn encode_credential_document(document: &CredentialDocument) -> Result<Vec<u8>, RuntimeError> {
    let bytes = serde_json::to_vec(document).map_err(|_| auth_store_failure())?;
    if bytes.len() as u64 >= CREDENTIAL_STORE_MAX_BYTES {
        return Err(auth_store_failure());
    }
    Ok(bytes)
}

pub(crate) struct CredentialStore {
    path: PathBuf,
    durable_ancestor: PathBuf,
    protected_parent: OnceLock<ProtectedParent>,
}

/// The bound credential directory and its chain from the durable ancestor.
struct ProtectedParent {
    parent: AnchoredDir,
    chain: Vec<AnchoredDir>,
    /// A read may bind the chain before any mutation has made it durable.
    chain_synced: AtomicBool,
}

impl CredentialStore {
    #[cfg(test)]
    pub(crate) fn at(path: PathBuf) -> Self {
        let durable_ancestor = nearest_existing_credential_ancestor(&path);
        Self {
            path,
            durable_ancestor,
            protected_parent: OnceLock::new(),
        }
    }

    pub(crate) fn platform_default() -> Result<Self, RuntimeError> {
        let path = default_credential_store_path()?;
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
            durable_ancestor,
            protected_parent: OnceLock::new(),
        })
    }

    pub(crate) fn read(&self) -> Result<Option<CredentialRecord>, RuntimeError> {
        self.read_document()?
            .map_or(Ok(None), |document| document.openai_codex)
    }

    /// The endpoint record a Run uses; an absent or undecodable one requires reauthentication.
    pub(crate) fn resolve_endpoint(&self) -> Result<EndpointCredential, RuntimeError> {
        let slot = self
            .read_document()?
            .map_or(Ok(None), |document| document.openai_compatible);
        require_record(slot, OPENAI_COMPATIBLE_PROVIDER_ID)
    }

    pub(crate) fn read_endpoint(&self) -> Result<Option<EndpointCredential>, RuntimeError> {
        self.read_document()?
            .map_or(Ok(None), |document| document.openai_compatible)
    }

    fn read_document(&self) -> Result<Option<DecodedDocument>, RuntimeError> {
        let Some(parent) = self.open_protected_parent(false)? else {
            return Ok(None);
        };
        self.read_anchored(&parent).map(Some)
    }

    pub(crate) fn replace_with_clock(
        &self,
        credential: &CredentialRecord,
        now: impl FnMut() -> Duration,
        wait: impl FnMut(Duration),
    ) -> Result<(), RuntimeError> {
        validate_credential_record(credential)?;
        self.mutate_with_clock(now, wait, Slot::OpenAiCodex(Some(credential.clone())))
            .map(|_| ())
    }

    pub(crate) fn replace_endpoint_with_clock(
        &self,
        credential: &EndpointCredential,
        now: impl FnMut() -> Duration,
        wait: impl FnMut(Duration),
    ) -> Result<(), RuntimeError> {
        validate_endpoint_credential(credential)?;
        self.mutate_with_clock(now, wait, Slot::OpenAiCompatible(Some(credential.clone())))
            .map(|_| ())
    }

    pub(crate) fn logout_with_clock(
        &self,
        now: impl FnMut() -> Duration,
        wait: impl FnMut(Duration),
    ) -> Result<bool, RuntimeError> {
        self.mutate_with_clock(now, wait, Slot::OpenAiCodex(None))
    }

    pub(crate) fn logout_endpoint_with_clock(
        &self,
        now: impl FnMut() -> Duration,
        wait: impl FnMut(Duration),
    ) -> Result<bool, RuntimeError> {
        self.mutate_with_clock(now, wait, Slot::OpenAiCompatible(None))
    }

    pub(crate) fn resolve_with_clock(
        &self,
        mut now_epoch_milliseconds: impl FnMut() -> Result<u64, RuntimeError>,
        mut refresh: impl FnMut(&CredentialRecord) -> Result<CredentialRecord, RuntimeError>,
        lock_now: impl FnMut() -> Duration,
        wait: impl FnMut(Duration),
    ) -> Result<CredentialRecord, RuntimeError> {
        let Some(parent) = self.open_protected_parent(false)? else {
            return Err(authentication_required(OPENAI_CODEX_PROVIDER_ID));
        };
        let current = require_record(
            self.read_anchored(&parent)?.openai_codex,
            OPENAI_CODEX_PROVIDER_ID,
        )?;
        if !needs_refresh(&current, now_epoch_milliseconds()?) {
            return Ok(current);
        }
        let parent = self
            .open_protected_parent(true)?
            .ok_or_else(auth_store_failure)?;
        let lock_path = parent.file(self.lock_leaf()?);
        let _lock = StoreLock::acquire_anchored(&lock_path, lock_now, wait)?;
        let decoded = self.read_anchored_and_finalize(&parent)?;
        let mut document = decoded.preserved();
        let current = require_record(decoded.openai_codex, OPENAI_CODEX_PROVIDER_ID)?;
        // Waiting for the lock may have consumed the refresh window, so expiry is judged now.
        if !needs_refresh(&current, now_epoch_milliseconds()?) {
            return Ok(current);
        }
        let replacement = refresh(&current)?;
        validate_credential_record(&replacement)?;
        document.openai_codex = Some(replacement.clone());
        self.write_document_anchored(&parent, &document)?;
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

    /// Changes one slot under the lock, keeps the other, and reports whether the changed slot held a
    /// record.
    fn mutate_with_clock(
        &self,
        now: impl FnMut() -> Duration,
        wait: impl FnMut(Duration),
        slot: Slot,
    ) -> Result<bool, RuntimeError> {
        let parent = self
            .open_protected_parent(true)?
            .ok_or_else(auth_store_failure)?;
        let lock_path = parent.file(self.lock_leaf()?);
        let _lock = StoreLock::acquire_anchored(&lock_path, now, wait)?;
        let decoded = self.read_anchored_and_finalize(&parent)?;
        let mut document = decoded.preserved();
        let present = match slot {
            Slot::OpenAiCodex(replacement) => {
                if let Some(credential) = &replacement {
                    validate_credential_record(credential)?;
                }
                document.openai_codex = replacement;
                slot_present(&decoded.openai_codex)
            }
            Slot::OpenAiCompatible(replacement) => {
                if let Some(credential) = &replacement {
                    validate_endpoint_credential(credential)?;
                }
                document.openai_compatible = replacement;
                slot_present(&decoded.openai_compatible)
            }
        };
        self.write_document_anchored(&parent, &document)?;
        Ok(present)
    }

    #[cfg(test)]
    pub(crate) fn acquire_lock_for_test(
        &self,
        now: impl FnMut() -> Duration,
        wait: impl FnMut(Duration),
    ) -> Result<StoreLock, RuntimeError> {
        let parent = self
            .open_protected_parent(true)?
            .ok_or_else(auth_store_failure)?;
        StoreLock::acquire_anchored(&parent.file(self.lock_leaf()?), now, wait)
    }

    fn open_protected_parent(&self, create: bool) -> Result<Option<AnchoredDir>, RuntimeError> {
        let bound = match self.protected_parent.get() {
            Some(bound) => bound,
            None => {
                let parent_path = self.path.parent().ok_or_else(auth_store_failure)?;
                let Some(chain) =
                    open_credential_parent(parent_path, &self.durable_ancestor, create)?
                else {
                    return Ok(None);
                };
                let parent = chain
                    .last()
                    .expect("credential parent chain is nonempty")
                    .clone()
                    .with_publication();
                let _ = self.protected_parent.set(ProtectedParent {
                    parent,
                    chain,
                    chain_synced: AtomicBool::new(false),
                });
                self.protected_parent
                    .get()
                    .expect("credential parent is initialized")
            }
        };
        bound.parent.validate_private()?;
        if create && !bound.chain_synced.load(Ordering::Acquire) {
            for directory in bound.chain.iter().rev() {
                sync_anchored_directory(directory)?;
            }
            bound.chain_synced.store(true, Ordering::Release);
        }
        Ok(Some(bound.parent.clone()))
    }

    fn read_anchored(&self, parent: &AnchoredDir) -> Result<DecodedDocument, RuntimeError> {
        parent.validate_private()?;
        let path = parent.file(self.credential_leaf()?);
        let (mut file, metadata) = match open_anchored_file_for_read(&path) {
            Ok(opened) => opened,
            Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
                return Ok(DecodedDocument::absent());
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
        if bytes.len() as u64 > CREDENTIAL_STORE_MAX_BYTES {
            return Err(auth_store_failure());
        }
        Ok(decode_credential_document(&bytes))
    }

    fn read_anchored_and_finalize(
        &self,
        parent: &AnchoredDir,
    ) -> Result<DecodedDocument, RuntimeError> {
        recover_abandoned_stages_anchored(&parent.file(self.credential_leaf()?))?;
        let current = self.read_anchored(parent)?;
        sync_anchored_directory(parent)?;
        Ok(current)
    }

    fn write_document_anchored(
        &self,
        parent: &AnchoredDir,
        document: &CredentialDocument,
    ) -> Result<(), RuntimeError> {
        let bytes = encode_credential_document(document)?;
        replace_atomically_anchored(&parent.file(self.credential_leaf()?), &bytes)
    }

    fn credential_leaf(&self) -> Result<&OsStr, RuntimeError> {
        self.path.file_name().ok_or_else(auth_store_failure)
    }

    fn lock_leaf(&self) -> Result<PathBuf, RuntimeError> {
        self.path
            .with_extension("lock")
            .file_name()
            .map(PathBuf::from)
            .ok_or_else(auth_store_failure)
    }
}

fn open_credential_parent(
    path: &Path,
    durable_ancestor: &Path,
    create: bool,
) -> Result<Option<Vec<AnchoredDir>>, RuntimeError> {
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
    current.validate_not_group_or_other_writable()?;
    let mut chain = vec![current.clone()];
    if leaves.is_empty() {
        current.validate_private()?;
    }
    for (index, leaf) in leaves.iter().enumerate() {
        let final_parent = index + 1 == leaves.len();
        let next = if final_parent {
            current.private_child(leaf, create, DirectoryErrorMode::Protocol)?
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
        if !final_parent {
            next.validate_not_group_or_other_writable()?;
        }
        current = next;
        chain.push(current.clone());
    }
    Ok(Some(chain))
}

fn needs_refresh(credential: &CredentialRecord, now_epoch_milliseconds: u64) -> bool {
    credential.expires <= now_epoch_milliseconds.saturating_add(REFRESH_WINDOW_MILLISECONDS)
}

/// An absent or undecodable record fails closed and requires reauthentication.
fn require_record<T>(slot: SlotResult<T>, provider_id: &str) -> Result<T, RuntimeError> {
    slot?.ok_or_else(|| authentication_required(provider_id))
}

/// Whether a slot holds a record; one that does not decode still counts, so it is reported and replaced.
fn slot_present<T>(slot: &SlotResult<T>) -> bool {
    !matches!(slot, Ok(None))
}

fn authentication_required(provider_id: &str) -> RuntimeError {
    let login = if provider_id == OPENAI_COMPATIBLE_PROVIDER_ID {
        format!("flow auth login {provider_id} --base-url <URL> --api-key")
    } else {
        format!("flow auth login {provider_id}")
    };
    RuntimeError::AuthenticationRequired(format!(
        "{provider_id} authentication is required; run {login}"
    ))
}

/// The protected document: one slot per provider, each optional.
#[derive(Serialize)]
struct CredentialDocument {
    #[serde(rename = "openai-codex", skip_serializing_if = "Option::is_none")]
    openai_codex: Option<CredentialRecord>,
    #[serde(rename = "openai-compatible", skip_serializing_if = "Option::is_none")]
    openai_compatible: Option<EndpointCredential>,
}

/// The stored document as read: each slot kept raw until it is decoded and validated on its own.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDocument {
    #[serde(rename = "openai-codex", default)]
    openai_codex: Option<serde_json::Value>,
    #[serde(rename = "openai-compatible", default)]
    openai_compatible: Option<serde_json::Value>,
}
