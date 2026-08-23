#[cfg(windows)]
use crate::runtime::fs_guards::path_io_error;
use crate::runtime::{
    fs_guards::{
        AnchoredDir, AnchoredFile, DirectoryErrorMode, ensure_not_hardlinked_open_file,
        open_anchored_real_file_for_read,
    },
    types::RuntimeError,
};
#[cfg(windows)]
use cap_fs_ext::MetadataExt as _;
use core_policy::{ProtectedPathMatchMode, protected_path_pattern_matches};
#[cfg(any(test, windows))]
use std::path::Path;
use std::{io, path::PathBuf};

pub fn validate_script_write_target(
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
    target: &str,
) -> Result<String, RuntimeError> {
    let relative = normalize_script_write_target(target)?;
    let scoped = core_script::workspace_scope_path(&relative);
    if !policy
        .filesystem
        .write_roots
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
        .write_roots
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
    ensure_script_target_not_protected(protected_path_match_mode, policy, &scoped)?;
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

pub fn ensure_script_target_not_protected(
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
    scoped_target: &str,
) -> Result<(), RuntimeError> {
    if !policy.filesystem.protected_paths.iter().any(|pattern| {
        protected_path_pattern_matches(protected_path_match_mode, pattern, scoped_target)
    }) {
        return Ok(());
    }

    if policy
        .filesystem
        .protected_path_grants
        .iter()
        .any(|pattern| {
            protected_path_pattern_matches(protected_path_match_mode, pattern, scoped_target)
        })
    {
        return Ok(());
    }

    Err(RuntimeError::denied(
        core_policy::DenyReasonCode::ProtectedPathDenied,
        format!(
            "tool {} cannot write protected path {scoped_target}",
            policy.tool_id
        ),
    ))
}

pub fn ensure_resolved_anchored_script_target_not_protected(
    workspace: &AnchoredDir,
    target: &str,
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
) -> Result<(), RuntimeError> {
    let resolved_target = resolved_anchored_workspace_scoped_target(workspace, target)?;
    ensure_script_target_not_protected(protected_path_match_mode, policy, &resolved_target)
}

pub fn resolved_anchored_workspace_scoped_target(
    workspace: &AnchoredDir,
    target: &str,
) -> Result<String, RuntimeError> {
    let mut resolved = PathBuf::new();
    let mut unresolved_suffix = false;
    let mut components = target.split('/').peekable();
    while let Some(component) = components.next() {
        if unresolved_suffix {
            resolved.push(component);
            continue;
        }
        let candidate = resolved.join(component);
        match workspace.dir.symlink_metadata(&candidate) {
            Ok(_) => {
                resolved = workspace.dir.canonicalize(&candidate).map_err(|source| {
                    RuntimeError::denied(
                        core_policy::DenyReasonCode::SymlinkEscapeDenied,
                        format!(
                            "own-script write target {target:?} cannot be resolved within {} without following a symlink or reparse point: {source}",
                            workspace.path.display()
                        ),
                    )
                })?;
                #[cfg(windows)]
                {
                    resolved = windows_long_anchored_relative_path(workspace, &resolved)?;
                }
                if components.peek().is_some() {
                    let metadata =
                        workspace
                            .dir
                            .metadata(&resolved)
                            .map_err(|source| RuntimeError::Io {
                                path: workspace.path.join(&resolved),
                                source,
                            })?;
                    if !metadata.is_dir() {
                        return Err(RuntimeError::denied(
                            core_policy::DenyReasonCode::WriteDenied,
                            format!(
                                "own-script write target {target:?} component {} must be a directory",
                                workspace.path.join(&resolved).display()
                            ),
                        ));
                    }
                }
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                unresolved_suffix = true;
                resolved.push(component);
            }
            Err(source) => {
                return Err(RuntimeError::Io {
                    path: workspace.path.join(candidate),
                    source,
                });
            }
        }
    }
    let relative = resolved
        .components()
        .map(|component| {
            component.as_os_str().to_str().ok_or_else(|| {
                RuntimeError::Protocol(format!(
                    "own-script write target {target:?} resolves to a non-UTF-8 path"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?
        .join("/");
    Ok(core_script::workspace_scope_path(&relative))
}

#[cfg(windows)]
fn windows_long_anchored_relative_path(
    workspace: &AnchoredDir,
    relative: &Path,
) -> Result<PathBuf, RuntimeError> {
    let mut parent = workspace.clone();
    let mut long_path = PathBuf::new();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let std::path::Component::Normal(requested) = component else {
            return Err(RuntimeError::Protocol(format!(
                "own-script resolved path {} must stay relative to {}",
                relative.display(),
                workspace.path.display()
            )));
        };
        let target = parent
            .dir
            .symlink_metadata(requested)
            .map_err(|source| path_io_error(&parent.path.join(requested), source))?;
        let mut matching_name = None;
        for entry in parent
            .dir
            .entries()
            .map_err(|source| path_io_error(&parent.path, source))?
        {
            let entry = entry.map_err(|source| path_io_error(&parent.path, source))?;
            let name = entry.file_name();
            let metadata = parent
                .dir
                .symlink_metadata(&name)
                .map_err(|source| path_io_error(&parent.path.join(&name), source))?;
            if metadata.dev() != target.dev() || metadata.ino() != target.ino() {
                continue;
            }
            if name == requested {
                matching_name = Some(name);
                break;
            }
            if matching_name.replace(name).is_some() {
                return Err(RuntimeError::denied(
                    core_policy::DenyReasonCode::WriteDenied,
                    format!(
                        "own-script resolved path {} has an ambiguous Windows alias",
                        relative.display()
                    ),
                ));
            }
        }
        let name = matching_name.ok_or_else(|| {
            RuntimeError::denied(
                core_policy::DenyReasonCode::WriteDenied,
                format!(
                    "own-script resolved path {} has no inventory-visible Windows name",
                    relative.display()
                ),
            )
        })?;
        long_path.push(&name);
        if components.peek().is_some() {
            let name = name.to_str().ok_or_else(|| {
                RuntimeError::Protocol(format!(
                    "own-script resolved path {} contains a non-UTF-8 directory name",
                    relative.display()
                ))
            })?;
            parent = parent.open_existing_child(name, DirectoryErrorMode::ScriptWrite)?;
        }
    }
    Ok(long_path)
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
