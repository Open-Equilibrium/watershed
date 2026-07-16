fn execute_own_script(
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

fn plan_own_script(
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

struct ScriptWrite {
    contents: Vec<u8>,
    target: String,
}

fn compile_own_script_operations(
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

fn script_redirection(line: &str) -> Result<Option<(String, String)>, RuntimeError> {
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

fn redirection_position(line: &str) -> Result<Option<usize>, RuntimeError> {
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

fn unquote_script_path(value: &str) -> Result<String, RuntimeError> {
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

fn validate_script_write_target(
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

fn script_replacement_temp_parent_scope(relative: &str) -> String {
    relative.rsplit_once('/').map_or_else(
        || "workspace".to_owned(),
        |(parent, _)| format!("workspace/{parent}"),
    )
}

fn write_script_output(
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
    let path = ensure_real_workspace_write_path(workspace, target)?;
    replace_script_output_atomically(workspace, target, &path, contents)
}

fn preflight_own_script_outputs(
    workspace: &Path,
    write: Option<&ScriptWrite>,
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
) -> Result<(), RuntimeError> {
    if let Some(write) = write {
        let path = preflight_real_workspace_write_path(workspace, &write.target)?;
        ensure_resolved_script_target_not_protected(
            workspace,
            &write.target,
            protected_path_match_mode,
            policy,
        )?;
        ensure_writable_regular_leaf(&path)?;
    }
    Ok(())
}

fn replace_script_output_atomically(
    workspace: &Path,
    target: &str,
    path: &Path,
    contents: &[u8],
) -> Result<(), RuntimeError> {
    ensure_real_workspace_write_path(workspace, target)?;
    let initial_leaf_existed = ensure_writable_regular_leaf(path)?;
    with_replacement_temp(
        path,
        Some(core_policy::DenyReasonCode::WriteDenied),
        |temp_path, mut temp_file| {
            temp_file
                .write_all(contents)
                .map_err(|source| RuntimeError::Io {
                    path: temp_path.to_owned(),
                    source,
                })?;
            drop(temp_file);

            ensure_real_workspace_write_path(workspace, target)?;
            if initial_leaf_existed {
                if ensure_writable_regular_leaf(path)? {
                    return replace_existing_leaf_from_temp(path, temp_path);
                }
            } else {
                ensure_new_leaf_available(path)?;
            }
            ensure_real_workspace_write_path(workspace, target)?;
            fs::rename(temp_path, path).map_err(|source| RuntimeError::Io {
                path: path.to_owned(),
                source,
            })
        },
    )
}

fn replace_existing_leaf_from_temp(path: &Path, temp_path: &Path) -> Result<(), RuntimeError> {
    fs::rename(temp_path, path).map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })
}

fn with_replacement_temp<T>(
    path: &Path,
    denied_reason: Option<core_policy::DenyReasonCode>,
    operation: impl FnOnce(&Path, fs::File) -> Result<T, RuntimeError>,
) -> Result<T, RuntimeError> {
    let (temp_path, temp_file) = create_replacement_temp(path, denied_reason)?;
    let result = operation(&temp_path, temp_file);
    if result.is_err() {
        let _ = fs::remove_file(temp_path);
    }
    result
}

fn create_replacement_temp(
    path: &Path,
    denied_reason: Option<core_policy::DenyReasonCode>,
) -> Result<(PathBuf, fs::File), RuntimeError> {
    for attempt in 0..100 {
        let temp_path = replacement_temp_path(path, attempt)?;
        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((temp_path, file)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(RuntimeError::Io {
                    path: temp_path,
                    source,
                });
            }
        }
    }
    Err(runtime_protocol_or_denied(
        denied_reason,
        format!(
            "could not allocate temporary replacement path for {}",
            path.display()
        ),
    ))
}

fn replacement_temp_path(path: &Path, attempt: u32) -> Result<PathBuf, RuntimeError> {
    let mut file_name = path
        .file_name()
        .ok_or_else(|| RuntimeError::Protocol("replacement path must have a file name".to_owned()))?
        .to_os_string();
    file_name.push(format!(".watershed-{}-{attempt}.tmp", std::process::id()));
    Ok(path.with_file_name(file_name))
}

fn ensure_script_target_not_protected(
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

fn ensure_resolved_script_target_not_protected(
    workspace: &Path,
    target: &str,
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
) -> Result<(), RuntimeError> {
    let resolved_target = resolved_workspace_scoped_target(workspace, target)?;
    ensure_script_target_not_protected(protected_path_match_mode, policy, &resolved_target)
}

fn resolved_workspace_scoped_target(
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
                                "own-script write target {target:?} has non-directory component {}",
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

fn ensure_real_workspace_write_path(
    workspace: &Path,
    target: &str,
) -> Result<PathBuf, RuntimeError> {
    let mut parts = target.split('/').peekable();
    let mut path = workspace.to_path_buf();
    while let Some(part) = parts.next() {
        path.push(part);
        if parts.peek().is_some() {
            ensure_created_script_real_directory(&path)?;
        }
    }
    Ok(path)
}

fn preflight_real_workspace_write_path(
    workspace: &Path,
    target: &str,
) -> Result<PathBuf, RuntimeError> {
    let mut parts = target.split('/').peekable();
    let mut path = workspace.to_path_buf();
    while let Some(part) = parts.next() {
        path.push(part);
        if parts.peek().is_some() {
            ensure_optional_script_real_directory(&path)?;
        }
    }
    Ok(path)
}

fn ensure_optional_script_real_directory(path: &Path) -> Result<bool, RuntimeError> {
    ensure_optional_directory_with(path, DirectoryErrorMode::ScriptWrite)
}

fn ensure_created_script_real_directory(path: &Path) -> Result<bool, RuntimeError> {
    ensure_created_directory_with(path, DirectoryErrorMode::ScriptWrite)
}

#[cfg(unix)]
fn ensure_opened_regular_leaf_matches_path(
    path: &Path,
    file: &fs::File,
) -> Result<(), RuntimeError> {
    let path_metadata = fs::symlink_metadata(path).map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })?;
    if path_metadata.file_type().is_symlink() {
        return Err(RuntimeError::Protocol(format!(
            "{} must not be a symlink",
            path.display()
        )));
    }
    if !path_metadata.is_file() {
        return Err(RuntimeError::Protocol(format!(
            "{} must be a file",
            path.display()
        )));
    }

    let file_metadata = file.metadata().map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })?;
    if !file_metadata.is_file() || !same_file_metadata(&path_metadata, &file_metadata) {
        return Err(RuntimeError::Protocol(format!(
            "{} changed before write",
            path.display()
        )));
    }
    ensure_not_hardlinked_file(path, &file_metadata)?;

    Ok(())
}

#[cfg(unix)]
fn same_file_metadata(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(unix)]
fn hard_link_count(_path: &Path, metadata: &fs::Metadata) -> Result<u64, RuntimeError> {
    use std::os::unix::fs::MetadataExt;

    Ok(metadata.nlink())
}

#[cfg(windows)]
fn hard_link_count(path: &Path, _metadata: &fs::Metadata) -> Result<u64, RuntimeError> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })?;
    hard_link_count_for_open_file(path, &file)
}

