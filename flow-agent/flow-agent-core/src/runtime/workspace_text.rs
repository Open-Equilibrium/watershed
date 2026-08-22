use crate::runtime::{
    fs_guards::{
        AnchoredDir, DirectoryErrorMode, path_io_error, read_anchored_to_string_with_limit,
    },
    types::RuntimeError,
};
use std::path::{Component, Path, PathBuf};

pub(crate) fn read_workspace_text_file(
    workspace: &Path,
    source: &str,
    max_bytes: u64,
    kind: &str,
) -> Result<String, RuntimeError> {
    let relative = core_script::normalize_safe_relative_path(source)
        .map(PathBuf::from)
        .ok_or_else(|| RuntimeError::Usage(format!("{kind} must stay within the workspace")))?;
    let workspace = AnchoredDir::workspace(workspace)?;
    let (parent, leaf) = open_relative_parent(&workspace, &relative)?;
    read_anchored_to_string_with_limit(&parent.file(leaf), max_bytes)
}

pub(in crate::runtime) fn open_relative_directory(
    parent: &AnchoredDir,
    path: &Path,
) -> Result<AnchoredDir, RuntimeError> {
    let mut current = parent.clone();
    for component in normal_components(path)? {
        current = current
            .child(component, false, DirectoryErrorMode::Protocol)?
            .ok_or_else(|| {
                path_io_error(
                    &current.path.join(component),
                    std::io::Error::from(std::io::ErrorKind::NotFound),
                )
            })?;
    }
    Ok(current)
}

fn open_relative_parent<'a>(
    parent: &AnchoredDir,
    path: &'a Path,
) -> Result<(AnchoredDir, &'a std::ffi::OsStr), RuntimeError> {
    let leaf = path
        .file_name()
        .ok_or_else(|| RuntimeError::Usage("authoring source must name a file".to_owned()))?;
    let directory = path.parent().unwrap_or_else(|| Path::new(""));
    Ok((open_relative_directory(parent, directory)?, leaf))
}

pub(in crate::runtime) fn normal_components(path: &Path) -> Result<Vec<&str>, RuntimeError> {
    path.components()
        .filter_map(|component| match component {
            Component::CurDir => None,
            Component::Normal(value) => Some(value.to_str().ok_or_else(|| {
                RuntimeError::Usage("authoring paths must be valid UTF-8".to_owned())
            })),
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => Some(Err(
                RuntimeError::Usage("authoring paths must stay within the workspace".to_owned()),
            )),
        })
        .collect()
}
