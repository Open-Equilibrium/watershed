#[cfg(all(test, not(all(target_os = "linux", target_arch = "x86_64"))))]
use super::probe::ProbedExecutor;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use super::{
    config::ExecutorConfigStore,
    probe::{ProbedExecutor, probe_executor},
};
use crate::runtime::types::RuntimeError;
use std::path::{Path, PathBuf};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::{env, fs::File, sync::Arc};

/// Authority that selected the effective productive Executor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutorSelectionSource {
    /// Administrator-supplied protected user override.
    Custom,
    /// Administrator-owned sibling installed with the running `flow` binary.
    Default,
}

impl ExecutorSelectionSource {
    /// Returns the stable human-readable selection name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Custom => "custom",
            Self::Default => "default",
        }
    }
}

/// Absolute productive Executor selected by the administrator boundary.
#[derive(Clone, Debug)]
pub struct ExecutorSelection {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    executable: Option<Arc<File>>,
    path: PathBuf,
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    probe: Option<proto::ExecutorProbeV0>,
    source: ExecutorSelectionSource,
}

impl PartialEq for ExecutorSelection {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path && self.source == other.source && {
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            {
                self.probe == other.probe
            }
            #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
            {
                true
            }
        }
    }
}

impl Eq for ExecutorSelection {}

impl ExecutorSelection {
    #[cfg(any(all(target_os = "linux", target_arch = "x86_64"), test))]
    pub(super) fn new(path: PathBuf, source: ExecutorSelectionSource) -> Self {
        Self {
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            executable: None,
            path,
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            probe: None,
            source,
        }
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn with_probe(mut self, probed: ProbedExecutor) -> Self {
        self.executable = Some(Arc::new(probed.executable));
        self.probe = Some(probed.probe);
        self
    }

    #[cfg(all(test, not(all(target_os = "linux", target_arch = "x86_64"))))]
    fn with_probe(self, _probed: ProbedExecutor) -> Self {
        self
    }

    /// Returns the selected absolute executable path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns whether the path came from protected custom configuration or the installed default.
    pub fn source(&self) -> ExecutorSelectionSource {
        self.source
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub(crate) fn probe(&self) -> &proto::ExecutorProbeV0 {
        self.probe
            .as_ref()
            .expect("resolved Executor selection carries its validated probe")
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub(crate) fn executable(&self) -> &File {
        self.executable
            .as_deref()
            .expect("resolved Executor selection carries its validated executable")
    }
}

/// Performs the no-Tool-spawn readiness check and returns the effective selection.
pub fn executor_check() -> Result<ExecutorSelection, RuntimeError> {
    resolve_executor()
}

/// Validates and atomically selects an administrator-supplied absolute Executor.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub fn configure_executor_path(path: &Path) -> Result<ExecutorSelection, RuntimeError> {
    if !path.is_absolute() {
        return Err(RuntimeError::Usage(
            "Executor path must be absolute".to_owned(),
        ));
    }
    let selection = ExecutorSelection::new(path.to_owned(), ExecutorSelectionSource::Custom);
    let probed = probe_executor(&selection, None)?;
    ExecutorConfigStore::platform_default()?.configure(path)?;
    Ok(selection.with_probe(probed))
}

/// Rejects productive Executor configuration on unsupported platforms.
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub fn configure_executor_path(_path: &Path) -> Result<ExecutorSelection, RuntimeError> {
    unsupported_platform()
}

/// Removes only the protected custom override and restores default sibling selection.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub fn configure_default_executor() -> Result<bool, RuntimeError> {
    ExecutorConfigStore::platform_default()?.configure_default()
}

/// Rejects productive Executor configuration on unsupported platforms.
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub fn configure_default_executor() -> Result<bool, RuntimeError> {
    unsupported_platform()
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) fn resolve_executor() -> Result<ExecutorSelection, RuntimeError> {
    let store = ExecutorConfigStore::platform_default()?;
    let flow = env::current_exe().map_err(|error| RuntimeError::Io {
        path: PathBuf::from("<current executable>"),
        source: error,
    })?;
    let (selection, official_flow) = match store.read()? {
        Some(selection) => (selection, None),
        None => (
            ExecutorSelection::new(
                default_executor_path(&flow),
                ExecutorSelectionSource::Default,
            ),
            Some(flow.as_path()),
        ),
    };
    let probed = probe_executor(&selection, official_flow)?;
    Ok(selection.with_probe(probed))
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub(crate) fn resolve_executor() -> Result<ExecutorSelection, RuntimeError> {
    unsupported_platform()
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) fn default_executor_path(flow: &Path) -> PathBuf {
    flow.with_file_name("flow-executor")
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn unsupported_platform<T>() -> Result<T, RuntimeError> {
    Err(RuntimeError::executor(
        proto::ExecutorErrorCodeV0::PolicyUnsupported,
        "productive Executor support requires Ubuntu 24.04 x64",
    ))
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
    use super::{ExecutorSelection, ExecutorSelectionSource};
    use std::{
        fs::{self, File},
        io::Read as _,
    };

    #[test]
    fn validated_executor_identity_survives_path_replacement() {
        let root = crate::tests::empty_workspace();
        let path = root.join("flow-executor");
        fs::write(&path, b"validated").expect("candidate is staged");
        let executable = File::open(&path).expect("candidate opens");
        let probe = proto::parse_executor_probe_v0(
            concat!(
                r#"{"backend":"bubblewrap-seccomp","backend_version":"test","executor":"flow-executor","executor_version":"0.0.0","platform":"ubuntu-24.04-x86_64","protocol_versions":["0"],"ready":true,"runtime_mounts":[],"schema":"flow-executor-probe-v0","supported_policy_features":["process-capacity","static-self-reexec"]}"#,
                "\n"
            )
            .as_bytes(),
        )
        .expect("test probe is valid");
        let selection = ExecutorSelection::new(path.clone(), ExecutorSelectionSource::Custom)
            .with_probe(super::ProbedExecutor { probe, executable });

        fs::rename(&path, root.join("validated-executor")).expect("validated inode is retained");
        fs::write(&path, b"replacement").expect("path is replaced");
        let mut retained = selection
            .executable()
            .try_clone()
            .expect("validated descriptor duplicates");
        let mut bytes = Vec::new();
        retained
            .read_to_end(&mut bytes)
            .expect("validated descriptor reads");

        assert_eq!(bytes, b"validated");
        assert_eq!(fs::read(path).expect("replacement reads"), b"replacement");
    }
}

#[cfg(all(test, not(all(target_os = "linux", target_arch = "x86_64"))))]
mod unsupported_platform_tests {
    use super::{ExecutorSelection, ExecutorSelectionSource};
    use std::path::PathBuf;

    #[test]
    fn selection_metadata_survives_the_unsupported_platform_probe_noop() {
        let selection = ExecutorSelection::new(
            PathBuf::from("administrator-selected-executor"),
            ExecutorSelectionSource::Custom,
        )
        .with_probe(super::ProbedExecutor {});
        let same_selection = ExecutorSelection::new(
            PathBuf::from("administrator-selected-executor"),
            ExecutorSelectionSource::Custom,
        );
        let default_selection = ExecutorSelection::new(
            PathBuf::from("administrator-selected-executor"),
            ExecutorSelectionSource::Default,
        );

        assert_eq!(selection.path(), same_selection.path());
        assert_eq!(selection.source(), ExecutorSelectionSource::Custom);
        assert_eq!(selection.source().as_str(), "custom");
        assert_eq!(default_selection.source().as_str(), "default");
        assert_eq!(selection, same_selection);
        assert_ne!(selection, default_selection);
    }
}
