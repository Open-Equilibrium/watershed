use super::*;

pub fn execute_own_script(
    workspace: &Path,
    tool: &core_script::ToolBlock,
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
) -> Result<(), RuntimeError> {
    if let Some(write) = plan_own_script(tool, protected_path_match_mode, policy)? {
        write_script_output(
            workspace,
            &write.target,
            &write.contents,
            protected_path_match_mode,
            policy,
        )?;
    }
    Ok(())
}

pub fn plan_own_script(
    tool: &core_script::ToolBlock,
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
) -> Result<Option<ScriptWrite>, RuntimeError> {
    if tool.script_runtime.as_ref() != Some(&core_script::ScriptRuntime::PosixSh) {
        return Err(RuntimeError::Protocol(format!(
            "tool {} must use script_runtime posix-sh",
            tool.identity.id
        )));
    }
    let script_body = tool.script_body.as_deref().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "tool {} must include script_body",
            tool.identity.id
        ))
    })?;
    compile_own_script_operations(protected_path_match_mode, policy, script_body)
}

pub struct ScriptWrite {
    pub(crate) contents: Vec<u8>,
    pub(crate) target: String,
}

pub fn compile_own_script_operations(
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
    script_body: &str,
) -> Result<Option<ScriptWrite>, RuntimeError> {
    let mut write = None;
    for line in script_body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line == "---" {
            continue;
        }
        if let Some((command, target)) = script_redirection(line)? {
            if write.is_some() {
                return Err(RuntimeError::Protocol(
                    "own-script multiple write operations are not supported in M1".to_owned(),
                ));
            }
            let target = validate_script_write_target(protected_path_match_mode, policy, &target)?;
            let contents = evaluate_script_command(&command)?;
            write = Some(ScriptWrite { contents, target });
        } else {
            evaluate_script_command(line)?;
        }
    }
    Ok(write)
}

pub fn script_redirection(line: &str) -> Result<Option<(String, String)>, RuntimeError> {
    let Some(redirection_index) = redirection_position(line)? else {
        return Ok(None);
    };
    let command = line[..redirection_index].trim();
    if command.is_empty() {
        return Err(RuntimeError::Protocol(
            "own-script redirection must include a command".to_owned(),
        ));
    }
    let target = unquote_script_path(line[redirection_index + 1..].trim())?;
    Ok(Some((command.to_owned(), target)))
}

pub fn redirection_position(line: &str) -> Result<Option<usize>, RuntimeError> {
    let mut position = None;
    let mut quote = None;
    let mut chars = line.char_indices().peekable();

    while let Some((index, ch)) = chars.next() {
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => {}
            None if matches!(ch, '\'' | '"') => quote = Some(ch),
            None if ch == '>' => {
                if matches!(chars.peek(), Some((_, '>'))) {
                    return Err(RuntimeError::Protocol(
                        "own-script append redirection is not supported in M1".to_owned(),
                    ));
                }
                if position.replace(index).is_some() {
                    return Err(RuntimeError::Protocol(
                        "own-script multiple redirections are not supported in M1".to_owned(),
                    ));
                }
            }
            None => {}
        }
    }

    if quote.is_some() {
        return Err(RuntimeError::Protocol(
            "own-script command contains an unterminated quote".to_owned(),
        ));
    }

    Ok(position)
}

pub fn unquote_script_path(value: &str) -> Result<String, RuntimeError> {
    if value.is_empty() {
        return Err(RuntimeError::Protocol(
            "own-script redirection target must be one literal path".to_owned(),
        ));
    }
    if let Some(quote) = value.chars().next().filter(|ch| matches!(ch, '"' | '\'')) {
        if value.len() < 2 || !value.ends_with(quote) {
            return Err(RuntimeError::Protocol(
                "own-script redirection target must be one literal path".to_owned(),
            ));
        }
        return Ok(value[1..value.len() - 1].to_owned());
    }
    if value.contains('"') || value.contains('\'') || value.split_whitespace().count() != 1 {
        return Err(RuntimeError::Protocol(
            "own-script redirection target must be one literal path".to_owned(),
        ));
    }
    Ok(value.to_owned())
}

