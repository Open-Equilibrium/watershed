use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{failed, now_ms, sql};
use crate::{ModelChoice, Permissions};

pub(crate) const SCHEMA: &str = "
DROP TABLE IF EXISTS agent;
CREATE TABLE IF NOT EXISTS agent_profile (
    id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, named INTEGER NOT NULL, root TEXT NOT NULL,
    model TEXT NOT NULL, permissions TEXT NOT NULL, request TEXT, thread TEXT NOT NULL,
    previous TEXT, generation INTEGER NOT NULL, seen_instructions TEXT NOT NULL,
    seen_routines TEXT NOT NULL, created_ms INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS agent_thread (
    agent TEXT NOT NULL, thread TEXT NOT NULL, generation INTEGER NOT NULL,
    PRIMARY KEY (agent, generation));
CREATE TABLE IF NOT EXISTS agent_change (
    id INTEGER PRIMARY KEY AUTOINCREMENT, agent TEXT NOT NULL, thread TEXT NOT NULL,
    turn INTEGER, kind TEXT NOT NULL, before TEXT, after TEXT, created_ms INTEGER NOT NULL,
    undone INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS agent_mention (
    id INTEGER PRIMARY KEY AUTOINCREMENT, agent TEXT NOT NULL, source TEXT NOT NULL,
    thread TEXT NOT NULL, message TEXT NOT NULL, created_ms INTEGER NOT NULL);
";

const MAX_NAME_CHARS: usize = 40;
const INSTRUCTIONS: &str = "instructions.md";
pub(crate) const ROUTINE_KEYS: [&str; 10] = [
    "id",
    "title",
    "prompt",
    "enabled",
    "schedule",
    "projectId",
    "workspaceStrategy",
    "modelSelection",
    "runtimeMode",
    "interactionMode",
];
const COMPARED_KEYS: [&str; 5] = ["id", "title", "prompt", "enabled", "schedule"];
const PERSON_WRITES: &str = "\n\nThe person writes:\n";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct NewAgent {
    pub name: Option<String>,
    pub root: String,
    pub model: ModelChoice,
    pub permissions: Permissions,
    pub instructions: String,
    pub request: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub named: bool,
    pub root: String,
    pub model: ModelChoice,
    pub permissions: Permissions,
    pub memory: PathBuf,
    pub request: Option<String>,
    pub thread: String,
    pub previous: Option<String>,
    pub generation: u64,
    #[serde(default)]
    pub next_run_ms: Option<u64>,
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

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentChangeKind {
    Name,
    Instructions,
    RoutineAdded,
    RoutineRemoved,
    RoutineChanged,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AgentChange {
    pub id: i64,
    pub thread: String,
    pub turn: Option<u64>,
    pub kind: AgentChangeKind,
    pub before: Option<String>,
    pub after: Option<String>,
    pub routine: Option<crate::Automation>,
    pub created_ms: u64,
    pub undone: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    pub agent: String,
    pub name: String,
    pub thread: String,
    pub message: String,
    pub created_ms: u64,
    pub root: String,
    pub answered: bool,
    pub answer: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Snapshot {
    pub instructions: String,
    pub routines: Vec<Value>,
}

pub(crate) fn memory(home: &Path, id: &str) -> PathBuf {
    home.join("agents").join(id).join("memory")
}

pub(crate) fn instructions_file(agent: &Agent) -> PathBuf {
    agent.memory.join(INSTRUCTIONS)
}

pub(crate) fn instructions(agent: &Agent) -> Result<String, String> {
    let path = instructions_file(agent);
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(format!("Could not read {}: {error}", path.display())),
    }
}

pub(crate) fn write_instructions(agent: &Agent, text: &str) -> Result<(), String> {
    crate::service::write_private(&instructions_file(agent), text)
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

pub(crate) fn heading(instructions: &str) -> Option<&str> {
    let name = instructions.lines().next()?.strip_prefix("# ")?.trim();
    check_name(name).ok().map(|()| name)
}

pub(crate) fn with_heading(instructions: &str, name: &str) -> String {
    let body = match instructions.split_once('\n') {
        Some((first, rest)) if first.starts_with("# ") => rest,
        None if instructions.starts_with("# ") => "",
        _ => instructions,
    };
    format!("# {name}\n{}", body)
}

pub(crate) fn create(
    db: &Connection,
    home: &Path,
    new: &NewAgent,
    thread: &str,
) -> Result<Agent, String> {
    let id = crate::t3::fresh_id("agent");
    let (name, named) = match &new.name {
        Some(name) => (name.clone(), true),
        None => (id.clone(), false),
    };
    check_name(&name)?;
    if get(db, home, &name)?.is_some() {
        return Err(format!("An agent named {name} already exists."));
    }
    let memory = memory(home, &id);
    crate::service::create_private_dir(&memory)?;
    let text = if named {
        with_heading(&format!("\n{}\n", new.instructions.trim()), &name)
    } else {
        String::new()
    };
    crate::service::write_private(&memory.join(INSTRUCTIONS), &text)?;
    db.execute(
        "INSERT INTO agent_profile (id, name, named, root, model, permissions, request, thread,
         previous, generation, seen_instructions, seen_routines, created_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, 0, ?9, '[]', ?10)",
        params![
            id,
            name,
            named,
            new.root,
            to_json(&new.model)?,
            to_json(&new.permissions)?,
            new.request,
            thread,
            text,
            sql(now_ms())
        ],
    )
    .map_err(failed)?;
    db.execute(
        "INSERT INTO agent_thread (agent, thread, generation) VALUES (?1, ?2, 0)",
        params![id, thread],
    )
    .map_err(failed)?;
    get(db, home, &id)?.ok_or_else(|| format!("Agent {name} was not stored."))
}

const COLUMNS: &str =
    "id, name, named, root, model, permissions, request, thread, previous, generation";

pub(crate) fn list(db: &Connection, home: &Path) -> Result<Vec<Agent>, String> {
    let mut query = db
        .prepare(&format!(
            "SELECT {COLUMNS} FROM agent_profile ORDER BY name"
        ))
        .map_err(failed)?;
    let rows = query
        .query_map([], |row| row_of(row, home))
        .map_err(failed)?;
    rows.map(|row| row.map_err(failed)?).collect()
}

pub(crate) fn get(db: &Connection, home: &Path, key: &str) -> Result<Option<Agent>, String> {
    db.query_row(
        &format!("SELECT {COLUMNS} FROM agent_profile WHERE id = ?1 OR name = ?1"),
        [key],
        |row| row_of(row, home),
    )
    .optional()
    .map_err(failed)?
    .transpose()
}

pub(crate) fn advance(db: &Connection, id: &str, next: &str) -> Result<(), String> {
    let changed = db
        .execute(
            "UPDATE agent_profile SET previous = thread, thread = ?2, generation = generation + 1
             WHERE id = ?1",
            params![id, next],
        )
        .map_err(failed)?;
    found(changed, id)?;
    db.execute(
        "INSERT INTO agent_thread (agent, thread, generation)
         SELECT id, thread, generation FROM agent_profile WHERE id = ?1",
        [id],
    )
    .map_err(failed)
    .map(|_| ())
}

pub(crate) fn settle(db: &Connection, id: &str) -> Result<(), String> {
    let changed = db
        .execute(
            "UPDATE agent_profile SET previous = NULL WHERE id = ?1",
            [id],
        )
        .map_err(failed)?;
    found(changed, id)
}

pub(crate) fn rename(db: &Connection, home: &Path, id: &str, name: &str) -> Result<(), String> {
    check_name(name)?;
    if get(db, home, name)?.is_some_and(|other| other.id != id) {
        return Err(format!("An agent named {name} already exists."));
    }
    let changed = db
        .execute(
            "UPDATE agent_profile SET name = ?2, named = 1 WHERE id = ?1",
            params![id, name],
        )
        .map_err(failed)?;
    found(changed, id)
}

pub(crate) fn set_model(
    db: &Connection,
    id: &str,
    model: &ModelChoice,
    permissions: Permissions,
) -> Result<(), String> {
    let changed = db
        .execute(
            "UPDATE agent_profile SET model = ?2, permissions = ?3 WHERE id = ?1",
            params![id, to_json(model)?, to_json(&permissions)?],
        )
        .map_err(failed)?;
    found(changed, id)
}

pub(crate) fn threads(db: &Connection, id: &str) -> Result<Vec<String>, String> {
    let mut query = db
        .prepare("SELECT thread FROM agent_thread WHERE agent = ?1 ORDER BY generation DESC")
        .map_err(failed)?;
    let rows = query.query_map([id], |row| row.get(0)).map_err(failed)?;
    rows.collect::<Result<_, _>>().map_err(failed)
}

pub(crate) fn remove(db: &Connection, id: &str) -> Result<(), String> {
    let changed = db
        .execute("DELETE FROM agent_profile WHERE id = ?1", [id])
        .map_err(failed)?;
    found(changed, id)?;
    for table in ["agent_thread", "agent_change", "agent_mention"] {
        db.execute(&format!("DELETE FROM {table} WHERE agent = ?1"), [id])
            .map_err(failed)?;
    }
    Ok(())
}

pub(crate) fn seen(db: &Connection, id: &str) -> Result<Snapshot, String> {
    let (instructions, routines): (String, String) = db
        .query_row(
            "SELECT seen_instructions, seen_routines FROM agent_profile WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(failed)?;
    Ok(Snapshot {
        instructions,
        routines: serde_json::from_str(&routines).map_err(|e| e.to_string())?,
    })
}

pub(crate) fn set_seen(db: &Connection, id: &str, seen: &Snapshot) -> Result<(), String> {
    let changed = db
        .execute(
            "UPDATE agent_profile SET seen_instructions = ?2, seen_routines = ?3 WHERE id = ?1",
            params![id, seen.instructions, to_json(&seen.routines)?],
        )
        .map_err(failed)?;
    found(changed, id)
}

pub(crate) fn routine_copy(task: &Value) -> Value {
    let mut copy = serde_json::json!({});
    for key in ROUTINE_KEYS {
        copy[key] = task[key].clone();
    }
    copy
}

fn compared(task: &Value) -> Vec<&Value> {
    COMPARED_KEYS.iter().map(|key| &task[*key]).collect()
}

pub(crate) fn differences(
    seen: &Snapshot,
    now: &Snapshot,
) -> Vec<(AgentChangeKind, Option<String>, Option<String>)> {
    let mut found = Vec::new();
    let named = |text: &str| heading(text).map(str::to_owned);
    if named(&seen.instructions) != named(&now.instructions)
        && let Some(after) = named(&now.instructions)
    {
        found.push((
            AgentChangeKind::Name,
            named(&seen.instructions),
            Some(after),
        ));
    }
    if seen.instructions != now.instructions {
        found.push((
            AgentChangeKind::Instructions,
            Some(seen.instructions.clone()),
            Some(now.instructions.clone()),
        ));
    }
    let find = |list: &[Value], id: &Value| list.iter().find(|task| &task["id"] == id).cloned();
    for task in &now.routines {
        match find(&seen.routines, &task["id"]) {
            None => found.push((AgentChangeKind::RoutineAdded, None, Some(task.to_string()))),
            Some(before) if compared(&before) != compared(task) => found.push((
                AgentChangeKind::RoutineChanged,
                Some(before.to_string()),
                Some(task.to_string()),
            )),
            Some(_) => {}
        }
    }
    for task in &seen.routines {
        if find(&now.routines, &task["id"]).is_none() {
            found.push((
                AgentChangeKind::RoutineRemoved,
                Some(task.to_string()),
                None,
            ));
        }
    }
    found
}

pub(crate) fn record(
    db: &Connection,
    id: &str,
    thread: &str,
    turn: Option<u64>,
    (kind, before, after): (AgentChangeKind, Option<String>, Option<String>),
) -> Result<(), String> {
    db.execute(
        "INSERT INTO agent_change (agent, thread, turn, kind, before, after, created_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id,
            thread,
            turn.map(sql),
            to_json(&kind)?,
            before,
            after,
            sql(now_ms())
        ],
    )
    .map_err(failed)
    .map(|_| ())
}

pub(crate) fn changes(db: &Connection, id: &str) -> Result<Vec<AgentChange>, String> {
    let mut query = db
        .prepare(
            "SELECT id, thread, turn, kind, before, after, created_ms, undone
             FROM agent_change WHERE agent = ?1 ORDER BY id",
        )
        .map_err(failed)?;
    let rows = query.query_map([id], change_of).map_err(failed)?;
    rows.map(|row| row.map_err(failed)?).collect()
}

pub(crate) fn change(db: &Connection, id: &str, change: i64) -> Result<AgentChange, String> {
    db.query_row(
        "SELECT id, thread, turn, kind, before, after, created_ms, undone
         FROM agent_change WHERE agent = ?1 AND id = ?2",
        params![id, change],
        change_of,
    )
    .optional()
    .map_err(failed)?
    .ok_or_else(|| format!("The agent has no change {change}."))?
}

pub(crate) fn mark_undone(db: &Connection, change: i64) -> Result<(), String> {
    db.execute("UPDATE agent_change SET undone = 1 WHERE id = ?1", [change])
        .map_err(failed)
        .map(|_| ())
}

pub(crate) fn add_mention(
    db: &Connection,
    agent: &Agent,
    source: &str,
    message: &str,
) -> Result<(), String> {
    db.execute(
        "INSERT INTO agent_mention (agent, source, thread, message, created_ms)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![agent.id, source, agent.thread, message, sql(now_ms())],
    )
    .map_err(failed)
    .map(|_| ())
}

pub(crate) fn mentions(db: &Connection, source: &str) -> Result<Vec<Mention>, String> {
    let mut query = db
        .prepare(
            "SELECT m.agent, p.name, m.thread, m.message, m.created_ms, p.root
             FROM agent_mention m JOIN agent_profile p ON p.id = m.agent
             WHERE m.source = ?1 ORDER BY m.id",
        )
        .map_err(failed)?;
    let rows = query
        .query_map([source], |row| {
            let created: i64 = row.get(4)?;
            Ok(Mention {
                agent: row.get(0)?,
                name: row.get(1)?,
                thread: row.get(2)?,
                message: row.get(3)?,
                created_ms: u64::try_from(created).unwrap_or_default(),
                root: row.get(5)?,
                answered: false,
                answer: String::new(),
            })
        })
        .map_err(failed)?;
    rows.collect::<Result<_, _>>().map_err(failed)
}

pub(crate) fn opening(agent: &Agent, instructions: &str, handoff: Option<&str>) -> String {
    let memory = agent.memory.display();
    let mut text = if agent.named {
        format!(
            "You are {name}, a persistent agent. Your instructions are in {memory}/{INSTRUCTIONS}; \
             its first line, `# {name}`, is your name. They read now:\n{instructions}",
            name = agent.name,
        )
    } else {
        format!(
            "You are a new persistent agent without a name yet. Choose a short name for the job \
             below, in lowercase letters, digits and dashes, and write your instructions to \
             {memory}/{INSTRUCTIONS}, starting with the line `# <name>`."
        )
    };
    text.push_str(&format!(
        "\n\nYour memory folder is {memory}. Read your instructions and notes there whenever you \
         wake up, and write down there whatever you need to remember next time: when this \
         conversation grows long, a fresh one replaces it. When the person asks you to work \
         differently, change {INSTRUCTIONS}. For recurring or scheduled work, use your \
         schedule_task tool, which binds the routine to this thread."
    ));
    if let Some(handoff) = handoff {
        text.push_str("\n\n");
        text.push_str(handoff);
    } else if let Some(request) = &agent.request {
        text.push_str(PERSON_WRITES);
        text.push_str(request);
    }
    text
}

pub(crate) fn spoken(opening: &str) -> Option<&str> {
    opening.split_once(PERSON_WRITES).map(|(_, said)| said)
}

pub(crate) fn full(context: Option<crate::Context>) -> bool {
    context.is_some_and(|context| {
        context.window > 0
            && context
                .tokens
                .is_some_and(|tokens| tokens.saturating_mul(5) >= context.window.saturating_mul(4))
    })
}

pub fn message_id(id: &str, generation: u64) -> String {
    format!("{id}-{generation}")
}

pub(crate) fn opening_generation(message: &str) -> Option<u64> {
    let (id, generation) = message.strip_prefix("agent-")?.rsplit_once('-')?;
    (id.len() == 24 && id.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| generation.parse().ok())
        .flatten()
}

fn found(changed: usize, id: &str) -> Result<(), String> {
    if changed == 0 {
        Err(format!("No agent is named {id}."))
    } else {
        Ok(())
    }
}

fn to_json(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| e.to_string())
}

fn change_of(row: &rusqlite::Row<'_>) -> rusqlite::Result<Result<AgentChange, String>> {
    let kind: String = row.get(3)?;
    let before: Option<String> = row.get(4)?;
    let after: Option<String> = row.get(5)?;
    let created: i64 = row.get(6)?;
    let undone: bool = row.get(7)?;
    let id: i64 = row.get(0)?;
    let thread: String = row.get(1)?;
    let turn: Option<i64> = row.get(2)?;
    Ok((|| {
        let kind: AgentChangeKind = serde_json::from_str(&kind).map_err(|e| e.to_string())?;
        let routine = match kind {
            AgentChangeKind::RoutineAdded | AgentChangeKind::RoutineChanged => after.as_deref(),
            AgentChangeKind::RoutineRemoved => before.as_deref(),
            AgentChangeKind::Name | AgentChangeKind::Instructions => None,
        }
        .and_then(|task| serde_json::from_str::<Value>(task).ok())
        .and_then(|task| crate::thread::routine(&task));
        Ok(AgentChange {
            id,
            thread,
            turn: turn.and_then(|turn| u64::try_from(turn).ok()),
            kind,
            before,
            after,
            routine,
            created_ms: u64::try_from(created).unwrap_or_default(),
            undone,
        })
    })())
}

fn row_of(row: &rusqlite::Row<'_>, home: &Path) -> rusqlite::Result<Result<Agent, String>> {
    let id: String = row.get(0)?;
    let model: String = row.get(4)?;
    let permissions: String = row.get(5)?;
    let generation: i64 = row.get(9)?;
    Ok((|| {
        Ok(Agent {
            memory: memory(home, &id),
            name: row.get(1).map_err(failed)?,
            named: row.get(2).map_err(failed)?,
            root: row.get(3).map_err(failed)?,
            model: serde_json::from_str(&model).map_err(|e| e.to_string())?,
            permissions: serde_json::from_str(&permissions).map_err(|e| e.to_string())?,
            request: row.get(6).map_err(failed)?,
            thread: row.get(7).map_err(failed)?,
            previous: row.get(8).map_err(failed)?,
            generation: u64::try_from(generation).map_err(|e| e.to_string())?,
            next_run_ms: None,
            id,
        })
    })())
}
