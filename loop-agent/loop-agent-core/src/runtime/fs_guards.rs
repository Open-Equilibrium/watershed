#[derive(Clone, Debug)]
struct AnchoredDir {
    dir: std::sync::Arc<Dir>,
    path: PathBuf,
}

#[derive(Clone, Debug)]
struct AnchoredFile {
    parent: AnchoredDir,
    leaf: PathBuf,
    path: PathBuf,
}

impl AnchoredDir {
    fn workspace(path: &Path) -> Result<Self, RuntimeError> {
        let dir = Dir::open_ambient_dir(path, ambient_authority())
            .map_err(|source| path_io_error(path, source))?;
        Ok(Self {
            dir: std::sync::Arc::new(dir),
            path: path.to_owned(),
        })
    }

    fn child(
        &self,
        leaf: &str,
        create: bool,
        error_mode: DirectoryErrorMode,
    ) -> Result<Option<Self>, RuntimeError> {
        let path = self.path.join(leaf);
        match self.dir.symlink_metadata(leaf) {
            Ok(metadata) => validate_anchored_directory(&path, &metadata, error_mode)?,
            Err(err) if err.kind() == io::ErrorKind::NotFound && !create => return Ok(None),
            Err(err) if err.kind() == io::ErrorKind::NotFound => match self.dir.create_dir(leaf) {
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
                Err(source) => return Err(path_io_error(&path, source)),
            },
            Err(source) => return Err(path_io_error(&path, source)),
        }
        let dir = self
            .dir
            .open_dir_nofollow(leaf)
            .map_err(|source| unsafe_anchored_directory(path.clone(), source, error_mode))?;
        Ok(Some(Self {
            dir: std::sync::Arc::new(dir),
            path,
        }))
    }

    fn file(&self, leaf: impl Into<PathBuf>) -> AnchoredFile {
        let leaf = leaf.into();
        AnchoredFile {
            path: self.path.join(&leaf),
            parent: self.clone(),
            leaf,
        }
    }
}

impl AnchoredFile {
    fn diagnostic_path(&self) -> &Path {
        &self.path
    }

    fn metadata(&self) -> Result<cap_std::fs::Metadata, RuntimeError> {
        self.parent
            .dir
            .symlink_metadata(&self.leaf)
            .map_err(|source| path_io_error(&self.path, source))
    }

    fn open(&self, options: &cap_std::fs::OpenOptions) -> Result<fs::File, RuntimeError> {
        self.parent
            .dir
            .open_with(&self.leaf, options)
            .map(cap_std::fs::File::into_std)
            .map_err(|source| path_io_error(&self.path, source))
    }

    fn remove(&self) -> Result<(), RuntimeError> {
        self.parent
            .dir
            .remove_file(&self.leaf)
            .map_err(|source| path_io_error(&self.path, source))
    }

    fn rename_to(&self, target: &Self) -> Result<(), RuntimeError> {
        self.parent
            .dir
            .rename(&self.leaf, &target.parent.dir, &target.leaf)
            .map_err(|source| path_io_error(&target.path, source))
    }
}