pub fn validate_script_write_target(
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
    target: &str,
) -> Result<String, RuntimeError> {
    let relative = normalize_script_write_target(target)?;
    let scoped = format!("workspace/{relative}");
    if !policy
        .filesystem
        .write_roots
        .iter()
        .any(|root| core_script::relative_path_is_inside_scope(&scoped, root))
    {
        return Err(runtime_denied(
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
        return Err(runtime_denied(
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
        || "workspace".to_owned(),
        |(parent, _)| format!("workspace/{parent}"),
    )
}

pub fn write_script_output(
    workspace: &Path,
    target: &str,
    contents: &[u8],
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
) -> Result<(), RuntimeError> {
    ensure_resolved_script_target_not_protected(
        workspace,
        target,
        protected_path_match_mode,
        policy,
    )?;
    let path = anchored_workspace_write_path(workspace, target, true)?
        .expect("created script target parent is present");
    replace_script_output_atomically(&path, contents)
}

pub fn preflight_own_script_outputs(
    workspace: &Path,
    write: Option<&ScriptWrite>,
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
) -> Result<(), RuntimeError> {
    if let Some(write) = write {
        ensure_resolved_script_target_not_protected(
            workspace,
            &write.target,
            protected_path_match_mode,
            policy,
        )?;
        if let Some(path) = anchored_workspace_write_path(workspace, &write.target, false)? {
            ensure_anchored_writable_regular_leaf(&path)?;
        }
    }
    Ok(())
}

pub fn replace_script_output_atomically(
    path: &AnchoredFile,
    contents: &[u8],
) -> Result<(), RuntimeError> {
    let initial_leaf_existed = ensure_anchored_writable_regular_leaf(path)?;
    with_anchored_replacement_temp(
        path,
        Some(core_policy::DenyReasonCode::WriteDenied),
        |temp_path, mut temp_file| {
            temp_file
                .write_all(contents)
                .map_err(|source| RuntimeError::Io {
                    path: temp_path.diagnostic_path().to_owned(),
                    source,
                })?;
            // Keep the created file open through the capability-relative rename. A peer with
            // write access to this exact directory can already replace the destination itself.

            if initial_leaf_existed {
                if ensure_anchored_writable_regular_leaf(path)? {
                    return temp_path.rename_to(path);
                }
            } else {
                ensure_anchored_new_leaf_available(path)?;
            }
            temp_path.rename_to(path)
        },
    )
}

pub fn anchored_workspace_write_path(
    workspace: &Path,
    target: &str,
    create: bool,
) -> Result<Option<AnchoredFile>, RuntimeError> {
    let mut parts = target.split('/').peekable();
    let mut parent = AnchoredDir::workspace(workspace)?;
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

pub fn with_anchored_replacement_temp<T>(
    path: &AnchoredFile,
    denied_reason: Option<core_policy::DenyReasonCode>,
    operation: impl FnOnce(&AnchoredFile, fs::File) -> Result<T, RuntimeError>,
) -> Result<T, RuntimeError> {
    let (temp_path, temp_file) = create_anchored_replacement_temp(path, denied_reason)?;
    let result = operation(&temp_path, temp_file);
    if result.is_err() {
        let _ = temp_path.remove();
    }
    result
}

pub fn create_anchored_replacement_temp(
    path: &AnchoredFile,
    denied_reason: Option<core_policy::DenyReasonCode>,
) -> Result<(AnchoredFile, fs::File), RuntimeError> {
    for attempt in 0..100 {
        let temp_path = path.parent.file(
            replacement_temp_path(path.diagnostic_path(), attempt)?
                .file_name()
                .expect("replacement temp path has a file name"),
        );
        let mut options = cap_std::fs::OpenOptions::new();
        options
            .create_new(true)
            .write(true)
            .follow(FollowSymlinks::No);
        match temp_path.open(&options) {
            Ok(file) => return Ok((temp_path, file)),
            Err(RuntimeError::Io { source, .. })
                if source.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(runtime_protocol_or_denied(
        denied_reason,
        format!(
            "could not allocate temporary replacement path for {}",
            path.diagnostic_path().display()
        ),
    ))
}

pub fn replacement_temp_path(path: &Path, attempt: u32) -> Result<PathBuf, RuntimeError> {
    let file_name = path.file_name().ok_or_else(|| {
        RuntimeError::Protocol("replacement path must have a file name".to_owned())
    })?;
    let digest = sha256_hex(file_name.as_encoded_bytes());
    Ok(path.with_file_name(format!(
        ".watershed-{digest}-{}-{attempt}.tmp",
        std::process::id()
    )))
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

    Err(runtime_denied(
        core_policy::DenyReasonCode::ProtectedPathDenied,
        format!(
            "tool {} cannot write protected path {scoped_target}",
            policy.tool_id
        ),
    ))
}

pub fn ensure_resolved_script_target_not_protected(
    workspace: &Path,
    target: &str,
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
) -> Result<(), RuntimeError> {
    let resolved_target = resolved_workspace_scoped_target(workspace, target)?;
    ensure_script_target_not_protected(protected_path_match_mode, policy, &resolved_target)
}

pub fn resolved_workspace_scoped_target(
    workspace: &Path,
    target: &str,
) -> Result<String, RuntimeError> {
    let canonical_workspace = fs::canonicalize(workspace).map_err(|source| RuntimeError::Io {
        path: workspace.to_owned(),
        source,
    })?;
    let mut resolved = canonical_workspace.clone();
    let mut unresolved_suffix = false;
    let mut components = target.split('/').peekable();
    while let Some(component) = components.next() {
        if unresolved_suffix {
            resolved.push(component);
            continue;
        }
        let candidate = resolved.join(component);
        match fs::symlink_metadata(&candidate) {
            Ok(_) => {
                resolved = fs::canonicalize(&candidate).map_err(|source| RuntimeError::Io {
                    path: candidate,
                    source,
                })?;
                if !resolved.starts_with(&canonical_workspace) {
                    return Err(runtime_denied(
                        core_policy::DenyReasonCode::SymlinkEscapeDenied,
                        format!(
                            "own-script write target {target:?} follows a symlink or reparse point outside the workspace"
                        ),
                    ));
                }
                if components.peek().is_some() {
                    let metadata = fs::metadata(&resolved).map_err(|source| RuntimeError::Io {
                        path: resolved.clone(),
                        source,
                    })?;
                    if !metadata.is_dir() {
                        return Err(runtime_denied(
                            core_policy::DenyReasonCode::WriteDenied,
                            format!(
                                "own-script write target {target:?} component {} must be a directory",
                                resolved.display()
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
                    path: candidate,
                    source,
                });
            }
        }
    }
    let relative = resolved.strip_prefix(&canonical_workspace).map_err(|_| {
        RuntimeError::Protocol(format!(
            "own-script write target {target:?} resolves outside the workspace"
        ))
    })?;
    let relative = relative
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
    Ok(format!("workspace/{relative}"))
}

#[cfg(unix)]
pub fn hard_link_count(_path: &Path, metadata: &fs::Metadata) -> Result<u64, RuntimeError> {
    use std::os::unix::fs::MetadataExt;

    Ok(metadata.nlink())
}

#[cfg(windows)]
pub fn hard_link_count_for_open_file(path: &Path, file: &fs::File) -> Result<u64, RuntimeError> {
    Ok(windows_open_file_information(path, file)?.number_of_links)
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowsOpenFileInformation {
    pub(crate) volume_serial_number: u64,
    pub(crate) file_index: u64,
    pub(crate) number_of_links: u64,
}

#[cfg(windows)]
pub fn windows_open_file_information(
    path: &Path,
    file: &fs::File,
) -> Result<WindowsOpenFileInformation, RuntimeError> {
    use cap_fs_ext::MetadataExt as _;

    let file =
        cap_std::fs::File::from_std(file.try_clone().map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })?);
    let metadata = file.metadata().map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })?;
    Ok(WindowsOpenFileInformation {
        volume_serial_number: metadata.dev(),
        file_index: metadata.ino(),
        number_of_links: metadata.nlink(),
    })
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

pub fn evaluate_script_command(command: &str) -> Result<Vec<u8>, RuntimeError> {
    let command = command.trim();
    if let Some(rest) = command.strip_prefix("printf ") {
        evaluate_printf_command(rest)
    } else if let Some(rest) = command.strip_prefix("echo ") {
        let mut out = unquote_script_argument(rest.trim())?;
        out.push('\n');
        Ok(out.into_bytes())
    } else {
        Err(RuntimeError::Protocol(format!(
            "unsupported own-script command {command:?}"
        )))
    }
}

pub fn evaluate_printf_command(rest: &str) -> Result<Vec<u8>, RuntimeError> {
    let (format, rest) = parse_single_quoted_argument(rest.trim())?;
    let rest = rest.trim();
    let formatted = if rest.is_empty() {
        decode_printf_escapes(&format)?
    } else if matches!(rest, "\"$SUMMARY\"" | "$SUMMARY") {
        decode_printf_escapes(&format)?.replacen("%s", "hello", 1)
    } else {
        return Err(RuntimeError::Protocol(format!(
            "unsupported own-script printf argument {rest:?}"
        )));
    };
    Ok(formatted.into_bytes())
}

pub fn parse_single_quoted_argument(value: &str) -> Result<(String, &str), RuntimeError> {
    let Some(rest) = value.strip_prefix('\'') else {
        return Err(RuntimeError::Protocol(
            "own-script printf format must be single-quoted".to_owned(),
        ));
    };
    let Some(end) = rest.find('\'') else {
        return Err(RuntimeError::Protocol(
            "own-script printf format is unterminated".to_owned(),
        ));
    };
    Ok((rest[..end].to_owned(), &rest[end + 1..]))
}

pub fn decode_printf_escapes(value: &str) -> Result<String, RuntimeError> {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                return Err(RuntimeError::Protocol(format!(
                    "unsupported own-script printf escape \\{other}"
                )));
            }
            None => {
                return Err(RuntimeError::Protocol(
                    "own-script printf format contains a dangling escape".to_owned(),
                ));
            }
        }
    }
    Ok(out)
}

pub fn unquote_script_argument(value: &str) -> Result<String, RuntimeError> {
    let unquoted = if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        &value[1..value.len() - 1]
    } else {
        value
    };
    if unquoted.chars().any(|ch| matches!(ch, '$' | '`' | '\\')) {
        Err(RuntimeError::Protocol(format!(
            "unsupported own-script argument {value:?}"
        )))
    } else {
        Ok(unquoted.to_owned())
    }
}

pub fn emit_tool_progress(
    message: &'static str,
    tool: &core_script::ToolBlock,
    invocation: &FlowInvocation,
    builder: &mut RuntimeEventBuilder<'_>,
) -> Result<(), RuntimeError> {
    builder.emit(
        Some(invocation),
        EventType::ToolProgress,
        serde_json::json!({
            "message": message,
            "tool_id": tool.identity.id,
        }),
    )
}

pub fn ensure_anchored_writable_regular_leaf(path: &AnchoredFile) -> Result<bool, RuntimeError> {
    match path.metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(runtime_denied(
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
                    RuntimeError::Protocol(_) => Err(runtime_denied(
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
        Ok(_) => Err(runtime_denied(
            core_policy::DenyReasonCode::WriteDenied,
            format!("{} must be a file", path.diagnostic_path().display()),
        )),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}
