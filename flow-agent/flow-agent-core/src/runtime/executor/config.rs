use crate::runtime::fs_guards::{
    ProtectedStateLock, ProtectedStateLockError, canonical_decimal, sync_directory,
};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use crate::runtime::fs_guards::{unix_access_is_private, validate_unix_private_directory_metadata};
use crate::runtime::types::RuntimeError;
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    fs,
    fs::{File, OpenOptions},
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Instant,
};

use super::{ExecutorSelection, ExecutorSelectionSource};

const EXECUTOR_CONFIG_SCHEMA: &str = "flow-executor-selection-v0";
pub(crate) const EXECUTOR_CONFIG_MAX_BYTES: u64 = 16 * 1024;
static STAGE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
thread_local! {
    static PARENT_MISSING_OBSERVER: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
fn set_parent_missing_observer(observer: impl FnOnce() + 'static) {
    PARENT_MISSING_OBSERVER.with_borrow_mut(|slot| *slot = Some(Box::new(observer)));
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
fn parent_missing_observer() {
    if let Some(observer) = PARENT_MISSING_OBSERVER.with_borrow_mut(Option::take) {
        observer();
    }
}

#[cfg(not(all(test, target_os = "linux", target_arch = "x86_64")))]
fn parent_missing_observer() {}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ExecutorConfigDocument {
    path: PathBuf,
    schema: String,
}

pub(crate) struct ExecutorConfigStore {
    path: PathBuf,
    protect: bool,
}

impl ExecutorConfigStore {
    #[cfg(test)]
    pub(crate) fn at(path: PathBuf) -> Self {
        Self {
            path,
            protect: false,
        }
    }

    #[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
    pub(crate) fn protected_at(path: PathBuf) -> Self {
        Self {
            path,
            protect: true,
        }
    }

    pub(crate) fn platform_default() -> Result<Self, RuntimeError> {
        Ok(Self {
            path: crate::runtime::credential_store::default_credential_store_path()?
                .with_file_name("executor.json"),
            protect: true,
        })
    }

    pub(crate) fn read(&self) -> Result<Option<ExecutorSelection>, RuntimeError> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(config_io(&self.path, error)),
        };
        verify_regular_unlinked(&metadata)?;
        if self.protect {
            verify_private_file(&self.path, &metadata)?;
            verify_private_parent(self.parent()?)?;
        }
        if metadata.len() > EXECUTOR_CONFIG_MAX_BYTES {
            return Err(config_failure(
                "protected Executor configuration is oversized",
            ));
        }
        let mut bytes = Vec::new();
        File::open(&self.path)
            .map_err(|error| config_io(&self.path, error))?
            .take(EXECUTOR_CONFIG_MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| config_io(&self.path, error))?;
        if bytes.len() as u64 > EXECUTOR_CONFIG_MAX_BYTES {
            return Err(config_failure(
                "protected Executor configuration is oversized",
            ));
        }
        let document: ExecutorConfigDocument = serde_json::from_slice(&bytes)
            .map_err(|_| config_failure("protected Executor configuration is invalid"))?;
        if document.schema != EXECUTOR_CONFIG_SCHEMA || !document.path.is_absolute() {
            return Err(config_failure(
                "protected Executor configuration is invalid",
            ));
        }
        Ok(Some(ExecutorSelection::new(
            document.path,
            ExecutorSelectionSource::Custom,
        )))
    }

    pub(crate) fn configure(&self, path: &Path) -> Result<(), RuntimeError> {
        if !path.is_absolute() {
            return Err(RuntimeError::Usage(
                "Executor path must be absolute".to_owned(),
            ));
        }
        let parent = self.ensure_parent()?;
        let _lock = acquire_config_lock(&parent.join(".executor.lock"), self.protect)?;
        recover_abandoned_stages(&parent)?;
        let document = ExecutorConfigDocument {
            path: path.to_owned(),
            schema: EXECUTOR_CONFIG_SCHEMA.to_owned(),
        };
        let mut bytes = serde_json::to_vec(&document)
            .map_err(|_| config_failure("Executor path cannot be represented in JSON"))?;
        bytes.push(b'\n');
        if bytes.len() as u64 > EXECUTOR_CONFIG_MAX_BYTES {
            return Err(config_failure(
                "protected Executor configuration is oversized",
            ));
        }
        self.replace_atomically(&parent, &bytes)
    }

    pub(crate) fn configure_default(&self) -> Result<bool, RuntimeError> {
        let parent = self.ensure_parent()?;
        let _lock = acquire_config_lock(&parent.join(".executor.lock"), self.protect)?;
        recover_abandoned_stages(&parent)?;
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(config_io(&self.path, error)),
        };
        verify_regular_unlinked(&metadata)?;
        if self.protect {
            verify_private_file(&self.path, &metadata)?;
        }
        fs::remove_file(&self.path).map_err(|error| config_io(&self.path, error))?;
        sync_directory(&parent)?;
        Ok(true)
    }

    fn parent(&self) -> Result<&Path, RuntimeError> {
        self.path
            .parent()
            .ok_or_else(|| config_failure("Executor configuration has no parent"))
    }

    fn ensure_parent(&self) -> Result<PathBuf, RuntimeError> {
        let parent = self.parent()?.to_owned();
        match fs::symlink_metadata(&parent) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err(config_failure("Executor configuration parent is unsafe")),
            Err(error) if error.kind() == io::ErrorKind::NotFound && self.protect => {
                let base = parent
                    .parent()
                    .ok_or_else(|| config_failure("Executor configuration base is unavailable"))?;
                fs::create_dir_all(base).map_err(|error| config_io(base, error))?;
                parent_missing_observer();
                match create_private_directory(&parent) {
                    Ok(()) => {}
                    Err(RuntimeError::Io { source, .. })
                        if source.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir_all(&parent).map_err(|error| config_io(&parent, error))?;
            }
            Err(error) => return Err(config_io(&parent, error)),
        }
        if self.protect {
            verify_private_parent(&parent)?;
        }
        Ok(parent)
    }

    fn replace_atomically(&self, parent: &Path, bytes: &[u8]) -> Result<(), RuntimeError> {
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) => {
                verify_regular_unlinked(&metadata)?;
                if self.protect {
                    verify_private_file(&self.path, &metadata)?;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(config_io(&self.path, error)),
        }
        let stage = parent.join(format!(
            ".executor.{}.{}.tmp",
            std::process::id(),
            STAGE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let operation = (|| {
            let mut file = create_private_file(&stage, self.protect)?;
            file.write_all(bytes)
                .and_then(|()| file.sync_all())
                .map_err(|error| config_io(&stage, error))?;
            fs::rename(&stage, &self.path).map_err(|error| config_io(&self.path, error))?;
            let metadata =
                fs::symlink_metadata(&self.path).map_err(|error| config_io(&self.path, error))?;
            verify_regular_unlinked(&metadata)?;
            if self.protect {
                verify_private_file(&self.path, &metadata)?;
            }
            sync_directory(parent)
        })();
        if operation.is_err() {
            let _ = fs::remove_file(&stage);
        }
        operation
    }
}

fn recover_abandoned_stages(parent: &Path) -> Result<(), RuntimeError> {
    let mut removed = false;
    for entry in fs::read_dir(parent).map_err(|error| config_io(parent, error))? {
        let entry = entry.map_err(|error| config_io(parent, error))?;
        if !is_executor_staging_leaf(&entry.file_name()) {
            continue;
        }
        let path = entry.path();
        fs::remove_file(&path).map_err(|error| config_io(&path, error))?;
        removed = true;
    }
    if removed {
        sync_directory(parent)?;
    }
    Ok(())
}

fn is_executor_staging_leaf(leaf: &OsStr) -> bool {
    let Some(value) = leaf
        .to_str()
        .and_then(|value| value.strip_prefix(".executor."))
        .and_then(|value| value.strip_suffix(".tmp"))
    else {
        return false;
    };
    let Some((pid, counter)) = value.split_once('.') else {
        return false;
    };
    canonical_decimal(pid, u32::MAX as u64) && canonical_decimal(counter, u64::MAX)
}

fn acquire_config_lock(path: &Path, protect: bool) -> Result<ProtectedStateLock, RuntimeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            verify_regular_unlinked(&metadata)?;
            if protect {
                verify_private_file(path, &metadata)?;
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(config_io(path, error)),
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(if protect { 0o600 } else { 0o666 });
    }
    let file = options.open(path).map_err(|error| config_io(path, error))?;
    #[cfg(unix)]
    if protect {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| config_io(path, error))?;
    }
    let metadata = file.metadata().map_err(|error| config_io(path, error))?;
    verify_regular_unlinked(&metadata)?;
    if protect {
        verify_private_file(path, &metadata)?;
    }
    let started = Instant::now();
    ProtectedStateLock::acquire(file, || started.elapsed(), thread::sleep).map_err(|error| {
        match error {
            ProtectedStateLockError::Busy => {
                config_failure("protected Executor configuration is busy")
            }
            ProtectedStateLockError::Io(error) => config_io(path, error),
        }
    })
}