fn ensure_anchored_new_leaf_available(file: &AnchoredFile) -> Result<(), RuntimeError> {
    match file.metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(RuntimeError::Protocol(format!(
            "{} must not be a symlink or reparse point",
            file.path.display()
        ))),
        Ok(_) => Err(path_io_error(
            &file.path,
            io::Error::new(io::ErrorKind::AlreadyExists, "file already exists"),
        )),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn ensure_anchored_real_file(file: &AnchoredFile) -> Result<(), RuntimeError> {
    let metadata = file.metadata()?;
    if metadata.file_type().is_symlink() {
        return Err(RuntimeError::Protocol(format!(
            "{} must not be a symlink or reparse point",
            file.path.display()
        )));
    }
    if !metadata.is_file() {
        return Err(RuntimeError::Protocol(format!(
            "{} must be a file",
            file.path.display()
        )));
    }
    Ok(())
}

fn open_anchored_real_file_for_read(
    file: &AnchoredFile,
) -> Result<(fs::File, fs::Metadata), RuntimeError> {
    ensure_anchored_real_file(file)?;
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let opened = file.open(&options)?;
    let metadata = opened
        .metadata()
        .map_err(|source| path_io_error(&file.path, source))?;
    validate_real_file(&file.path, &metadata)?;
    Ok((opened, metadata))
}

fn open_anchored_file_for_read(
    file: &AnchoredFile,
) -> Result<(fs::File, fs::Metadata), RuntimeError> {
    let (opened, metadata) = open_anchored_real_file_for_read(file)?;
    ensure_not_hardlinked_open_file(&file.path, &opened, &metadata)?;
    Ok((opened, metadata))
}

fn read_anchored_file_with_limit(
    file: &AnchoredFile,
    max_bytes: u64,
) -> Result<Vec<u8>, RuntimeError> {
    let (opened, metadata) = open_anchored_file_for_read(file)?;
    read_opened_file_with_limit(opened, metadata.len(), &file.path, max_bytes)
}

fn read_anchored_to_string_with_limit(
    file: &AnchoredFile,
    max_bytes: u64,
) -> Result<String, RuntimeError> {
    decode_utf8(&file.path, read_anchored_file_with_limit(file, max_bytes)?)
}

fn segmented_jsonl_path(base: &AnchoredFile, ordinal: u64) -> Result<AnchoredFile, RuntimeError> {
    if ordinal == 1 {
        return Ok(base.clone());
    }
    let leaf = segmented_jsonl_stem(base)?;
    Ok(base.parent.file(format!("{leaf}.{ordinal:06}.jsonl")))
}

fn segmented_jsonl_stem(base: &AnchoredFile) -> Result<&str, RuntimeError> {
    base.leaf
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .and_then(|leaf| leaf.strip_suffix(".jsonl"))
        .ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "{} segmented JSONL path must end in .jsonl",
                base.diagnostic_path().display()
            ))
        })
}

fn segmented_jsonl_siblings(base: &AnchoredFile) -> Result<Vec<(u64, AnchoredFile)>, RuntimeError> {
    let leaf = segmented_jsonl_stem(base)?;
    let base_name = format!("{leaf}.jsonl");
    let prefix = format!("{leaf}.");
    let mut siblings = Vec::new();
    for entry in base
        .parent
        .dir
        .entries()
        .map_err(|source| path_io_error(&base.parent.path, source))?
    {
        let entry = entry.map_err(|source| path_io_error(&base.parent.path, source))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let candidate = name.to_ascii_lowercase();
        if candidate == base_name && name != base_name {
            return Err(RuntimeError::Protocol(format!(
                "{} contains non-canonical segmented JSONL name {name}",
                base.parent.path.display()
            )));
        }
        let Some(ordinal) = candidate
            .strip_prefix(&prefix)
            .and_then(|suffix| suffix.strip_suffix(".jsonl"))
            .filter(|ordinal| {
                ordinal.len() == 6 && ordinal.bytes().all(|byte| byte.is_ascii_digit())
            })
            .and_then(|ordinal| ordinal.parse::<u64>().ok())
        else {
            continue;
        };
        if candidate != name {
            return Err(RuntimeError::Protocol(format!(
                "{} contains non-canonical segmented JSONL name {name}",
                base.parent.path.display()
            )));
        }
        siblings.push((ordinal, base.parent.file(name)));
    }
    siblings.sort_unstable_by_key(|(ordinal, _)| *ordinal);
    Ok(siblings)
}

