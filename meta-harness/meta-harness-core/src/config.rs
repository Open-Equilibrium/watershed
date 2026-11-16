//! The central configuration (ADR-0225, ADR-0227, ADR-0298): shared building blocks kept in
//! Meta-Harness's SQLite, changed only through proposals that someone on the account approves.
//! Proposals, revisions and the log are append-only; a revert restores an earlier revision as a
//! new one.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

mod homes;

pub(crate) use homes::{Homes, provider_instances};

/// The building blocks of one revision.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct Blocks {
    /// Instructions by name, written into each harness's instruction file in name order.
    #[serde(default)]
    pub instructions: BTreeMap<String, String>,
    /// Skills by name: each a folder of text files with a `SKILL.md`.
    #[serde(default)]
    pub skills: BTreeMap<String, BTreeMap<String, String>>,
    /// MCP servers by name.
    #[serde(default)]
    pub mcp_servers: BTreeMap<String, McpServer>,
}

/// An MCP server a harness starts or reaches.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpServer {
    /// A program the harness starts and speaks to over standard input and output.
    Command {
        /// The program.
        command: String,
        /// Its arguments.
        #[serde(default)]
        args: Vec<String>,
        /// Environment variables it gets.
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    /// A server the harness reaches over HTTP.
    Url {
        /// Its address.
        url: String,
    },
}

/// One change to a block: a value sets it, `None` removes it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    /// An instruction's text.
    Instruction {
        /// Its name.
        name: String,
        /// Its text.
        text: Option<String>,
    },
    /// A skill's files, by path inside its folder.
    Skill {
        /// Its name.
        name: String,
        /// Its files.
        files: Option<BTreeMap<String, String>>,
    },
    /// An MCP server.
    McpServer {
        /// Its name.
        name: String,
        /// The server.
        server: Option<McpServer>,
    },
}

/// What a proposal does once approved.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Proposed {
    /// Applies these changes to the revision current at approval.
    Changes(Vec<Change>),
    /// Makes the blocks of this earlier revision current again.
    Restore(u64),
}

/// A proposal that waits for a decision.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    /// Its number.
    pub number: u64,
    /// What its author says it does.
    pub summary: String,
    /// What it does.
    pub proposed: Proposed,
    /// The client that proposed it, as it named itself.
    pub client: String,
    /// When, in milliseconds since the Unix epoch.
    pub at_ms: u64,
}

/// A step the log records.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// A proposal was made.
    Proposed,
    /// It was approved and became a revision.
    Approved,
    /// It was rejected.
    Rejected,
}

/// One entry of the append-only log.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// Its position, from 1.
    pub number: u64,
    /// When, in milliseconds since the Unix epoch.
    pub at_ms: u64,
    /// The client that took the step, as it named itself; anyone on the account can name any
    /// client, so this records, not proves, who it was (ADR-0298).
    pub client: String,
    /// The step.
    pub action: Action,
    /// The proposal.
    pub proposal: u64,
    /// The revision an approval made.
    pub revision: Option<u64>,
}

/// The current revision.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Current {
    /// Its number; 0 before the first approval.
    pub revision: u64,
    /// Its blocks.
    pub blocks: Blocks,
}

