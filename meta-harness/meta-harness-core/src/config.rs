use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

mod homes;

pub(crate) use homes::{Homes, provider_instances};

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct Blocks {
    #[serde(default)]
    pub instructions: BTreeMap<String, String>,
    #[serde(default)]
    pub skills: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    pub mcp_servers: BTreeMap<String, McpServer>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum McpServer {
    Command {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    Url {
        url: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
    Instruction {
        name: String,
        text: Option<String>,
    },
    Skill {
        name: String,
        files: Option<BTreeMap<String, String>>,
    },
    McpServer {
        name: String,
        server: Option<McpServer>,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Proposed {
    Changes(Vec<Change>),
    Restore(u64),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub number: u64,
    pub summary: String,
    pub proposed: Proposed,
    pub client: String,
    pub at_ms: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Proposed,
    Approved,
    Rejected,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub number: u64,
    pub at_ms: u64,
    pub client: String,
    pub action: Action,
    pub proposal: u64,
    pub revision: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Current {
    pub revision: u64,
    pub blocks: Blocks,
}

pub(crate) fn apply(blocks: &mut Blocks, change: Change) {
    fn set<T>(map: &mut BTreeMap<String, T>, name: String, value: Option<T>) {
        match value {
            Some(value) => map.insert(name, value),
            None => map.remove(&name),
        };
    }
    match change {
        Change::Instruction { name, text } => set(&mut blocks.instructions, name, text),
        Change::Skill { name, files } => set(&mut blocks.skills, name, files),
        Change::McpServer { name, server } => set(&mut blocks.mcp_servers, name, server),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Add,
    Replace,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    pub direction: Direction,
    pub change: Change,
}

pub fn diff_blocks(from: &Blocks, to: &Blocks) -> Vec<Difference> {
    let mut differences = Vec::new();
    diff_map(
        &mut differences,
        &from.instructions,
        &to.instructions,
        |name, text| Change::Instruction { name, text },
    );
    diff_map(&mut differences, &from.skills, &to.skills, |name, files| {
        Change::Skill { name, files }
    });
    diff_map(
        &mut differences,
        &from.mcp_servers,
        &to.mcp_servers,
        |name, server| Change::McpServer { name, server },
    );
    differences
}

fn diff_map<T: Clone + PartialEq>(
    differences: &mut Vec<Difference>,
    from: &BTreeMap<String, T>,
    to: &BTreeMap<String, T>,
    change: impl Fn(String, Option<T>) -> Change,
) {
    for (name, value) in from {
        let direction = match to.get(name) {
            Some(existing) if existing == value => continue,
            Some(_) => Direction::Replace,
            None => Direction::Add,
        };
        differences.push(Difference {
            direction,
            change: change(name.clone(), Some(value.clone())),
        });
    }
    for name in to.keys().filter(|name| !from.contains_key(*name)) {
        differences.push(Difference {
            direction: Direction::Remove,
            change: change(name.clone(), None),
        });
    }
}

const MAX_NAME: usize = 64;
pub(crate) const MAX_SUMMARY: usize = 1000;
const MAX_TEXT: usize = 256 * 1024;
pub(crate) const MAX_ENTRIES: usize = 256;
const MAX_FIELD: usize = 4096;
const MAX_TOTAL: usize = 8 * 1024 * 1024;

pub(crate) fn check_change(change: &Change) -> Result<(), String> {
    match change {
        Change::Instruction { name, text } => {
            check_name(name)?;
            text.as_deref()
                .map_or(Ok(()), |t| check_text("An instruction", t, MAX_TEXT))
        }
        Change::Skill { name, files } => {
            check_name(name)?;
            files.as_ref().map_or(Ok(()), check_skill)
        }
        Change::McpServer { name, server } => {
            check_name(name)?;
            server.as_ref().map_or(Ok(()), check_server)
        }
    }
}

pub(crate) fn check_name(name: &str) -> Result<(), String> {
    let fine = !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if fine {
        Ok(())
    } else {
        Err(format!(
            "The name {name:?} must be 1 to {MAX_NAME} letters, digits, - or _."
        ))
    }
}

pub(crate) fn check_text(what: &str, text: &str, max: usize) -> Result<(), String> {
    if text.len() > max || text.contains('\0') {
        Err(format!(
            "{what} is too large or holds a NUL character (at most {max} bytes)."
        ))
    } else {
        Ok(())
    }
}

fn check_skill(files: &BTreeMap<String, String>) -> Result<(), String> {
    if !files.contains_key("SKILL.md") {
        return Err("A skill needs a SKILL.md.".to_owned());
    }
    if files.len() > MAX_ENTRIES {
        return Err(format!("A skill holds at most {MAX_ENTRIES} files."));
    }
    let folded: std::collections::BTreeSet<String> =
        files.keys().map(|p| p.to_ascii_lowercase()).collect();
    if folded.len() < files.len()
        || folded.iter().any(|p| {
            folded
                .range(format!("{p}/")..)
                .next()
                .is_some_and(|q| q.starts_with(&format!("{p}/")))
        })
    {
        return Err(
            "Two skill paths clash: one names the other's folder, or they differ only in case."
                .to_owned(),
        );
    }
    for (path, text) in files {
        let fine = path.len() <= 255
            && path.split('/').all(|part| {
                !part.is_empty()
                    && !part.starts_with('.')
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            });
        if !fine {
            return Err(format!(
                "The skill path {path:?} must be relative, without empty, hidden or .. parts."
            ));
        }
        check_text("A skill file", text, MAX_TEXT)?;
    }
    Ok(())
}

fn check_server(server: &McpServer) -> Result<(), String> {
    match server {
        McpServer::Command { command, args, env } => {
            if command.is_empty() {
                return Err("An MCP server needs a command.".to_owned());
            }
            if args.len() > MAX_ENTRIES || env.len() > MAX_ENTRIES {
                return Err(format!(
                    "An MCP server has at most {MAX_ENTRIES} arguments and variables."
                ));
            }
            for text in std::iter::once(command).chain(args).chain(env.values()) {
                check_text("An MCP server field", text, MAX_FIELD)?;
            }
            for key in env.keys() {
                let fine = key.len() <= MAX_NAME
                    && key.bytes().next().is_some_and(|b| !b.is_ascii_digit())
                    && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
                if !fine {
                    return Err(format!(
                        "The variable name {key:?} must be letters, digits and _."
                    ));
                }
            }
            Ok(())
        }
        McpServer::Url { url } => {
            check_text("An MCP server address", url, MAX_FIELD)?;
            if url.starts_with("https://") || url.starts_with("http://") {
                Ok(())
            } else {
                Err(format!(
                    "The MCP server address {url:?} must use http or https."
                ))
            }
        }
    }
}

pub(crate) fn check_size(blocks: &Blocks) -> Result<(), String> {
    let entries = [
        blocks.instructions.len(),
        blocks.skills.len(),
        blocks.mcp_servers.len(),
    ];
    if entries.iter().any(|n| *n > MAX_ENTRIES) || json(blocks).len() > MAX_TOTAL {
        return Err(format!(
            "A revision holds at most {MAX_ENTRIES} blocks of each kind and {MAX_TOTAL} bytes."
        ));
    }
    let folded: std::collections::BTreeSet<String> = blocks
        .skills
        .keys()
        .map(|n| n.to_ascii_lowercase())
        .collect();
    if folded.len() < blocks.skills.len() {
        return Err("Two skills differ only in case, so they would share one folder.".to_owned());
    }
    Ok(())
}

pub(crate) fn json(value: &impl Serialize) -> String {
    serde_json::to_string(value).expect("configuration values serialize")
}