#[cfg(windows)]
fn hard_link_count_for_open_file(path: &Path, file: &fs::File) -> Result<u64, RuntimeError> {
    Ok(windows_open_file_information(path, file)?.number_of_links)
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WindowsOpenFileInformation {
    volume_serial_number: u64,
    file_index: u64,
    number_of_links: u64,
}

#[cfg(windows)]
fn windows_open_file_information(
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

fn normalize_script_write_target(target: &str) -> Result<String, RuntimeError> {
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

fn evaluate_script_command(command: &str) -> Result<Vec<u8>, RuntimeError> {
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

fn evaluate_printf_command(rest: &str) -> Result<Vec<u8>, RuntimeError> {
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

fn parse_single_quoted_argument(value: &str) -> Result<(String, &str), RuntimeError> {
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

fn decode_printf_escapes(value: &str) -> Result<String, RuntimeError> {
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

fn unquote_script_argument(value: &str) -> Result<String, RuntimeError> {
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

fn emit_tool_progress(
    message: &'static str,
    tool: &core_script::ToolBlock,
    invocation: &LoopInvocation,
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

fn ensure_writable_regular_leaf(path: &Path) -> Result<bool, RuntimeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(runtime_denied(
            core_policy::DenyReasonCode::SymlinkEscapeDenied,
            format!("{} must not be a symlink", path.display()),
        )),
        Ok(metadata) if metadata.is_file() => {
            ensure_script_leaf_not_hardlinked(path, &metadata)?;
            Ok(true)
        }
        Ok(_) => Err(runtime_denied(
            core_policy::DenyReasonCode::WriteDenied,
            format!("{} must be a file", path.display()),
        )),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(RuntimeError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

#[cfg(any(unix, windows))]
fn ensure_script_leaf_not_hardlinked(
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<(), RuntimeError> {
    if hard_link_count(path, metadata)? > 1 {
        return Err(runtime_denied(
            core_policy::DenyReasonCode::WriteDenied,
            format!("{} must not be hard-linked", path.display()),
        ));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn ensure_script_leaf_not_hardlinked(
    _path: &Path,
    _metadata: &fs::Metadata,
) -> Result<(), RuntimeError> {
    Ok(())
}
