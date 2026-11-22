use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::config::{failed, now_ms, sql};
use crate::{ModelChoice, Permissions};

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS agent (
    name TEXT PRIMARY KEY, root TEXT NOT NULL, model TEXT NOT NULL, permissions TEXT NOT NULL,
    instructions TEXT NOT NULL, thread TEXT NOT NULL, previous TEXT,
    generation INTEGER NOT NULL, created_ms INTEGER NOT NULL);
";

const MAX_NAME_CHARS: usize = 40;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct NewAgent {
    pub name: String,
    pub root: String,
    pub model: ModelChoice,
    pub permissions: Permissions,
    pub instructions: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    pub name: String,
    pub root: String,
    pub model: ModelChoice,
    pub permissions: Permissions,
    pub instructions: String,
    pub memory: PathBuf,
    pub thread: String,
    pub previous: Option<String>,
    pub generation: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct NewRoutine {
    pub title: String,
    pub prompt: String,
    pub schedule: crate::thread::Schedule,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AddedRoutine {
    pub id: String,
    pub webhook: Option<String>,
}

pub(crate) fn memory(home: &Path, name: &str) -> PathBuf {
    home.join("agents").join(name).join("memory")
}

fn check_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && name.len() <= MAX_NAME_CHARS;
    if valid {
        Ok(())
    } else {
        Err(format!(
            "An agent's name has 1 to {MAX_NAME_CHARS} lowercase letters, digits or dashes and starts with a letter: {name:?}"
        ))
    }
}

pub(crate) fn create(
    db: &Connection,
    home: &Path,
    new: &NewAgent,
    thread: &str,
) -> Result<Agent, String> {
    check_name(&new.name)?;
    if get(db, home, &new.name)?.is_some() {
        return Err(format!("An agent named {} already exists.", new.name));
    }
    let memory = memory(home, &new.name);
    crate::service::create_private_dir(&memory)?;
    db.execute(
        "INSERT INTO agent (name, root, model, permissions, instructions, thread, previous,
         generation, created_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 0, ?7)",
        params![
            new.name,
            new.root,
            to_json(&new.model)?,
            to_json(&new.permissions)?,
            new.instructions,
            thread,
            sql(now_ms())
        ],
    )
    .map_err(failed)?;
    get(db, home, &new.name)?.ok_or_else(|| format!("Agent {} was not stored.", new.name))
}

pub(crate) fn list(db: &Connection, home: &Path) -> Result<Vec<Agent>, String> {
    let mut query = db
        .prepare(
            "SELECT name, root, model, permissions, instructions, thread, previous, generation
             FROM agent ORDER BY name",
        )
        .map_err(failed)?;
    let rows = query
        .query_map([], |row| row_of(row, home))
        .map_err(failed)?;
    rows.map(|row| row.map_err(failed)?).collect()
}

pub(crate) fn get(db: &Connection, home: &Path, name: &str) -> Result<Option<Agent>, String> {
    db.query_row(
        "SELECT name, root, model, permissions, instructions, thread, previous, generation
         FROM agent WHERE name = ?1",
        [name],
        |row| row_of(row, home),
    )
    .optional()
    .map_err(failed)?
    .transpose()
}

pub(crate) fn advance(db: &Connection, name: &str, next: &str) -> Result<(), String> {
    let changed = db
        .execute(
            "UPDATE agent SET previous = thread, thread = ?2, generation = generation + 1
             WHERE name = ?1",
            params![name, next],
        )
        .map_err(failed)?;
    found(changed, name)
}

pub(crate) fn settle(db: &Connection, name: &str) -> Result<(), String> {
    let changed = db
        .execute("UPDATE agent SET previous = NULL WHERE name = ?1", [name])
        .map_err(failed)?;
    found(changed, name)
}

pub(crate) fn remove(db: &Connection, name: &str) -> Result<(), String> {
    let changed = db
        .execute("DELETE FROM agent WHERE name = ?1", [name])
        .map_err(failed)?;
    found(changed, name)
}

pub(crate) fn opening(agent: &Agent, handoff: Option<&str>) -> String {
    let mut text = format!(
        "You are {name}, a persistent agent. Your instructions:\n{instructions}\n\n\
         Your memory folder is {memory}. Read your notes there whenever you wake up, and write \
         down there whatever you need to remember next time: when this conversation grows long, \
         a fresh one replaces it.",
        name = agent.name,
        instructions = agent.instructions,
        memory = agent.memory.display(),
    );
    if let Some(handoff) = handoff {
        text.push_str("\n\n");
        text.push_str(handoff);
    }
    text
}

pub(crate) fn full(context: Option<crate::Context>) -> bool {
    context.is_some_and(|context| {
        context.window > 0
            && context
                .tokens
                .is_some_and(|tokens| tokens.saturating_mul(5) >= context.window.saturating_mul(4))
    })
}

pub(crate) fn message_id(name: &str, generation: u64) -> String {
    format!("agent-{name}-{generation}")
}

fn found(changed: usize, name: &str) -> Result<(), String> {
    if changed == 0 {
        Err(format!("No agent is named {name}."))
    } else {
        Ok(())
    }
}

fn to_json(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| e.to_string())
}

fn row_of(row: &rusqlite::Row<'_>, home: &Path) -> rusqlite::Result<Result<Agent, String>> {
    let name: String = row.get(0)?;
    let model: String = row.get(2)?;
    let permissions: String = row.get(3)?;
    let generation: i64 = row.get(7)?;
    Ok((|| {
        Ok(Agent {
            memory: memory(home, &name),
            root: row.get(1).map_err(failed)?,
            model: serde_json::from_str(&model).map_err(|e| e.to_string())?,
            permissions: serde_json::from_str(&permissions).map_err(|e| e.to_string())?,
            instructions: row.get(4).map_err(failed)?,
            thread: row.get(5).map_err(failed)?,
            previous: row.get(6).map_err(failed)?,
            generation: u64::try_from(generation).map_err(|e| e.to_string())?,
            name,
        })
    })())
}
