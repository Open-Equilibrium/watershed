//! Writes a session's record into a new folder in the Downloads folder (ADR-0257). Nothing is
//! overwritten, and a failed export leaves no folder behind.

use std::path::{Path, PathBuf};

/// The text of `README.txt`: what the record covers and leaves out.
pub(crate) const README: &str = "Session record

session.jsonl   pi's session file, copied byte for byte.
record.json     The host, the harness, the project folder and the branch when exported. Per turn:
                its times in Unix milliseconds, how it ended, its start and end checkpoint commits
                and the files it changed, with added and removed lines.

Covers: only turns that ran through Meta-Harness, and the changes between each turn's start and
end checkpoints, in files of the project that Git does not ignore.

Leaves out: files Git ignores; files outside the project folder; effects beyond files, such as
commands, network requests or other programs; anything done outside Meta-Harness.

The transcript is pi's own file. Nothing here is signed or proven complete.
";

/// The name of the copied session file inside a record.
pub(crate) const SESSION_FILE: &str = "session.jsonl";

/// Longest title kept in a folder name, in bytes. With ` – record <day>` (22 bytes), a numbered
/// suffix and the temporary folder's `.` and `.partial`, every name stays within 255 bytes.
const TITLE_BYTES: usize = 200;

/// The Downloads folder: the one the Linux desktop names in `user-dirs.dirs`, else `~/Downloads`
/// (macOS has no such file).
pub(crate) fn download_dir() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let user_dirs = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
        .join("user-dirs.dirs");
    let text = cfg!(target_os = "linux")
        .then(|| std::fs::read_to_string(user_dirs).ok())
        .flatten();
    Some(download_dir_in(&home, text.as_deref()))
}

/// The Downloads folder for a home folder and the text of `user-dirs.dirs`, if any.
pub(crate) fn download_dir_in(home: &Path, user_dirs: Option<&str>) -> PathBuf {
    user_dirs
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.trim().strip_prefix("XDG_DOWNLOAD_DIR="))
        })
        .map(|value| value.trim_matches('"'))
        .and_then(|value| match value.strip_prefix("$HOME") {
            Some(rest) => Some(home.join(rest.trim_start_matches('/'))),
            None => value.starts_with('/').then(|| PathBuf::from(value)),
        })
        .unwrap_or_else(|| home.join("Downloads"))
}

/// The folder name of a record: the session's title without characters a file name cannot hold,
/// then the day it was exported, as `<title> – record <day>`.
pub(crate) fn record_name(title: &str, day: &str) -> String {
    let mut bytes = 0;
    let kept: String = title
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        .map_while(|c| {
            bytes += c.len_utf8();
            (bytes <= TITLE_BYTES).then_some(c)
        })
        .collect();
    let title = kept.trim().trim_start_matches('.').trim();
    let title = if title.is_empty() {
        "Untitled session"
    } else {
        title
    };
    format!("{title} – record {day}")
}

/// Creates a folder named `name` in `downloads`, or `name 2`, `name 3`… when it exists, and fills
/// it with `write` in a temporary sibling that is renamed into place only once it succeeded. The
/// folder is claimed by creating it first, so no other export takes its name; a failure removes
/// both. Returns the folder's path.
pub(crate) fn write_folder(
    downloads: &Path,
    name: &str,
    write: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<PathBuf, String> {
    let mut number = 1;
    let folder = loop {
        let candidate = if number == 1 {
            name.to_owned()
        } else {
            format!("{name} {number}")
        };
        let folder = downloads.join(candidate);
        match std::fs::create_dir(&folder) {
            Ok(()) => break folder,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => number += 1,
            Err(error) => return Err(format!("could not create {}: {error}", folder.display())),
        }
    };
    let temporary = folder.with_file_name(format!(
        ".{}.partial",
        folder.file_name().unwrap_or_default().to_string_lossy()
    ));
    let _ = std::fs::remove_dir_all(&temporary);
    let result = std::fs::create_dir(&temporary)
        .map_err(|error| format!("could not create {}: {error}", temporary.display()))
        .and_then(|()| write(&temporary))
        .and_then(|()| {
            std::fs::rename(&temporary, &folder)
                .map_err(|error| format!("could not finish {}: {error}", folder.display()))
        });
    match result {
        Ok(()) => Ok(folder),
        Err(error) => {
            let _ = std::fs::remove_dir_all(&temporary);
            let _ = std::fs::remove_dir(&folder);
            Err(error)
        }
    }
}
