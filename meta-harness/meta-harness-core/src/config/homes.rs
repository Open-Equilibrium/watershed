//! The homes Meta-Harness owns for Claude Code and Codex: it writes the current revision's blocks
//! into each harness's documented files there, and T3 starts each harness with its home
//! (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`). The person's own `~/.claude` and `~/.codex` stay as they
//! are, while both harnesses keep the person's login (ADR-0298).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Blocks, McpServer};

/// The skills Meta-Harness wrote into a home, so it removes only its own.
const MANAGED: &str = ".meta-harness-skills.json";

/// Claude Code keys its login by this variable's folder, else by `CLAUDE_CONFIG_DIR`'s, so set to
/// the person's own folder (empty for the default) it keeps their login in the new home. It is
/// not a published setting (SECURITY.md).
const CLAUDE_LOGIN: &str = "CLAUDE_SECURESTORAGE_CONFIG_DIR";

/// Codex keeps its login in these files, follows a link for each and locks with the second.
const CODEX_LOGIN: [&str; 2] = ["auth.json", "auth.lock"];

/// The two harness homes under Meta-Harness's data folder.
pub(crate) struct Homes {
    /// Claude Code's `CLAUDE_CONFIG_DIR`.
    pub claude: PathBuf,
    /// Codex's `CODEX_HOME`.
    pub codex: PathBuf,
}

impl Homes {
    /// The homes inside Meta-Harness's data folder `home`.
    pub fn under(home: &Path) -> Self {
        Self {
            claude: home.join("harness").join("claude"),
            codex: home.join("harness").join("codex"),
        }
    }

    /// Claude Code's MCP file, which T3 passes with `--mcp-config`.
    pub fn claude_mcp(&self) -> PathBuf {
        self.claude.join("mcp.json")
    }

    /// Writes `blocks` into both homes and links Codex's login to the person's Codex home
    /// `person_codex`.
    pub fn write(&self, blocks: &Blocks, person_codex: &Path) -> Result<(), String> {
        let instructions = (!blocks.instructions.is_empty()).then(|| {
            let texts: Vec<&str> = blocks.instructions.values().map(|t| t.trim_end()).collect();
            format!("{}\n", texts.join("\n\n"))
        });
        for (home, file) in [(&self.claude, "CLAUDE.md"), (&self.codex, "AGENTS.md")] {
            real_folder(home)?;
            match &instructions {
                Some(text) => write(&home.join(file), text)?,
                None => remove(&home.join(file))?,
            }
            write_skills(home, &blocks.skills)?;
        }
        write(&self.claude_mcp(), &claude_mcp(&blocks.mcp_servers))?;
        write(
            &self.codex.join("config.toml"),
            &codex_config(&blocks.mcp_servers),
        )?;
        for name in CODEX_LOGIN {
            link(&self.codex.join(name), &person_codex.join(name))?;
        }
        Ok(())
    }
}

/// T3's provider instances `stored` with Claude Code and Codex pointed at `homes`, keeping every
/// other instance and setting. `claude_dir` is the person's own `CLAUDE_CONFIG_DIR`, if any.
pub(crate) fn provider_instances(
    stored: &Value,
    homes: &Homes,
    claude_dir: Option<&str>,
) -> Result<Value, String> {
    let mcp = homes.claude_mcp().to_string_lossy().into_owned();
    // T3 splits launch arguments like a shell; a quote or backslash in the path would split it.
    if mcp.contains(['"', '\\']) {
        return Err(format!(
            "Meta-Harness's data folder {} holds a quote or backslash, which T3 cannot pass to \
             Claude Code.",
            homes.claude.display()
        ));
    }
    let mut instances = stored.as_object().cloned().unwrap_or_default();
    instance(&mut instances, "codex")["config"]["homePath"] = json!(homes.codex);
    let claude = instance(&mut instances, "claudeAgent");
    claude["config"]["homePath"] = json!(homes.claude);
    claude["config"]["launchArgs"] = json!(format!("--mcp-config \"{mcp}\""));
    let mut environment: Vec<Value> = claude["environment"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    environment.retain(|v| v["name"] != CLAUDE_LOGIN);
    environment.push(
        json!({"name": CLAUDE_LOGIN, "value": claude_dir.unwrap_or(""),
                            "sensitive": false}),
    );
    claude["environment"] = Value::Array(environment);
    Ok(Value::Object(instances))
}

/// T3's instance `id`, created for its driver of the same name when missing.
fn instance<'a>(instances: &'a mut serde_json::Map<String, Value>, id: &str) -> &'a mut Value {
    let entry = instances
        .entry(id.to_owned())
        .or_insert_with(|| json!({"driver": id}));
    if !entry["config"].is_object() {
        entry["config"] = json!({});
    }
    entry
}

