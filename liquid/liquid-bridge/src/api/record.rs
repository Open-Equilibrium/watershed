use std::path::{Path, PathBuf};

pub(crate) const README: &str = "Session record

thread.json     T3 Code's record of the thread as Meta-Harness held it when exported: the
                thread, its runs, messages, tool calls, provider turns with their tokens,
                requests and checkpoints.
record.json     The host, the harness, the project folder and the branch when exported. Per turn:
                its times in Unix milliseconds, how it ended and the files its end checkpoint
                changed, with added and removed lines.

Covers: only turns that ran through Meta-Harness, and the changes T3 captured between each
turn's checkpoints, in files of the checkout that Git does not ignore.
Leaves out: reverted turns; files Git ignores; files outside the checkout; effects beyond files,
such as commands, network requests or other programs; anything done outside Meta-Harness.
Nothing here is signed or proven complete.
";

pub(crate) const THREAD_FILE: &str = "thread.json";

const TITLE_BYTES: usize = 200;

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

pub(crate) fn download_dir_in(home: &Path, user_dirs: Option<&str>) -> PathBuf {
    user_dirs
        .and_then(|text| {
            text.lines()
                .rev()
                .find_map(|line| line.trim().strip_prefix("XDG_DOWNLOAD_DIR="))
        })
        .and_then(|value| {
            let quoted = value
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
                .unwrap_or(value);
            match quoted.strip_prefix("$HOME") {
                Some(rest) => Some(home.join(unescape(rest).trim_start_matches('/'))),
                None => {
                    let path = unescape(quoted);
                    path.starts_with('/').then(|| PathBuf::from(path))
                }
            }
        })
        .unwrap_or_else(|| home.join("Downloads"))
}

fn unescape(text: &str) -> String {
    let mut unescaped = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match chars.peek() {
            Some(&next) if c == '\\' && matches!(next, '"' | '\\' | '$' | '`') => {
                unescaped.push(next);
                chars.next();
            }
            _ => unescaped.push(c),
        }
    }
    unescaped
}

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
    if let Err(error) = std::fs::create_dir(&temporary) {
        let _ = std::fs::remove_dir(&folder);
        return Err(format!("could not create {}: {error}", temporary.display()));
    }
    let result = write(&temporary).and_then(|()| {
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
