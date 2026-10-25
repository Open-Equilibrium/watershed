//! pi's stored session files, read only as far as a listing needs (ADR-0203).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::Value;

use crate::pi::content_text;
use crate::{Frame, LineFramer};

/// Bytes read from the start of a session file to find its title.
const START_BYTES: usize = 256 * 1024;

/// Characters of a first prompt kept as a title.
const TITLE_CHARS: usize = 120;

/// A session pi stored, as a list of recent sessions shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSession {
    /// The session file.
    pub path: PathBuf,
    /// Its name, or else the first line of its first prompt.
    pub title: String,
    /// When pi last wrote it, in Unix milliseconds.
    pub modified_ms: u64,
}

/// Up to `limit` sessions in pi's session directory `dir` that started in `project`, newest
/// first, without `exclude`. Only the start of each file is read, and only until `limit`
/// sessions are found.
pub fn recent_sessions(
    dir: &Path,
    project: &Path,
    exclude: Option<&Path>,
    limit: usize,
) -> std::io::Result<Vec<StoredSession>> {
    let mut files: Vec<(u64, PathBuf)> = std::fs::read_dir(dir)?
        .flatten()
        // A pipe or device would block the read; pi writes plain files.
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .filter(|path| Some(path.as_path()) != exclude)
        .filter_map(|path| {
            let modified = path.metadata().ok()?.modified().ok()?;
            let millis = modified.duration_since(UNIX_EPOCH).ok()?.as_millis();
            Some((u64::try_from(millis).ok()?, path))
        })
        .collect();
    files.sort_by_key(|file| std::cmp::Reverse(file.0));
    let project = canonical(project);
    Ok(files
        .into_iter()
        .filter_map(|(modified_ms, path)| {
            let title = title(&path, &project)?;
            Some(StoredSession {
                path,
                title,
                modified_ms,
            })
        })
        .take(limit)
        .collect())
}

/// The title of a session file started in `project`: its latest name, else its first prompt.
fn title(path: &Path, project: &Path) -> Option<String> {
    let mut start = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(START_BYTES as u64)
        .read_to_end(&mut start)
        .ok()?;
    let mut lines = LineFramer::new(START_BYTES)
        .push(&start)
        .into_iter()
        .filter_map(|frame| match frame {
            Frame::Record(line) => serde_json::from_slice::<Value>(&line).ok(),
            Frame::Oversized { .. } => None,
        });
    let header = lines.next()?;
    let cwd = header.get("cwd").and_then(Value::as_str)?;
    if header.get("type").and_then(Value::as_str) != Some("session")
        || canonical(Path::new(cwd)) != project
    {
        return None;
    }
    let (mut name, mut prompt) = (None, None);
    for entry in lines {
        match entry.get("type").and_then(Value::as_str) {
            Some("session_info") => {
                if let Some(given) = entry.get("name").and_then(Value::as_str) {
                    name = Some(given.to_owned());
                }
            }
            Some("message")
                if prompt.is_none()
                    && entry.pointer("/message/role").and_then(Value::as_str) == Some("user") =>
            {
                let text = content_text(entry.pointer("/message/content"));
                let line = text.lines().next().unwrap_or_default().trim();
                prompt = Some(line.chars().take(TITLE_CHARS).collect());
            }
            _ => {}
        }
    }
    name.or(prompt).filter(|title: &String| !title.is_empty())
}

/// The path with links resolved, or as given when it does not exist here.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}
