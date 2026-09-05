use crate::runtime::{
    fs_guards::{
        AnchoredDir, AnchoredFile, DirectoryErrorMode, ensure_not_hardlinked_open_file,
        open_anchored_real_file_for_read,
    },
    types::RuntimeError,
};
use std::io;
#[cfg(test)]
use std::path::Path;

pub fn validate_script_write_target(
    policy: &core_policy::CommandPolicy,
    target: &str,
) -> Result<String, RuntimeError> {
    let relative = normalize_script_write_target(target)?;
    let scoped = core_script::workspace_scope_path(&relative);
    if !policy
        .filesystem
        .writable_mounts
        .iter()
        .any(|root| core_script::relative_path_is_inside_scope(&scoped, root))
    {
        return Err(RuntimeError::denied(
            core_policy::DenyReasonCode::WriteDenied,
            format!("tool {} lacks write scope {scoped}", policy.tool_id),
        ));
    }
    let temp_parent_scoped = script_replacement_temp_parent_scope(&relative);
    if !policy
        .filesystem
        .writable_mounts
        .iter()
        .any(|root| core_script::relative_path_is_inside_scope(&temp_parent_scoped, root))
    {
        return Err(RuntimeError::denied(
            core_policy::DenyReasonCode::WriteDenied,
            format!(
                "tool {} lacks write scope for replacement temp under {temp_parent_scoped}",
                policy.tool_id
            ),
        ));
    }
    Ok(relative)
}

pub fn script_replacement_temp_parent_scope(relative: &str) -> String {
    relative.rsplit_once('/').map_or_else(
        || core_script::workspace_scope_path(""),
        |(parent, _)| core_script::workspace_scope_path(parent),
    )
}

#[cfg(test)]
pub fn anchored_workspace_write_path(
    workspace: &Path,
    target: &str,
    create: bool,
) -> Result<Option<AnchoredFile>, RuntimeError> {
    let workspace = AnchoredDir::workspace(workspace)?;
    anchored_workspace_write_path_from(&workspace, target, create)
}

pub fn anchored_workspace_write_path_from(
    workspace: &AnchoredDir,
    target: &str,
    create: bool,
) -> Result<Option<AnchoredFile>, RuntimeError> {
    let mut parts = target.split('/').peekable();
    let mut parent = workspace.clone();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return Ok(Some(parent.file(part)));
        }
        let Some(child) = parent.child(part, create, DirectoryErrorMode::ScriptWrite)? else {
            return Ok(None);
        };
        parent = child;
    }
    Err(RuntimeError::Protocol(
        "own-script write target must name a file".to_owned(),
    ))
}

pub fn normalize_script_write_target(target: &str) -> Result<String, RuntimeError> {
    // WHY: script write targets use one shared slash-only path policy across parser,
    // policy and runtime checks.
    if target.is_empty()
        || target.starts_with('/')
        || target.contains(':')
        || target.contains('\\')
        || target.contains('$')
        || target.contains('*')
        || target.contains('?')
    {
        return Err(RuntimeError::Protocol(format!(
            "own-script write target {target:?} must be a literal workspace-relative path"
        )));
    }
    if core_script::relative_path_has_windows_alias(target) {
        return Err(RuntimeError::Protocol(format!(
            "own-script write target {target:?} must not use a Windows path alias"
        )));
    }
    core_script::normalize_safe_relative_path(target).ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "own-script write target {target:?} must stay inside the workspace"
        ))
    })
}

pub fn ensure_anchored_writable_regular_leaf(path: &AnchoredFile) -> Result<bool, RuntimeError> {
    match path.metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(RuntimeError::denied(
            core_policy::DenyReasonCode::SymlinkEscapeDenied,
            format!(
                "{} must not be a symlink or reparse point",
                path.diagnostic_path().display()
            ),
        )),
        Ok(metadata) if metadata.is_file() => {
            let (file, metadata) = open_anchored_real_file_for_read(path)?;
            if let Err(error) =
                ensure_not_hardlinked_open_file(path.diagnostic_path(), &file, &metadata)
            {
                return match error {
                    RuntimeError::Protocol(_) => Err(RuntimeError::denied(
                        core_policy::DenyReasonCode::WriteDenied,
                        format!(
                            "{} must not be hard-linked",
                            path.diagnostic_path().display()
                        ),
                    )),
                    other => Err(other),
                };
            }
            Ok(true)
        }
        Ok(_) => Err(RuntimeError::denied(
            core_policy::DenyReasonCode::WriteDenied,
            format!("{} must be a file", path.diagnostic_path().display()),
        )),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}