fn config_failure(message: &str) -> RuntimeError {
    RuntimeError::executor(proto::ExecutorErrorCodeV0::Unavailable, message)
}

fn config_io(path: &Path, source: io::Error) -> RuntimeError {
    RuntimeError::Io {
        path: path.to_owned(),
        source,
    }
}

fn verify_regular_unlinked(metadata: &fs::Metadata) -> Result<(), RuntimeError> {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(config_failure("protected Executor configuration is unsafe"));
    }
    #[cfg(unix)]
    if std::os::unix::fs::MetadataExt::nlink(metadata) != 1 {
        return Err(config_failure("protected Executor configuration is unsafe"));
    }
    Ok(())
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn create_private_directory(path: &Path) -> Result<(), RuntimeError> {
    use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    builder
        .create(path)
        .map_err(|error| config_io(path, error))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| config_io(path, error))
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn create_private_directory(_path: &Path) -> Result<(), RuntimeError> {
    Err(config_failure(
        "platform configuration protection is unsupported",
    ))
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn create_private_file(path: &Path, protect: bool) -> Result<File, RuntimeError> {
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    if protect {
        options.mode(0o600);
    }
    let file = options.open(path).map_err(|error| config_io(path, error))?;
    if protect {
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| config_io(path, error))?;
    }
    Ok(file)
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn create_private_file(path: &Path, protect: bool) -> Result<File, RuntimeError> {
    if protect {
        return Err(config_failure(
            "platform configuration protection is unsupported",
        ));
    }
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| config_io(path, error))?;
    Ok(file)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn verify_private_parent(path: &Path) -> Result<(), RuntimeError> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    let metadata = fs::symlink_metadata(path).map_err(|error| config_io(path, error))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(config_failure("Executor configuration parent is unsafe"));
    }
    validate_unix_private_directory_metadata(
        path,
        metadata.uid(),
        metadata.permissions().mode(),
        rustix::process::geteuid().as_raw(),
    )
    .map_err(|_| config_failure("Executor configuration parent is unsafe"))
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn verify_private_parent(_path: &Path) -> Result<(), RuntimeError> {
    Err(config_failure(
        "platform configuration protection is unsupported",
    ))
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn verify_private_file(_path: &Path, metadata: &fs::Metadata) -> Result<(), RuntimeError> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    if !unix_access_is_private(
        metadata.uid(),
        metadata.permissions().mode(),
        rustix::process::geteuid().as_raw(),
    ) || metadata.nlink() != 1
    {
        return Err(config_failure("protected Executor configuration is unsafe"));
    }
    Ok(())
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn verify_private_file(_path: &Path, _metadata: &fs::Metadata) -> Result<(), RuntimeError> {
    Err(config_failure(
        "platform configuration protection is unsupported",
    ))
}

#[cfg(test)]
mod tests {
    use super::ExecutorConfigStore;

    #[test]
    fn executor_config_is_beside_the_canonical_credential_store() {
        let credential = crate::runtime::credential_store::default_credential_store_path()
            .expect("platform credential path resolves");
        let executor = ExecutorConfigStore::platform_default()
            .expect("platform Executor configuration path resolves");

        assert_eq!(executor.path, credential.with_file_name("executor.json"));
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn protected_configuration_settles_a_first_use_parent_race() {
        let root = crate::tests::empty_workspace();
        let config = root.join("flow-agent/executor.json");
        let outer_path = root.join("outer-executor");
        let peer_path = root.join("peer-executor");
        let peer_config = config.clone();
        super::set_parent_missing_observer(move || {
            ExecutorConfigStore::protected_at(peer_config)
                .configure(&peer_path)
                .expect("the peer wins first-use configuration");
        });

        let store = ExecutorConfigStore::protected_at(config);
        store
            .configure(&outer_path)
            .expect("the raced configuration serializes after its peer");

        assert_eq!(
            store
                .read()
                .expect("the serialized configuration reads")
                .expect("the serialized configuration exists")
                .path(),
            outer_path
        );
    }
}