/// Rows never change once written: triggers refuse updates and deletions.
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS revision (number INTEGER PRIMARY KEY, blocks TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS proposal (
    number INTEGER PRIMARY KEY, summary TEXT NOT NULL, proposed TEXT NOT NULL,
    client TEXT NOT NULL, at_ms INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS log (
    number INTEGER PRIMARY KEY AUTOINCREMENT, at_ms INTEGER NOT NULL, client TEXT NOT NULL,
    action TEXT NOT NULL, proposal INTEGER NOT NULL, revision INTEGER);
INSERT OR IGNORE INTO revision (number, blocks) VALUES (0, '{}');
";

const TABLES: [&str; 3] = ["revision", "proposal", "log"];

/// Meta-Harness's configuration store, one SQLite file.
pub(crate) struct Store {
    db: Mutex<Connection>,
}

impl Store {
    /// Opens the store at `path`, creating it when missing.
    pub fn open(path: &Path) -> Result<Self, String> {
        let db = Connection::open(path).map_err(failed)?;
        let mut schema = SCHEMA.to_owned();
        for table in TABLES {
            for verb in ["UPDATE", "DELETE"] {
                schema.push_str(&format!(
                    "CREATE TRIGGER IF NOT EXISTS {table}_no_{verb} BEFORE {verb} ON {table} \
                     BEGIN SELECT RAISE(ABORT, '{table} rows never change'); END;\n"
                ));
            }
        }
        db.execute_batch(&format!("BEGIN; {schema} COMMIT;"))
            .map_err(failed)?;
        Ok(Self { db: Mutex::new(db) })
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The current revision.
    pub fn current(&self) -> Result<Current, String> {
        current(&self.db())
    }

    /// Records a proposal by `client` after checking it, and returns its number.
    pub fn propose(&self, client: &str, summary: &str, proposed: Proposed) -> Result<u64, String> {
        check_text("The summary", summary, MAX_SUMMARY)?;
        let mut db = self.db();
        match &proposed {
            Proposed::Changes(changes) => {
                if changes.len() > MAX_ENTRIES {
                    return Err(format!("A proposal holds at most {MAX_ENTRIES} changes."));
                }
                changes.iter().try_for_each(check_change)?;
            }
            Proposed::Restore(revision) => {
                blocks_of(&db, *revision)?
                    .ok_or_else(|| format!("There is no revision {revision}."))?;
            }
        }
        let tx = db.transaction().map_err(failed)?;
        let at = now_ms();
        tx.execute(
            "INSERT INTO proposal (summary, proposed, client, at_ms) VALUES (?1, ?2, ?3, ?4)",
            params![summary, json(&proposed), client, sql(at)],
        )
        .map_err(failed)?;
        let number = u64::try_from(tx.last_insert_rowid()).map_err(|e| e.to_string())?;
        log(&tx, at, client, Action::Proposed, number, None)?;
        tx.commit().map_err(failed)?;
        Ok(number)
    }

    /// The proposals that wait for a decision, oldest first.
    pub fn pending(&self) -> Result<Vec<Proposal>, String> {
        let db = self.db();
        let mut statement = db
            .prepare(
                "SELECT number, summary, proposed, client, at_ms FROM proposal \
                 WHERE number NOT IN (SELECT proposal FROM log WHERE action != 'proposed') \
                 ORDER BY number",
            )
            .map_err(failed)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(failed)?;
        rows.map(|row| {
            let (number, summary, proposed, client, at_ms) = row.map_err(failed)?;
            Ok(Proposal {
                number: count(number),
                summary,
                proposed: parse(&proposed)?,
                client,
                at_ms: count(at_ms),
            })
        })
        .collect()
    }

    /// Approves proposal `number` for `client`: its changes apply to the current revision, which
    /// makes a new one.
    pub fn approve(&self, client: &str, number: u64) -> Result<Current, String> {
        let mut db = self.db();
        let tx = db.transaction().map_err(failed)?;
        let proposed: Proposed = parse(&undecided(&tx, number)?)?;
        let base = current(&tx)?;
        let blocks = match proposed {
            Proposed::Changes(changes) => {
                let mut blocks = base.blocks;
                for change in changes {
                    apply(&mut blocks, change);
                }
                check_size(&blocks)?;
                blocks
            }
            Proposed::Restore(revision) => {
                blocks_of(&tx, revision)?.ok_or("The revision to restore is gone.")?
            }
        };
        let revision = base.revision + 1;
        tx.execute(
            "INSERT INTO revision (number, blocks) VALUES (?1, ?2)",
            params![sql(revision), json(&blocks)],
        )
        .map_err(failed)?;
        log(
            &tx,
            now_ms(),
            client,
            Action::Approved,
            number,
            Some(revision),
        )?;
        tx.commit().map_err(failed)?;
        Ok(Current { revision, blocks })
    }

    /// Rejects proposal `number` for `client`.
    pub fn reject(&self, client: &str, number: u64) -> Result<(), String> {
        let mut db = self.db();
        let tx = db.transaction().map_err(failed)?;
        undecided(&tx, number)?;
        log(&tx, now_ms(), client, Action::Rejected, number, None)?;
        tx.commit().map_err(failed)
    }

    /// The whole log, oldest first.
    pub fn log(&self) -> Result<Vec<LogEntry>, String> {
        let db = self.db();
        let mut statement = db
            .prepare(
                "SELECT number, at_ms, client, action, proposal, revision FROM log ORDER BY number",
            )
            .map_err(failed)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                ))
            })
            .map_err(failed)?;
        rows.map(|row| {
            let (number, at_ms, client, action, proposal, revision) = row.map_err(failed)?;
            Ok(LogEntry {
                number: count(number),
                at_ms: count(at_ms),
                client,
                action: parse(&format!("\"{action}\""))?,
                proposal: count(proposal),
                revision: revision.map(count),
            })
        })
        .collect()
    }
}