fn segmented_jsonl_files(
    base: &AnchoredFile,
    limits: SessionStreamLimits,
) -> Result<Vec<AnchoredFile>, RuntimeError> {
    ensure_anchored_real_file(base)?;
    let mut files = vec![base.clone()];
    for (expected, (ordinal, candidate)) in (2..).zip(segmented_jsonl_siblings(base)?) {
        if ordinal < 2 {
            return Err(RuntimeError::Protocol(format!(
                "{} has invalid segmented JSONL ordinal {ordinal:06}",
                base.diagnostic_path().display()
            )));
        }
        if ordinal > limits.max_segments {
            return Err(RuntimeError::Protocol(format!(
                "{} segment count exceeds max {}",
                base.diagnostic_path().display(),
                limits.max_segments
            )));
        }
        if ordinal != expected {
            return Err(RuntimeError::Protocol(format!(
                "{} has non-contiguous segmented JSONL ordinals",
                base.diagnostic_path().display()
            )));
        }
        ensure_anchored_real_file(&candidate)?;
        files.push(candidate);
    }
    Ok(files)
}

fn read_segmented_jsonl(
    base: &AnchoredFile,
    limits: SessionStreamLimits,
) -> Result<String, RuntimeError> {
    let mut bytes = Vec::new();
    let files = segmented_jsonl_files(base, limits)?;
    for (index, file) in files.iter().enumerate() {
        let segment = read_anchored_file_with_limit(file, MAX_SESSION_SEGMENT_BYTES)?;
        if index + 1 != files.len() && !segment.ends_with(b"\n") {
            return Err(RuntimeError::Protocol(format!(
                "{} non-final segment must end with LF",
                file.diagnostic_path().display()
            )));
        }
        let total = u64::try_from(bytes.len().saturating_add(segment.len())).unwrap_or(u64::MAX);
        if total > limits.max_total_bytes {
            return Err(RuntimeError::Protocol(format!(
                "{} segmented JSONL size {total} bytes exceeds max {}",
                base.diagnostic_path().display(),
                limits.max_total_bytes
            )));
        }
        bytes.extend_from_slice(&segment);
    }
    decode_utf8(base.diagnostic_path(), bytes)
}

fn for_each_segmented_jsonl_line(
    base: &AnchoredFile,
    limits: SessionStreamLimits,
    mut visit: impl FnMut(&str) -> Result<(), RuntimeError>,
) -> Result<u64, RuntimeError> {
    let mut total = 0u64;
    for file in segmented_jsonl_files(base, limits)? {
        let segment_bytes =
            for_each_anchored_file_line_with_limit(&file, MAX_SESSION_SEGMENT_BYTES, |line| {
                visit(line)
            })?;
        total = total.saturating_add(segment_bytes);
        if total > limits.max_total_bytes {
            return Err(RuntimeError::Protocol(format!(
                "{} segmented JSONL size {total} bytes exceeds max {}",
                base.diagnostic_path().display(),
                limits.max_total_bytes
            )));
        }
    }
    Ok(total)
}

fn remove_segmented_jsonl(base: &AnchoredFile) {
    if let Ok(siblings) = segmented_jsonl_siblings(base) {
        for (_, file) in siblings.into_iter().rev() {
            let _ = file.remove();
        }
    }
    let _ = base.remove();
}

fn create_anchored_file(file: &AnchoredFile) -> Result<fs::File, RuntimeError> {
    ensure_anchored_new_leaf_available(file)?;
    let mut options = cap_std::fs::OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    file.open(&options)
}

fn ensure_anchored_non_hardlinked_file(file: &AnchoredFile) -> Result<(), RuntimeError> {
    open_anchored_file_for_read(file).map(|_| ())
}

