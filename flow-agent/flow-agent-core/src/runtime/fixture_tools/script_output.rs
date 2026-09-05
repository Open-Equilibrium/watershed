use crate::runtime::{
    execution_plan::ScriptWrite,
    fs_guards::{AnchoredDir, AnchoredFile, with_anchored_replacement_temp_checked},
    types::RuntimeError,
};
#[cfg(test)]
use std::cell::RefCell;
use std::io::{self, Write};

mod target;
#[cfg(test)]
pub use target::{anchored_workspace_write_path, normalize_script_write_target};
pub use target::{
    anchored_workspace_write_path_from, ensure_anchored_writable_regular_leaf,
    validate_script_write_target,
};

#[cfg(test)]
thread_local! {
    static SCRIPT_OUTPUT_PUBLISH_OBSERVER: RefCell<Option<Box<dyn FnOnce()>>> =
        RefCell::new(None);
    static SCRIPT_OUTPUT_CLEANUP_ERRORS: RefCell<std::collections::VecDeque<io::ErrorKind>> =
        const { RefCell::new(std::collections::VecDeque::new()) };
}

#[cfg(test)]
pub fn set_script_output_publish_observer(observer: impl FnOnce() + 'static) {
    SCRIPT_OUTPUT_PUBLISH_OBSERVER.with_borrow_mut(|slot| *slot = Some(Box::new(observer)));
}

#[cfg(test)]
fn observe_script_output_publish() {
    if let Some(observer) = SCRIPT_OUTPUT_PUBLISH_OBSERVER.with_borrow_mut(Option::take) {
        observer();
    }
}

#[cfg(test)]
pub fn set_script_output_cleanup_error_once(kind: io::ErrorKind) {
    set_script_output_cleanup_errors([kind]);
}

#[cfg(test)]
pub fn set_script_output_cleanup_errors(errors: impl IntoIterator<Item = io::ErrorKind>) {
    SCRIPT_OUTPUT_CLEANUP_ERRORS.with_borrow_mut(|pending| {
        pending.clear();
        pending.extend(errors);
    });
}

fn remove_published_script_temp(path: &AnchoredFile) -> Result<(), RuntimeError> {
    #[cfg(test)]
    if let Some(kind) = SCRIPT_OUTPUT_CLEANUP_ERRORS.with_borrow_mut(|pending| pending.pop_front())
    {
        return Err(RuntimeError::Io {
            path: path.diagnostic_path().to_owned(),
            source: io::Error::new(kind, "injected own-script output cleanup failure"),
        });
    }
    path.remove()
}

pub fn write_script_output(
    workspace: &AnchoredDir,
    target: &str,
    contents: &[u8],
) -> Result<(), RuntimeError> {
    let path = anchored_workspace_write_path_from(workspace, target, true)?
        .expect("created script target parent is present");
    replace_script_output_atomically_checked(&path, contents, |_| Ok(()))
}

pub fn preflight_own_script_outputs(
    workspace: &AnchoredDir,
    write: Option<&ScriptWrite>,
) -> Result<(), RuntimeError> {
    if let Some(write) = write
        && let Some(path) = anchored_workspace_write_path_from(workspace, &write.target, false)?
    {
        ensure_script_output_target_available(&path)?;
    }
    Ok(())
}

#[cfg(test)]
pub fn replace_script_output_atomically(
    path: &AnchoredFile,
    contents: &[u8],
) -> Result<(), RuntimeError> {
    replace_script_output_atomically_checked(path, contents, |_| Ok(()))
}

fn replace_script_output_atomically_checked(
    path: &AnchoredFile,
    contents: &[u8],
    candidate_check: impl Fn(&AnchoredFile) -> Result<(), RuntimeError>,
) -> Result<(), RuntimeError> {
    ensure_script_output_target_available(path)?;
    with_anchored_replacement_temp_checked(
        path,
        Some(core_policy::DenyReasonCode::WriteDenied),
        candidate_check,
        |temp_path, mut temp_file| {
            temp_file
                .write_all(contents)
                .map_err(|source| RuntimeError::Io {
                    path: temp_path.diagnostic_path().to_owned(),
                    source,
                })?;
            // Keep the created file open through capability-relative publication. A peer with
            // write access to this exact directory can already replace the destination itself.

            ensure_script_output_target_available(path)?;
            #[cfg(test)]
            observe_script_output_publish();
            match temp_path.hard_link_to(path) {
                Ok(()) => {}
                Err(RuntimeError::Io { source, .. })
                    if source.kind() == io::ErrorKind::AlreadyExists =>
                {
                    return Err(RuntimeError::denied(
                        core_policy::DenyReasonCode::WriteDenied,
                        format!(
                            "own-script output {} already exists; existing outputs are not replaceable",
                            path.diagnostic_path().display()
                        ),
                    ));
                }
                Err(error) => return Err(error),
            }
            drop(temp_file);
            for attempt in 0..2 {
                match remove_published_script_temp(temp_path) {
                    Ok(()) => break,
                    Err(RuntimeError::Io { source, .. })
                        if source.kind() == io::ErrorKind::NotFound =>
                    {
                        break;
                    }
                    Err(_) if attempt == 0 => {}
                    Err(source) => {
                        return Err(RuntimeError::PublishedOutputCleanupFailure {
                            output: path.diagnostic_path().to_owned(),
                            temporary: temp_path.diagnostic_path().to_owned(),
                            source: Box::new(source),
                        });
                    }
                }
            }
            Ok(())
        },
    )
}

fn ensure_script_output_target_available(path: &AnchoredFile) -> Result<(), RuntimeError> {
    if ensure_anchored_writable_regular_leaf(path)? {
        return Err(RuntimeError::denied(
            core_policy::DenyReasonCode::WriteDenied,
            format!(
                "own-script output {} already exists; existing outputs are not replaceable",
                path.diagnostic_path().display()
            ),
        ));
    }
    Ok(())
}
