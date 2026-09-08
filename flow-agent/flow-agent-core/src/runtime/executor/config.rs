use crate::runtime::fs_guards::{
    AnchoredDir, AnchoredFile, DirectoryErrorMode, ProtectedStateLock, ProtectedStateLockError,
    canonical_decimal, open_anchored_file_for_read, sync_anchored_directory,
    unix_access_is_private,
};
use crate::runtime::types::RuntimeError;
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _};
use cap_std::fs::{OpenOptions, OpenOptionsExt as _};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    fs,
    fs::File,
    io::{self, Read as _, Write as _},
    path::{Component, Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
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
    static READ_VALIDATED_OBSERVER: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
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

#[cfg(not(test))]
fn parent_missing_observer() {}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ExecutorConfigDocument {
    path: PathBuf,
    schema: String,
}

pub(crate) struct ExecutorConfigStore {
    path: PathBuf,
    retained_parent: OnceLock<AnchoredDir>,
}

impl ExecutorConfigStore {
    #[cfg(test)]
    pub(crate) fn at(path: PathBuf) -> Self {
        Self {
            path,
            retained_parent: OnceLock::new(),
        }
    }

    pub(crate) fn platform_default() -> Result<Self, RuntimeError> {
        Ok(Self {
            path: crate::runtime::credential_store::default_credential_store_path()?
                .with_file_name("executor.json"),
            retained_parent: OnceLock::new(),
        })
    }

    pub(crate) fn read(&self) -> Result<Option<ExecutorSelection>, RuntimeError> {
        let Some(parent) = self.open_parent(false)? else {
            return Ok(None);
        };
        let path = self.anchored_path(&parent)?;
        let (file, metadata) = match open_anchored_file_for_read(&path) {
            Ok(opened) => opened,
            Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => {
                return Err(config_guard_error(
                    error,
                    "protected Executor configuration is unsafe",
                ));
            }
        };
        verify_open_file(&metadata)?;
        if metadata.len() > EXECUTOR_CONFIG_MAX_BYTES {
            return Err(config_failure(
                "protected Executor configuration is oversized",
            ));
        }
        #[cfg(test)]
        if let Some(observer) = READ_VALIDATED_OBSERVER.with_borrow_mut(Option::take) {
            observer();
        }
        let mut bytes = Vec::new();
        file.take(EXECUTOR_CONFIG_MAX_BYTES + 1)
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
        let _lock = acquire_config_lock(&parent.file(".executor.lock"))?;
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
        let _lock = acquire_config_lock(&parent.file(".executor.lock"))?;
        recover_abandoned_stages(&parent)?;
        let path = self.anchored_path(&parent)?;
        match verify_anchored_file(&path) {
            Ok(()) => {}
            Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
                return Ok(false);
            }
            Err(error) => return Err(error),
        }
        path.remove()?;
        sync_anchored_directory(&parent)?;
        Ok(true)
    }

    fn parent(&self) -> Result<&Path, RuntimeError> {
        self.path
            .parent()
            .ok_or_else(|| config_failure("Executor configuration has no parent"))
    }

    fn anchored_path(&self, parent: &AnchoredDir) -> Result<AnchoredFile, RuntimeError> {
        let leaf = self
            .path
            .file_name()
            .ok_or_else(|| config_failure("Executor configuration has no file name"))?;
        Ok(parent.file(leaf))
    }

    fn ensure_parent(&self) -> Result<AnchoredDir, RuntimeError> {
        self.open_parent(true)?
            .ok_or_else(|| config_failure("Executor configuration parent is unavailable"))
    }

    fn open_parent(&self, create: bool) -> Result<Option<AnchoredDir>, RuntimeError> {
        if self.retained_parent.get().is_none() {
            let mut components = self.parent()?.components();
            if components.next() != Some(Component::RootDir) {
                return Err(config_failure("Executor configuration parent is unsafe"));
            }
            let mut current = AnchoredDir::workspace(Path::new("/"))?;
            let mut components = components.peekable();
            while let Some(component) = components.next() {
                let Component::Normal(leaf) = component else {
                    return Err(config_failure("Executor configuration parent is unsafe"));
                };
                let next = if components.peek().is_none() {
                    match current.private_child(leaf, false, DirectoryErrorMode::Protocol) {
                        Ok(None) if create => {
                            parent_missing_observer();
                            current.private_child(leaf, true, DirectoryErrorMode::Protocol)
                        }
                        result => result,
                    }
                } else {
                    current.child(leaf, create, DirectoryErrorMode::Protocol)
                }
                .map_err(|error| {
                    config_guard_error(error, "Executor configuration parent is unsafe")
                })?;
                let Some(next) = next else {
                    return Ok(None);
                };
                current = next;
            }
            let _ = self.retained_parent.set(current);
        }
        let parent = self
            .retained_parent
            .get()
            .expect("Executor configuration parent is initialized");
        parent.validate_private().map_err(|error| {
            config_guard_error(error, "Executor configuration parent is unsafe")
        })?;
        Ok(Some(parent.clone()))
    }

    fn replace_atomically(&self, parent: &AnchoredDir, bytes: &[u8]) -> Result<(), RuntimeError> {
        let path = self.anchored_path(parent)?;
        match verify_anchored_file(&path) {
            Ok(()) => {}
            Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let stage = parent.file(format!(
            ".executor.{}.{}.tmp",
            std::process::id(),
            STAGE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let operation = (|| {
            let mut file = create_private_file(&stage)?;
            file.write_all(bytes)
                .and_then(|()| file.sync_all())
                .map_err(|error| config_io(stage.diagnostic_path(), error))?;
            stage.rename_to(&path)?;
            verify_anchored_file(&path)?;
            sync_anchored_directory(parent)
        })();
        if operation.is_err() {
            let _ = stage.remove();
        }
        operation
    }
}

fn recover_abandoned_stages(parent: &AnchoredDir) -> Result<(), RuntimeError> {
    let mut removed = false;
    for entry in parent
        .dir
        .entries()
        .map_err(|error| config_io(&parent.path, error))?
    {
        let entry = entry.map_err(|error| config_io(&parent.path, error))?;
        if !is_executor_staging_leaf(&entry.file_name()) {
            continue;
        }
        parent.file(entry.file_name()).remove()?;
        removed = true;
    }
    if removed {
        sync_anchored_directory(parent)?;
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

fn acquire_config_lock(path: &AnchoredFile) -> Result<ProtectedStateLock, RuntimeError> {
    match verify_anchored_file(path) {
        Ok(()) => {}
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .follow(FollowSymlinks::No);
    let file = path
        .open(&options)
        .map_err(|error| config_guard_error(error, "protected Executor configuration is unsafe"))?;
    let metadata = file
        .metadata()
        .map_err(|error| config_io(path.diagnostic_path(), error))?;
    verify_open_file(&metadata)?;
    use std::os::unix::fs::PermissionsExt as _;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| config_io(path.diagnostic_path(), error))?;
    let started = Instant::now();
    ProtectedStateLock::acquire(file, || started.elapsed(), thread::sleep).map_err(|error| {
        match error {
            ProtectedStateLockError::Busy => {
                config_failure("protected Executor configuration is busy")
            }
            ProtectedStateLockError::Io(error) => config_io(path.diagnostic_path(), error),
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

fn config_guard_error(error: RuntimeError, message: &str) -> RuntimeError {
    match error {
        RuntimeError::Protocol(_) => config_failure(message),
        RuntimeError::Io { ref source, .. }
            if source.raw_os_error() == Some(rustix::io::Errno::LOOP.raw_os_error())
                || source.kind() == io::ErrorKind::NotADirectory =>
        {
            config_failure(message)
        }
        error => error,
    }
}

fn create_private_file(path: &AnchoredFile) -> Result<File, RuntimeError> {
    use std::os::unix::fs::PermissionsExt as _;
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .mode(0o600)
        .follow(FollowSymlinks::No);
    let file = path
        .open(&options)
        .map_err(|error| config_guard_error(error, "protected Executor configuration is unsafe"))?;
    let metadata = file
        .metadata()
        .map_err(|error| config_io(path.diagnostic_path(), error))?;
    verify_open_file(&metadata)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| config_io(path.diagnostic_path(), error))?;
    Ok(file)
}

fn verify_anchored_file(path: &AnchoredFile) -> Result<(), RuntimeError> {
    use cap_std::fs::{MetadataExt as _, PermissionsExt as _};
    let metadata = path.metadata()?;
    verify_file_access(
        metadata.is_file(),
        metadata.uid(),
        metadata.permissions().mode(),
        metadata.nlink(),
    )
}

fn verify_open_file(metadata: &fs::Metadata) -> Result<(), RuntimeError> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    verify_file_access(
        metadata.is_file(),
        metadata.uid(),
        metadata.permissions().mode(),
        metadata.nlink(),
    )
}

fn verify_file_access(
    regular: bool,
    owner: u32,
    mode: u32,
    links: u64,
) -> Result<(), RuntimeError> {
    if !regular
        || links != 1
        || !unix_access_is_private(owner, mode, rustix::process::geteuid().as_raw())
    {
        return Err(config_failure("protected Executor configuration is unsafe"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ExecutorConfigStore;

    #[test]
    fn protected_configuration_read_of_missing_parent_has_no_side_effects() {
        let root = crate::tests::empty_workspace();
        let store = ExecutorConfigStore::at(root.join("missing/private/executor.json"));

        assert!(store.read().expect("missing override reads").is_none());
        assert!(
            std::fs::read_dir(&*root)
                .expect("fixture directory reads")
                .next()
                .is_none(),
            "reading an absent parent must not create configuration state"
        );
        let executable = root.join("executor");
        store
            .configure(&executable)
            .expect("first configuration stores after an absent read");
        assert_eq!(
            store
                .read()
                .expect("configured override reads")
                .expect("override exists")
                .path(),
            executable
        );
    }

    #[test]
    fn configuration_safety_failures_remain_executor_unavailable() {
        use std::{fs, os::unix::fs::PermissionsExt as _};

        for unsafe_parent in [false, true] {
            let root = crate::tests::empty_workspace();
            let parent = root.join("private");
            let config = parent.join("executor.json");
            let executable = root.join("executor");
            let store = ExecutorConfigStore::at(config.clone());
            store
                .configure(&executable)
                .expect("private override stores");
            let bytes = fs::read(&config).expect("override document reads");
            let (unsafe_path, mode) = if unsafe_parent {
                (&parent, 0o755)
            } else {
                (&config, 0o644)
            };
            fs::set_permissions(unsafe_path, fs::Permissions::from_mode(mode))
                .expect("fixture grants unsafe access");

            for result in [
                store.read().map(|_| ()),
                store.configure(&executable),
                store.configure_default().map(|_| ()),
            ] {
                let error = result.expect_err("unsafe configuration must be rejected");
                assert!(
                    matches!(&error, super::RuntimeError::Executor(failure)
                        if failure.code() == proto::ExecutorErrorCodeV0::Unavailable),
                    "{error}"
                );
            }
            assert_eq!(fs::read(&config).expect("unsafe override remains"), bytes);
        }
    }

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
            ExecutorConfigStore::at(peer_config)
                .configure(&peer_path)
                .expect("the peer wins first-use configuration");
        });

        let store = ExecutorConfigStore::at(config);
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

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn protected_configuration_retains_parent_replaced_after_read_validation() {
        use std::fs;

        let root = crate::tests::empty_workspace();
        let parent = root.join("private");
        let retained = root.join("retained");
        let replacement = root.join("replacement");
        let config = parent.join("executor.json");
        let original = root.join("original-executor");
        let injected = root.join("injected-executor");
        let updated = root.join("updated-executor");
        let store = ExecutorConfigStore::at(config.clone());
        store
            .configure(&original)
            .expect("original override stores");
        let replacement_config = replacement.join("executor.json");
        ExecutorConfigStore::at(replacement_config.clone())
            .configure(&injected)
            .expect("replacement namespace override stores");
        let replacement_bytes = fs::read(&replacement_config).expect("replacement document reads");
        let retained_store = ExecutorConfigStore::at(retained.join("executor.json"));

        super::READ_VALIDATED_OBSERVER.with_borrow_mut(|slot| {
            *slot = Some(Box::new(move || {
                fs::rename(&parent, &retained).expect("validated parent moves");
                fs::rename(&replacement, &parent).expect("replacement takes the checked name");
            }));
        });
        let result = store.read();
        assert!(
            super::READ_VALIDATED_OBSERVER
                .with_borrow_mut(Option::take)
                .is_none(),
            "the read must reach the deterministic parent replacement"
        );
        assert_eq!(
            result
                .expect("retained override reads")
                .expect("override exists")
                .path(),
            original,
            "a parent replaced after validation must not supply the Executor selection"
        );

        store
            .configure(&updated)
            .expect("retained override updates");
        assert_eq!(
            retained_store
                .read()
                .expect("retained namespace reads")
                .expect("updated override exists")
                .path(),
            updated
        );
        assert_eq!(
            fs::read(&config).expect("replacement document survives configure"),
            replacement_bytes,
            "configure must not overwrite the replacement namespace"
        );
        assert!(store.configure_default().expect("retained override resets"));
        assert!(store.read().expect("reset override reads").is_none());
        assert!(
            retained_store
                .read()
                .expect("retained namespace reads")
                .is_none()
        );
        assert!(!store.configure_default().expect("reset stays idempotent"));
        assert_eq!(
            fs::read(&config).expect("replacement document survives reset"),
            replacement_bytes,
            "reset must not remove the replacement namespace override"
        );
    }
}