fn write_skills(
    home: &Path,
    skills: &BTreeMap<String, BTreeMap<String, String>>,
) -> Result<(), String> {
    let manifest = home.join(MANAGED);
    // The list sits in a folder the harness writes too, so only plain names count.
    let managed: Vec<String> = std::fs::read_to_string(&manifest)
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|name| super::check_name(name).is_ok())
        .collect();
    let root = home.join("skills");
    real_folder(&root)?;
    // Each name is one plain folder name: proposals and the list above were checked.
    for name in managed.iter().chain(skills.keys()) {
        let folder = root.join(name);
        match std::fs::remove_dir_all(&folder) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(failed(&folder, &e)),
            _ => {}
        }
    }
    for (name, files) in skills {
        for (path, text) in files {
            write(&root.join(name).join(path), text)?;
        }
    }
    let names: Vec<&String> = skills.keys().collect();
    write(
        &manifest,
        &serde_json::to_string(&names).expect("names serialize"),
    )
}

fn claude_mcp(servers: &BTreeMap<String, McpServer>) -> String {
    let servers: serde_json::Map<String, Value> = servers
        .iter()
        .map(|(name, server)| {
            let value = match server {
                McpServer::Command { command, args, env } => {
                    json!({"type": "stdio", "command": command, "args": args, "env": env})
                }
                McpServer::Url { url } => json!({"type": "http", "url": url}),
            };
            (name.clone(), value)
        })
        .collect();
    json!({"mcpServers": servers}).to_string()
}

fn codex_config(servers: &BTreeMap<String, McpServer>) -> String {
    let mut text = String::from(
        "# Written by Meta-Harness from its central configuration; edits here are replaced.\n",
    );
    // Names and variable names were checked to be TOML bare keys.
    for (name, server) in servers {
        text.push_str(&format!("\n[mcp_servers.{name}]\n"));
        match server {
            McpServer::Command { command, args, env } => {
                text.push_str(&format!("command = {}\n", toml_string(command)));
                if !args.is_empty() {
                    let args: Vec<String> = args.iter().map(|a| toml_string(a)).collect();
                    text.push_str(&format!("args = [{}]\n", args.join(", ")));
                }
                if !env.is_empty() {
                    let env: Vec<String> = env
                        .iter()
                        .map(|(k, v)| format!("{k} = {}", toml_string(v)))
                        .collect();
                    text.push_str(&format!("env = {{ {} }}\n", env.join(", ")));
                }
            }
            McpServer::Url { url } => text.push_str(&format!("url = {}\n", toml_string(url))),
        }
    }
    text
}

/// A TOML basic string.
fn toml_string(value: &str) -> String {
    let mut out = String::from('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Makes `path` a real folder. A link in its place is removed, never followed, so nothing outside
/// the home changes.
fn real_folder(path: &Path) -> Result<(), String> {
    if std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
        // A link to a folder is a folder on Windows, which only `remove_dir` takes away.
        std::fs::remove_file(path)
            .or_else(|_| std::fs::remove_dir(path))
            .map_err(|e| failed(path, &e))?;
    }
    std::fs::create_dir_all(path).map_err(|e| failed(path, &e))
}

/// Writes `text` to `path` through a hidden temporary file beside it, so a harness never reads
/// half of it. The temporary file is created new under a random name, so no link or file planted
/// at a predictable name is ever written through. Skill paths have no hidden parts, so the name
/// clashes with none.
fn write(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().expect("a home file has a folder");
    std::fs::create_dir_all(parent).map_err(|e| failed(parent, &e))?;
    let name = path
        .file_name()
        .expect("a home file has a name")
        .to_string_lossy();
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("the operating system provides random bytes");
    let temporary = parent.join(format!(
        ".{name}.{}.meta-harness-tmp",
        crate::util::hex(&bytes)
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| failed(&temporary, &e))?;
    let written = file
        .write_all(text.as_bytes())
        .map_err(|e| failed(&temporary, &e));
    drop(file);
    let result =
        written.and_then(|()| std::fs::rename(&temporary, path).map_err(|e| failed(path, &e)));
    // A failed write or rename leaves the temporary file behind otherwise.
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn remove(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(failed(path, &e)),
        _ => Ok(()),
    }
}

/// Points `path` at `target`, unless a real file is there: a login made in the home stays.
fn link(path: &Path, target: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_symlink() => return Ok(()),
        Ok(_) if std::fs::read_link(path).is_ok_and(|t| t == target) => return Ok(()),
        Ok(_) => remove(path)?,
        Err(_) => {}
    }
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(target, path);
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(target, path);
    linked.map_err(|e| failed(path, &e))
}

fn failed(path: &Path, error: &std::io::Error) -> String {
    format!(
        "Could not write the harness home at {}: {error}",
        path.display()
    )
}
