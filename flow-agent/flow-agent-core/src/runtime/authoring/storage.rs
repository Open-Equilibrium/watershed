pub(in crate::runtime::authoring) use crate::runtime::workspace_text::open_relative_directory;
use crate::runtime::{
    fs_guards::{
        AnchoredDir, DirectoryErrorMode, anchored_file_identity,
        ensure_anchored_new_leaf_available, path_io_error, read_anchored_to_string_with_limit,
        sync_anchored_directory, verify_owned_anchored_file, with_anchored_replacement_temp,
    },
    types::RuntimeError,
    workspace_text::{normal_components, read_workspace_text_file},
};
use core_script::MAX_REGISTRY_DEFINITION_BYTES;
#[cfg(test)]
use std::cell::RefCell;
use std::{io::Write, path::Path};

#[cfg(test)]
std::thread_local! {
    static AUTHORING_POST_PUBLICATION_FAILURE: RefCell<bool> = const { RefCell::new(false) };
}

#[cfg(test)]
pub(crate) fn set_authoring_post_publication_failure() {
    AUTHORING_POST_PUBLICATION_FAILURE.with(|failure| failure.replace(true));
}

#[cfg(test)]
fn observe_authoring_post_publication() -> Result<(), RuntimeError> {
    AUTHORING_POST_PUBLICATION_FAILURE.with(|failure| {
        if failure.replace(false) {
            Err(RuntimeError::Protocol(
                "injected authoring post-publication failure".to_owned(),
            ))
        } else {
            Ok(())
        }
    })
}

/// Reads one bounded UTF-8 authoring source relative to an opened workspace.
pub fn read_authoring_file(
    workspace: impl AsRef<Path>,
    source: &str,
) -> Result<String, RuntimeError> {
    read_workspace_text_file(
        workspace.as_ref(),
        source,
        u64::try_from(MAX_REGISTRY_DEFINITION_BYTES).expect("definition limit fits u64"),
        "authoring source",
    )
}

pub(super) fn ensure_child_absent(parent: &AnchoredDir, leaf: &str) -> Result<(), RuntimeError> {
    match parent.dir.symlink_metadata(leaf) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(path_io_error(&parent.path.join(leaf), source)),
        Ok(_) => Err(RuntimeError::WorkspaceAlreadyInitialized {
            path: parent.path.join(leaf),
        }),
    }
}

pub(super) fn ensure_relative_path_absent(
    parent: &AnchoredDir,
    path: &Path,
) -> Result<(), RuntimeError> {
    let components = normal_components(path)?;
    let mut current = parent.clone();
    for (index, component) in components.iter().enumerate() {
        match current.dir.symlink_metadata(component) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(source) => return Err(path_io_error(&current.path.join(component), source)),
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(RuntimeError::Protocol(format!(
                    "{} must be a real directory",
                    current.path.join(component).display()
                )));
            }
            Ok(_) if index + 1 == components.len() => {
                return Err(RuntimeError::WorkspaceAlreadyInitialized {
                    path: current.path.join(component),
                });
            }
            Ok(_) => {
                current = current
                    .child(component, false, DirectoryErrorMode::Protocol)?
                    .expect("existing directory remains present");
            }
        }
    }
    Ok(())
}

pub(super) fn create_relative_directory(
    parent: &AnchoredDir,
    path: &Path,
) -> Result<AnchoredDir, RuntimeError> {
    let mut current = parent.clone();
    for component in normal_components(path)? {
        let child = current
            .child(component, true, DirectoryErrorMode::Protocol)?
            .expect("created directory is present");
        sync_anchored_directory(&current)?;
        current = child;
    }
    Ok(current)
}

pub(super) fn write_new_file(
    target: &crate::runtime::fs_guards::AnchoredFile,
    bytes: &[u8],
    kind: &str,
) -> Result<(), RuntimeError> {
    write_new_file_with(target, bytes, kind, |opened, bytes| {
        opened
            .write_all(bytes)
            .map_err(|source| path_io_error(target.diagnostic_path(), source))
    })
}

fn write_new_file_with(
    target: &crate::runtime::fs_guards::AnchoredFile,
    bytes: &[u8],
    kind: &str,
    write: impl FnOnce(&mut std::fs::File, &[u8]) -> Result<(), RuntimeError>,
) -> Result<(), RuntimeError> {
    with_anchored_replacement_temp(target, None, |temporary, mut opened| {
        let identity = anchored_file_identity(temporary.diagnostic_path(), &opened)?;
        write(&mut opened, bytes)?;
        opened
            .sync_all()
            .map_err(|source| path_io_error(temporary.diagnostic_path(), source))?;
        verify_owned_anchored_file(temporary, &opened, kind)?;
        let current = anchored_file_identity(temporary.diagnostic_path(), &opened)?;
        if identity != current {
            return Err(RuntimeError::Protocol(format!(
                "{} {kind} identity changed while writing",
                temporary.diagnostic_path().display()
            )));
        }
        ensure_anchored_new_leaf_available(target)?;
        temporary.hard_link_to(target)?;
        drop(opened);
        temporary
            .remove()
            .map_err(|source| RuntimeError::PublishedOutputFinalizationFailure {
                output: target.diagnostic_path().to_owned(),
                source: Box::new(source),
            })?;
        #[cfg(test)]
        observe_authoring_post_publication().map_err(|source| {
            RuntimeError::PublishedOutputFinalizationFailure {
                output: target.diagnostic_path().to_owned(),
                source: Box::new(source),
            }
        })?;
        sync_anchored_directory(&target.parent).map_err(|source| {
            RuntimeError::PublishedOutputFinalizationFailure {
                output: target.diagnostic_path().to_owned(),
                source: Box::new(source),
            }
        })?;
        let persisted = read_anchored_to_string_with_limit(
            target,
            u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        )
        .map_err(|source| RuntimeError::PublishedOutputFinalizationFailure {
            output: target.diagnostic_path().to_owned(),
            source: Box::new(source),
        })?;
        if persisted.as_bytes() != bytes {
            return Err(RuntimeError::PublishedOutputFinalizationFailure {
                output: target.diagnostic_path().to_owned(),
                source: Box::new(RuntimeError::Protocol(format!(
                    "{} {kind} did not round-trip exact bytes",
                    target.diagnostic_path().display()
                ))),
            });
        }
        Ok(())
    })
}

#[cfg(test)]
pub(crate) fn write_new_file_for_test(
    target: &crate::runtime::fs_guards::AnchoredFile,
    bytes: &[u8],
    kind: &str,
) -> Result<(), RuntimeError> {
    write_new_file_with(target, bytes, kind, |_, _| {
        Err(RuntimeError::Io {
            path: target.diagnostic_path().to_owned(),
            source: std::io::Error::other("injected authoring write failure"),
        })
    })
}