#[cfg(any(unix, windows))]
fn ensure_not_hardlinked_open_file(
    path: &Path,
    file: &fs::File,
    _metadata: &fs::Metadata,
) -> Result<(), RuntimeError> {
    #[cfg(unix)]
    let _ = file;
    #[cfg(unix)]
    let links = hard_link_count(path, _metadata)?;
    #[cfg(windows)]
    let links = hard_link_count_for_open_file(path, file)?;
    if links > 1 {
        return Err(RuntimeError::Protocol(format!(
            "{} must not be hard-linked",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn ensure_not_hardlinked_open_file(
    _path: &Path,
    _file: &fs::File,
    _metadata: &fs::Metadata,
) -> Result<(), RuntimeError> {
    Ok(())
}

struct RuntimeDirs {
    logs: AnchoredDir,
    sessions: AnchoredDir,
}

fn ensure_runtime_dirs(workspace: &Path) -> Result<RuntimeDirs, RuntimeError> {
    let workspace = AnchoredDir::workspace(workspace)?;
    let loop_dir = workspace
        .child(".loop", true, DirectoryErrorMode::Protocol)?
        .expect("created runtime directory is present");
    let sessions = loop_dir
        .child("sessions", true, DirectoryErrorMode::Protocol)?
        .expect("created session directory is present");
    let logs = loop_dir
        .child("logs", true, DirectoryErrorMode::Protocol)?
        .expect("created log directory is present");
    Ok(RuntimeDirs { logs, sessions })
}

fn open_runtime_dir(workspace: &Path, leaf: &str) -> Result<Option<AnchoredDir>, RuntimeError> {
    let workspace = AnchoredDir::workspace(workspace)?;
    let Some(loop_dir) = workspace.child(".loop", false, DirectoryErrorMode::Protocol)? else {
        return Ok(None);
    };
    loop_dir.child(leaf, false, DirectoryErrorMode::Protocol)
}

fn validate_anchored_directory(
    path: &Path,
    metadata: &cap_std::fs::Metadata,
    error_mode: DirectoryErrorMode,
) -> Result<(), RuntimeError> {
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(unsafe_anchored_directory(
            path.to_owned(),
            io::Error::other("not a real directory"),
            error_mode,
        ));
    }
    Ok(())
}

fn unsafe_anchored_directory(
    path: PathBuf,
    source: io::Error,
    error_mode: DirectoryErrorMode,
) -> RuntimeError {
    let message = format!(
        "{} must not be a symlink or reparse point and must be a directory: {source}",
        path.display()
    );
    match error_mode {
        DirectoryErrorMode::Protocol => RuntimeError::Protocol(message),
        DirectoryErrorMode::ScriptWrite => {
            runtime_denied(core_policy::DenyReasonCode::SymlinkEscapeDenied, message)
        }
    }
}

#[derive(Clone, Copy)]
enum DirectoryErrorMode {
    Protocol,
    ScriptWrite,
}

#[cfg(windows)]
fn has_windows_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn has_windows_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn open_anchored_session_log_append_file(path: &AnchoredFile) -> Result<fs::File, RuntimeError> {
    let mut options = cap_std::fs::OpenOptions::new();
    #[cfg(not(windows))]
    options.append(true);
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt as _;

        const FILE_SHARE_READ: u32 = 0x0000_0001;
        options.read(true).write(true).share_mode(FILE_SHARE_READ);
    }
    options.follow(FollowSymlinks::No);
    let file = path.open(&options)?;
    validate_open_session_log_append_file(&path.path, &file)?;
    Ok(file)
}
fn validate_open_session_log_append_file(path: &Path, file: &fs::File) -> Result<(), RuntimeError> {
    let metadata = file
        .metadata()
        .map_err(|source| path_io_error(path, source))?;
    validate_real_file(path, &metadata)?;
    ensure_not_hardlinked_open_file(path, file, &metadata)
}

fn validate_real_file(path: &Path, metadata: &fs::Metadata) -> Result<(), RuntimeError> {
    if metadata.file_type().is_symlink() || has_windows_reparse_point(metadata) {
        return Err(RuntimeError::Protocol(format!(
            "{} must not be a symlink or reparse point",
            path.display()
        )));
    }
    if !metadata.is_file() {
        return Err(RuntimeError::Protocol(format!(
            "{} must be a file",
            path.display()
        )));
    }
    Ok(())
}
