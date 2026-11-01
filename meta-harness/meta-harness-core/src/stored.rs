//! pi's stored session files, read only as far as a listing needs.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::Value;

use crate::history::first_line;
use crate::pi::content_text;
use crate::{Frame, LineFramer};

/// Bytes read from the start of a session file to find its title.
const START_BYTES: usize = 256 * 1024;

/// The title of a session whose start names it nowhere.
const UNTITLED: &str = "Untitled session";

/// Characters of a first prompt kept as a title.
const TITLE_CHARS: usize = 120;

/// A session pi stored, as a list of recent sessions shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSession {
    /// The session file.
    pub path: PathBuf,
    /// Its name, or else the first line of its first prompt, or else "Untitled session".
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
    let mut files: Vec<(u64, PathBuf)> = session_files(dir)?
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
            // A session named by nothing in the part read still shows, so it can continue.
            let title = head(&path, &project)?
                .title
                .unwrap_or_else(|| UNTITLED.to_owned());
            Some(StoredSession {
                path,
                title,
                modified_ms,
            })
        })
        .take(limit)
        .collect())
}

/// The title of the session stored in `file`, as [`recent_sessions`] gives it; none before the
/// session has a name or a prompt, or for a session of another project.
pub fn session_title(file: &Path, project: &Path) -> Option<String> {
    head(file, &canonical(project))?.title
}

/// The id of the session stored in `file`; none for a file that is no session started in
/// `project`.
pub fn session_id(file: &Path, project: &Path) -> Option<String> {
    Some(head(file, &canonical(project))?.id).filter(|id| !id.is_empty())
}

/// The title of the session with the id `id` in pi's session directory `dir` that started in
/// `project`, as [`recent_sessions`] gives it.
pub fn session_title_of(dir: &Path, project: &Path, id: &str) -> Option<String> {
    let project = canonical(project);
    session_files(dir).ok()?.find_map(|path| {
        let head = head(&path, &project)?;
        (head.id == id).then(|| head.title.unwrap_or_else(|| UNTITLED.to_owned()))
    })
}

/// The titles of the sessions in pi's session directory `dir` that started in `project`, by
/// session id, as [`recent_sessions`] gives them; one pass over the directory.
pub fn session_titles(dir: &Path, project: &Path) -> HashMap<String, String> {
    let project = canonical(project);
    session_files(dir)
        .into_iter()
        .flatten()
        .filter_map(|path| head(&path, &project))
        .map(|head| (head.id, head.title.unwrap_or_else(|| UNTITLED.to_owned())))
        .collect()
}

/// The session files in `dir`.
fn session_files(dir: &Path) -> std::io::Result<impl Iterator<Item = PathBuf>> {
    Ok(std::fs::read_dir(dir)?
        .flatten()
        .map(|entry| entry.path())
        // A pipe or device would block the read; pi writes plain files, perhaps linked.
        .filter(|path| path.metadata().is_ok_and(|metadata| metadata.is_file()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        }))
}

/// What the start of a session file says about it.
struct Head {
    id: String,
    /// Its name or first prompt, if any.
    title: Option<String>,
}

/// The start of a session file; `None` when the file is not a session started in `project`.
fn head(path: &Path, project: &Path) -> Option<Head> {
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
    let id = header
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
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
                // The first line with words, as Liquid names an unsaved session.
                let text = content_text(entry.pointer("/message/content"));
                prompt = Some(first_line(&text, TITLE_CHARS));
            }
            _ => {}
        }
    }
    Some(Head {
        id,
        title: name.or(prompt).filter(|title: &String| !title.is_empty()),
    })
}

/// The path with links resolved, or as given when it does not exist here.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}
