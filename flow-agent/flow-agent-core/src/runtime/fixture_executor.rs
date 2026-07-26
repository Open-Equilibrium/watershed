use super::*;

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

pub fn execute_own_script(
    workspace: &AnchoredDir,
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
    workspace: &AnchoredDir,
    target: &str,
    contents: &[u8],
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
) -> Result<(), RuntimeError> {
    let resolved_target = resolved_anchored_workspace_scoped_target(workspace, target)?;
    ensure_script_target_not_protected(protected_path_match_mode, policy, &resolved_target)?;
    let path = anchored_workspace_write_path_from(workspace, target, true)?
        .expect("created script target parent is present");
    replace_script_output_atomically_checked(&path, contents, |temp_path| {
        let temp_name = temp_path
            .diagnostic_path()
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                RuntimeError::Protocol(
                    "own-script replacement temp must have a UTF-8 file name".to_owned(),
                )
            })?;
        let (resolved_parent, _) = resolved_target.rsplit_once('/').ok_or_else(|| {
            RuntimeError::Protocol(
                "resolved own-script target must include a workspace-relative file name".to_owned(),
            )
        })?;
        ensure_script_target_not_protected(
            protected_path_match_mode,
            policy,
            &format!("{resolved_parent}/{temp_name}"),
        )
    })
}

pub fn preflight_own_script_outputs(
    workspace: &AnchoredDir,
    write: Option<&ScriptWrite>,
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
) -> Result<(), RuntimeError> {
    if let Some(write) = write {
        ensure_resolved_anchored_script_target_not_protected(
            workspace,
            &write.target,
            protected_path_match_mode,
            policy,
        )?;
        if let Some(path) = anchored_workspace_write_path_from(workspace, &write.target, false)? {
            ensure_script_output_target_available(&path)?;
        }
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
                    return Err(runtime_denied(
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
        return Err(runtime_denied(
            core_policy::DenyReasonCode::WriteDenied,
            format!(
                "own-script output {} already exists; existing outputs are not replaceable",
                path.diagnostic_path().display()
            ),
        ));
    }
    Ok(())
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

pub fn with_anchored_replacement_temp<T>(
    path: &AnchoredFile,
    denied_reason: Option<core_policy::DenyReasonCode>,
    operation: impl FnOnce(&AnchoredFile, fs::File) -> Result<T, RuntimeError>,
) -> Result<T, RuntimeError> {
    with_anchored_replacement_temp_checked(path, denied_reason, |_| Ok(()), operation)
}

fn with_anchored_replacement_temp_checked<T>(
    path: &AnchoredFile,
    denied_reason: Option<core_policy::DenyReasonCode>,
    candidate_check: impl Fn(&AnchoredFile) -> Result<(), RuntimeError>,
    operation: impl FnOnce(&AnchoredFile, fs::File) -> Result<T, RuntimeError>,
) -> Result<T, RuntimeError> {
    let (temp_path, temp_file) =
        create_anchored_replacement_temp(path, denied_reason, candidate_check)?;
    match operation(&temp_path, temp_file) {
        Ok(value) => Ok(value),
        Err(operation @ RuntimeError::PublishedOutputCleanupFailure { .. }) => Err(operation),
        Err(operation) => match temp_path.remove() {
            Ok(()) => Err(operation),
            Err(cleanup) => Err(RuntimeError::TemporaryReplacementFailures {
                operation: Box::new(operation),
                cleanup: Box::new(cleanup),
            }),
        },
    }
}

fn create_anchored_replacement_temp(
    path: &AnchoredFile,
    denied_reason: Option<core_policy::DenyReasonCode>,
    candidate_check: impl Fn(&AnchoredFile) -> Result<(), RuntimeError>,
) -> Result<(AnchoredFile, fs::File), RuntimeError> {
    for attempt in 0..100 {
        let temp_path = path.parent.file(
            replacement_temp_path(path.diagnostic_path(), attempt)?
                .file_name()
                .expect("replacement temp path has a file name"),
        );
        candidate_check(&temp_path)?;
        let mut options = cap_std::fs::OpenOptions::new();
        options
            .create_new(true)
            .write(true)
            .follow(FollowSymlinks::No);
        #[cfg(windows)]
        {
            use cap_std::fs::OpenOptionsExt as _;
            use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_WRITE, WRITE_DAC};

            options.access_mode(FILE_GENERIC_WRITE | WRITE_DAC);
        }
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
                    runtime_denied(
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
                        return Err(runtime_denied(
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
    Ok(format!("workspace/{relative}"))
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
                return Err(runtime_denied(
                    core_policy::DenyReasonCode::WriteDenied,
                    format!(
                        "own-script resolved path {} has an ambiguous Windows alias",
                        relative.display()
                    ),
                ));
            }
        }
        let name = matching_name.ok_or_else(|| {
            runtime_denied(
                core_policy::DenyReasonCode::WriteDenied,
                format!(
                    "own-script resolved path {} has no inventory-visible Windows name",
                    relative.display()
                ),
            )
        })?;
        long_path.push(&name);
        if components.peek().is_some() {
            let dir = parent
                .dir
                .open_dir_nofollow(&name)
                .map_err(|source| path_io_error(&parent.path.join(&name), source))?;
            parent = AnchoredDir {
                dir: std::sync::Arc::new(dir),
                path: workspace.path.join(&long_path),
            };
        }
    }
    Ok(long_path)
}

#[cfg(unix)]
pub fn hard_link_count(_path: &Path, metadata: &fs::Metadata) -> Result<u64, RuntimeError> {
    Ok(std::os::unix::fs::MetadataExt::nlink(metadata))
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
    let argument = if rest.is_empty() {
        None
    } else if matches!(rest, "\"$SUMMARY\"" | "$SUMMARY") {
        Some("hello")
    } else {
        return Err(RuntimeError::Protocol(format!(
            "unsupported own-script printf argument {rest:?}"
        )));
    };
    let formatted = evaluate_printf_string_format(&decode_printf_escapes(&format)?, argument)?;
    Ok(formatted.into_bytes())
}

pub fn evaluate_printf_string_format(
    format: &str,
    mut argument: Option<&str>,
) -> Result<String, RuntimeError> {
    let mut formatted = String::new();
    let mut chars = format.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            formatted.push(ch);
            continue;
        }
        match chars.next() {
            Some('%') => formatted.push('%'),
            Some('s') => formatted.push_str(argument.take().unwrap_or_default()),
            Some(conversion) => {
                return Err(RuntimeError::Protocol(format!(
                    "unsupported own-script printf conversion %{conversion}"
                )));
            }
            None => {
                return Err(RuntimeError::Protocol(
                    "unsupported own-script printf conversion at end of format".to_owned(),
                ));
            }
        }
    }
    Ok(formatted)
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
