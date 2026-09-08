use super::{
    config::ExecutorConfigStore,
    probe::{ProbedExecutor, probe_executor},
};
use crate::runtime::fs_guards::AnchoredDir;
use crate::runtime::types::RuntimeError;
use std::path::{Path, PathBuf};
use std::{env, fs::File};

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
#[derive(Debug)]
pub struct ExecutorSelection {
    validated: Option<ProbedExecutor>,
    path: PathBuf,
    source: ExecutorSelectionSource,
}

impl ExecutorSelection {
    pub(super) fn new(path: PathBuf, source: ExecutorSelectionSource) -> Self {
        Self {
            validated: None,
            path,
            source,
        }
    }

    fn with_probe(mut self, probed: ProbedExecutor) -> Self {
        self.validated = Some(probed);
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

    pub(crate) fn probe(&self) -> &proto::ExecutorProbeV0 {
        &self
            .validated
            .as_ref()
            .expect("resolved Executor selection carries its validated probe")
            .probe
    }

    pub(crate) fn executable(&self) -> &File {
        &self
            .validated
            .as_ref()
            .expect("resolved Executor selection carries its validated executable")
            .programs
            .selected
            .image
    }

    pub(super) fn programs(&self) -> impl Iterator<Item = &super::probe::InstalledProgram> {
        let programs = &self
            .validated
            .as_ref()
            .expect("resolved Executor selection carries its installed programs")
            .programs;
        [&programs.selected, &programs.flow]
            .into_iter()
            .chain(programs.sibling.as_ref())
    }
}

/// Performs the no-Tool-spawn readiness check and returns the effective selection.
pub fn executor_check() -> Result<ExecutorSelection, RuntimeError> {
    resolve_executor()
}

/// Validates and atomically selects an administrator-supplied absolute Executor.
pub fn configure_executor_path(path: &Path) -> Result<ExecutorSelection, RuntimeError> {
    if !path.is_absolute() {
        return Err(RuntimeError::Usage(
            "Executor path must be absolute".to_owned(),
        ));
    }
    let selection = ExecutorSelection::new(path.to_owned(), ExecutorSelectionSource::Custom);
    let roots = protected_directories(false)?;
    let probed = probe_executor(&selection, &current_flow_path()?, &roots)?;
    ExecutorConfigStore::platform_default()?.configure(path)?;
    Ok(selection.with_probe(probed))
}

/// Removes only the protected custom override and restores default sibling selection.
pub fn configure_default_executor() -> Result<bool, RuntimeError> {
    ExecutorConfigStore::platform_default()?.configure_default()
}

fn resolve_executor() -> Result<ExecutorSelection, RuntimeError> {
    resolve_executor_with_roots(&protected_directories(false)?)
}

pub(super) fn protected_directories(create: bool) -> Result<Vec<AnchoredDir>, RuntimeError> {
    let open = || {
        let home = crate::runtime::session_store::open_flow_agent_home(create)?;
        let platform = ExecutorConfigStore::platform_default()?.open_parent(create)?;
        Ok::<_, RuntimeError>([home, platform].into_iter().flatten().collect())
    };
    open().map_err(|error| {
        RuntimeError::executor(
            proto::ExecutorErrorCodeV0::Unavailable,
            format!("protected directories are unavailable: {error}"),
        )
    })
}

pub(super) fn resolve_executor_with_roots(
    roots: &[AnchoredDir],
) -> Result<ExecutorSelection, RuntimeError> {
    let store = ExecutorConfigStore::platform_default()?;
    let flow = current_flow_path()?;
    let selection = store.read()?.unwrap_or_else(|| {
        ExecutorSelection::new(
            default_executor_path(&flow),
            ExecutorSelectionSource::Default,
        )
    });
    let probed = probe_executor(&selection, &flow, roots)?;
    Ok(selection.with_probe(probed))
}

fn current_flow_path() -> Result<PathBuf, RuntimeError> {
    env::current_exe().map_err(|source| RuntimeError::Io {
        path: PathBuf::from("<current executable>"),
        source,
    })
}

pub(crate) fn default_executor_path(flow: &Path) -> PathBuf {
    flow.with_file_name("flow-executor")
}

#[cfg(test)]
mod tests {
    use super::{ExecutorSelection, ExecutorSelectionSource};
    use std::{fs, io::Read as _, os::unix::fs::PermissionsExt as _};

    #[test]
    fn validated_executor_identity_survives_path_replacement() {
        let root = crate::tests::empty_workspace();
        let path = root.join("flow-executor");
        let flow = root.join("flow");
        let custom = root.join("custom-executor");
        for image in [&path, &flow, &custom] {
            fs::write(image, b"validated").expect("candidate is staged");
            fs::set_permissions(image, fs::Permissions::from_mode(0o700))
                .expect("synthetic program is executable and private");
        }
        let selection = ExecutorSelection::new(custom.clone(), ExecutorSelectionSource::Custom);
        let programs = super::super::probe::open_validated_executable(&selection, &flow)
            .expect("installed programs are retained");
        let probe = proto::parse_executor_probe_v0(
            concat!(
                r#"{"backend":"bubblewrap-seccomp","backend_version":"test","executor":"flow-executor","executor_version":"0.0.0","platform":"ubuntu-24.04-x86_64","protocol_versions":["0"],"ready":true,"schema":"flow-executor-probe-v0","supported_policy_features":["flow-owned-write-protection"]}"#,
                "\n"
            )
            .as_bytes(),
        )
        .expect("test probe is valid");
        let selection = selection.with_probe(super::ProbedExecutor { probe, programs });
        let retained_programs = &selection
            .validated
            .as_ref()
            .expect("selection is ready")
            .programs;
        let sibling = retained_programs
            .sibling
            .as_ref()
            .expect("present sibling is retained");
        for (index, (image, retained)) in [
            (&custom, selection.executable()),
            (&flow, &retained_programs.flow.image),
            (&path, &sibling.image),
        ]
        .into_iter()
        .enumerate()
        {
            fs::rename(image, root.join(format!("validated-{index}")))
                .expect("validated inode is retained");
            fs::write(image, b"replacement").expect("path is replaced");
            let mut bytes = Vec::new();
            retained
                .try_clone()
                .expect("validated descriptor duplicates")
                .read_to_end(&mut bytes)
                .expect("validated descriptor reads");
            assert_eq!(bytes, b"validated");
            assert_eq!(fs::read(image).expect("replacement reads"), b"replacement");
        }
    }
}