fn current(db: &Connection) -> Result<Current, String> {
    let (revision, blocks): (i64, String) = db
        .query_row(
            "SELECT number, blocks FROM revision ORDER BY number DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(failed)?;
    Ok(Current {
        revision: count(revision),
        blocks: parse(&blocks)?,
    })
}

fn blocks_of(db: &Connection, revision: u64) -> Result<Option<Blocks>, String> {
    db.query_row(
        "SELECT blocks FROM revision WHERE number = ?1",
        [sql(revision)],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(failed)?
    .map(|text| parse(&text))
    .transpose()
}

/// The proposal `number` as stored, while no decision was logged for it.
fn undecided(db: &Connection, number: u64) -> Result<String, String> {
    let proposed: String = db
        .query_row(
            "SELECT proposed FROM proposal WHERE number = ?1",
            [sql(number)],
            |row| row.get(0),
        )
        .optional()
        .map_err(failed)?
        .ok_or_else(|| format!("There is no proposal {number}."))?;
    let decided: bool = db
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM log WHERE proposal = ?1 AND action != 'proposed')",
            [sql(number)],
            |row| row.get(0),
        )
        .map_err(failed)?;
    if decided {
        return Err(format!("Proposal {number} was already decided."));
    }
    Ok(proposed)
}

fn log(
    db: &Connection,
    at: u64,
    client: &str,
    action: Action,
    proposal: u64,
    revision: Option<u64>,
) -> Result<(), String> {
    let action = json(&action);
    db.execute(
        "INSERT INTO log (at_ms, client, action, proposal, revision) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            sql(at),
            client,
            action.trim_matches('"'),
            sql(proposal),
            revision.map(sql)
        ],
    )
    .map(|_| ())
    .map_err(failed)
}

fn apply(blocks: &mut Blocks, change: Change) {
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

/// Bounds that keep a revision small enough to write into every harness home at once.
const MAX_NAME: usize = 64;
const MAX_SUMMARY: usize = 1000;
const MAX_TEXT: usize = 256 * 1024;
const MAX_ENTRIES: usize = 256;
const MAX_FIELD: usize = 4096;
const MAX_TOTAL: usize = 8 * 1024 * 1024;

fn check_change(change: &Change) -> Result<(), String> {
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

/// A block name doubles as a folder, table key and file name in each harness home, so it uses
/// only letters, digits, `-` and `_`.
fn check_name(name: &str) -> Result<(), String> {
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

fn check_text(what: &str, text: &str, max: usize) -> Result<(), String> {
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

fn check_size(blocks: &Blocks) -> Result<(), String> {
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
    Ok(())
}

/// SQLite stores integers signed; Meta-Harness's numbers and times are never negative.
fn sql(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

fn json(value: &impl Serialize) -> String {
    serde_json::to_string(value).expect("configuration values serialize")
}

fn parse<T: for<'a> Deserialize<'a>>(text: &str) -> Result<T, String> {
    serde_json::from_str(text).map_err(|e| format!("The configuration store is damaged: {e}"))
}

fn failed(error: rusqlite::Error) -> String {
    format!("The configuration store failed: {error}")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}
