use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::handoff::HANDOFF_BYTES;
use crate::store::{count, failed, sql};
use crate::{ChatItem, ModelChoice, Permissions};

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS moved_thread (
    transfer TEXT PRIMARY KEY, thread TEXT NOT NULL, root TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('preparing', 'handed', 'sealed', 'released')),
    to_host TEXT, target_address TEXT NOT NULL, target_name TEXT NOT NULL,
    target_environment TEXT NOT NULL,
    since_ms INTEGER NOT NULL, queued TEXT NOT NULL DEFAULT '[]', package TEXT);
CREATE UNIQUE INDEX IF NOT EXISTS one_open_move ON moved_thread (thread)
    WHERE state IN ('preparing', 'handed');
CREATE TABLE IF NOT EXISTS arrival (
    transfer TEXT PRIMARY KEY, state TEXT NOT NULL CHECK (state IN ('starting', 'landed', 'refused')),
    root TEXT NOT NULL, thread TEXT NOT NULL, queued TEXT NOT NULL DEFAULT '[]',
    delivered INTEGER NOT NULL DEFAULT 0, at_ms INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS move_record (
    thread TEXT PRIMARY KEY, transfer TEXT NOT NULL UNIQUE, from_host TEXT NOT NULL,
    to_host TEXT NOT NULL, at_ms INTEGER NOT NULL, transcript TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS pause_pending (
    thread TEXT PRIMARY KEY, root TEXT NOT NULL
);
";

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Harness {
    ClaudeCode,
    Codex,
}

impl Harness {
    pub(crate) fn of(model: &ModelChoice) -> Option<Self> {
        Self::of_instance(&model.instance)
    }

    pub(crate) fn of_instance(instance: &str) -> Option<Self> {
        match instance {
            "claudeAgent" => Some(Self::ClaudeCode),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MovePhase {
    Preparing,
    Handed,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Moving {
    pub transfer: String,
    pub phase: MovePhase,
    pub target_address: String,
    pub target_name: String,
    pub target_environment: String,
    pub since_ms: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct MoveCheck {
    pub title: String,
    pub model: Option<ModelChoice>,
    pub harness: Option<Harness>,
    pub running: bool,
    pub uncommitted_files: u32,
    pub branch: Option<String>,
    pub remote_url: Option<String>,
    pub blocked: Option<String>,
    pub moved: bool,
    pub moving: Option<Moving>,
    pub agent: Option<crate::agents::AgentMoveFacts>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MovePackage {
    pub from_host: String,
    pub title: String,
    pub harness: Harness,
    pub model: ModelChoice,
    pub permissions: Permissions,
    pub branch: String,
    pub remote_url: String,
    pub handoff: String,
    pub transcript: Vec<ChatItem>,
    pub transcript_capped: bool,
    pub queued: Vec<String>,
    pub transfer: String,
    pub agent: Option<Box<crate::agents::AgentCarry>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Landing {
    NewWorktree,
    Continue { path: String },
    Attach,
    Blocked { reason: String },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MoveFinish {
    Seal { to_host: String },
    Release,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Arrival {
    Landed { thread: String },
    Starting,
    Refused,
    Unknown,
}

pub(crate) struct Departure<'a> {
    pub transfer: &'a str,
    pub from_host: &'a str,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MoveRecord {
    pub from_host: String,
    pub to_host: String,
    pub at_ms: u64,
    pub transcript: Vec<ChatItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    Moving,
    Sealed,
}

pub(crate) struct Facts {
    pub title: String,
    pub model: Option<ModelChoice>,
    pub permissions: Permissions,
    pub running: bool,
    pub uncommitted_files: u32,
    pub branch: Option<String>,
    pub remote_url: Option<String>,
    pub queued_attachments: bool,
}

pub(crate) fn refusal(state: State) -> &'static str {
    match state {
        State::Moving => {
            "This session is moving to another host and takes no new messages until the move ends."
        }
        State::Sealed => "This session moved to another host and takes no new messages here.",
    }
}

pub(crate) struct Movable {
    pub title: String,
    pub model: ModelChoice,
    pub harness: Harness,
    pub permissions: Permissions,
    pub branch: String,
    pub remote_url: String,
    pub earlier: Option<Vec<ChatItem>>,
}

pub(crate) fn movable(
    facts: &Facts,
    state: Option<State>,
    earlier: Option<Vec<ChatItem>>,
) -> Result<Movable, String> {
    let (Some(model), Some(harness)) = (
        facts.model.clone(),
        facts.model.as_ref().and_then(Harness::of),
    ) else {
        return Err("Only Claude Code and Codex sessions can move.".to_owned());
    };
    if let Some(state) = state {
        return Err(match state {
            State::Moving => "This session is already moving to another host.".to_owned(),
            State::Sealed => "This session has already moved to another host.".to_owned(),
        });
    }
    let Some(branch) = facts.branch.clone() else {
        return Err("This session has no branch to move.".to_owned());
    };
    let Some(remote_url) = facts.remote_url.clone() else {
        return Err("This project has no Git remote that T3 can resolve. A remote given as a folder path does not count, so the session cannot move.".to_owned());
    };
    if carries_credential(&remote_url) {
        return Err("This project's Git remote holds a credential (a password, a token or a query string), so the session cannot move; the credential would travel with it.".to_owned());
    }
    if facts.queued_attachments {
        return Err("A queued message has attachments, which a move does not carry, so the session cannot move yet.".to_owned());
    }
    Ok(Movable {
        title: facts.title.clone(),
        model,
        harness,
        permissions: facts.permissions,
        branch,
        remote_url,
        earlier,
    })
}

pub(crate) fn carries_credential(remote: &str) -> bool {
    if remote.contains('?') || remote_helper(remote) {
        return true;
    }
    let Some((scheme, rest)) = remote.split_once("://") else {
        return false;
    };
    let authority = rest
        .trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or_default();
    let Some((userinfo, _host)) = authority.rsplit_once('@') else {
        return false;
    };
    let web = matches!(
        scheme.to_ascii_lowercase().as_str(),
        "http" | "https" | "ftp" | "ftps"
    );
    userinfo.contains(':') || (web && !userinfo.is_empty())
}

fn remote_helper(remote: &str) -> bool {
    remote.split_once("::").is_some_and(|(transport, _)| {
        !transport.is_empty() && !transport.contains(['/', ':', '@', '['])
    })
}

pub(crate) fn check(facts: &Facts, state: Option<State>) -> MoveCheck {
    MoveCheck {
        title: facts.title.clone(),
        model: facts.model.clone(),
        harness: facts.model.as_ref().and_then(Harness::of),
        running: facts.running,
        uncommitted_files: facts.uncommitted_files,
        branch: facts.branch.clone(),
        remote_url: facts
            .remote_url
            .clone()
            .filter(|remote| !carries_credential(remote)),
        blocked: movable(facts, state, None).err(),
        moved: state == Some(State::Sealed),
        moving: None,
        agent: None,
    }
}

pub(crate) const UNKNOWN_MOVE: &str = "There is no move with this transfer.";

pub(crate) const MOVE_ENDED: &str = "This move already ended, so nothing changed.";

pub(crate) fn state(db: &Connection, thread: &str) -> Result<Option<State>, String> {
    let open: Option<String> = db
        .query_row(
            "SELECT state FROM moved_thread WHERE thread = ?1 AND state IN ('preparing', 'handed')",
            [thread],
            |row| row.get(0),
        )
        .optional()
        .map_err(failed)?;
    if open.is_some() {
        return Ok(Some(State::Moving));
    }
    let sealed: Option<String> = db
        .query_row(
            "SELECT state FROM moved_thread WHERE thread = ?1 AND state = 'sealed'",
            [thread],
            |row| row.get(0),
        )
        .optional()
        .map_err(failed)?;
    Ok(sealed.map(|_| State::Sealed))
}

pub(crate) fn open_move(db: &Connection, thread: &str) -> Result<Option<Moving>, String> {
    db.query_row(
        "SELECT transfer, state, target_address, target_name, target_environment, since_ms \
         FROM moved_thread WHERE thread = ?1 AND state IN ('preparing', 'handed')",
        [thread],
        |row| {
            Ok(Moving {
                transfer: row.get(0)?,
                phase: if row.get::<_, String>(1)? == "handed" {
                    MovePhase::Handed
                } else {
                    MovePhase::Preparing
                },
                target_address: row.get(2)?,
                target_name: row.get(3)?,
                target_environment: row.get(4)?,
                since_ms: count(row.get(5)?),
            })
        },
    )
    .optional()
    .map_err(failed)
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct Saved {
    pub turn: u64,
    pub text: String,
}

fn parse_saved(text: &str) -> Result<Vec<Saved>, String> {
    serde_json::from_str(text).map_err(|e| e.to_string())
}

fn write_saved(db: &Connection, transfer: &str, saved: &[Saved]) -> Result<(), String> {
    let text = serde_json::to_string(saved).map_err(|e| e.to_string())?;
    db.execute(
        "UPDATE moved_thread SET queued = ?2 WHERE transfer = ?1",
        params![transfer, text],
    )
    .map(|_| ())
    .map_err(failed)
}

pub(crate) fn saved(db: &Connection, transfer: &str) -> Result<Vec<Saved>, String> {
    let text: Option<String> = db
        .query_row(
            "SELECT queued FROM moved_thread WHERE transfer = ?1 AND state IN ('preparing', 'handed')",
            [transfer],
            |row| row.get(0),
        )
        .optional()
        .map_err(failed)?;
    parse_saved(&text.ok_or_else(|| MOVE_ENDED.to_owned())?)
}

pub(crate) fn queued(db: &Connection, transfer: &str) -> Result<Vec<String>, String> {
    Ok(saved(db, transfer)?
        .into_iter()
        .map(|entry| entry.text)
        .collect())
}

pub(crate) fn enqueue(
    db: &Connection,
    transfer: &str,
    turn: u64,
    text: &str,
) -> Result<(), String> {
    let mut entries = saved(db, transfer)?;
    if !entries.iter().any(|entry| entry.turn == turn) {
        entries.push(Saved {
            turn,
            text: text.to_owned(),
        });
    }
    write_saved(db, transfer, &entries)
}

pub(crate) fn forget(db: &Connection, transfer: &str, turn: u64) -> Result<(), String> {
    let mut entries = saved(db, transfer)?;
    entries.retain(|entry| entry.turn != turn);
    write_saved(db, transfer, &entries)
}

pub(crate) fn phase(db: &Connection, transfer: &str) -> Result<Option<Phase>, String> {
    let text: Option<String> = db
        .query_row(
            "SELECT state FROM moved_thread WHERE transfer = ?1",
            [transfer],
            |row| row.get(0),
        )
        .optional()
        .map_err(failed)?;
    Ok(text.as_deref().map(|text| match text {
        "preparing" => Phase::Preparing,
        "handed" => Phase::Handed,
        "sealed" => Phase::Sealed,
        _ => Phase::Released,
    }))
}

pub(crate) fn package(db: &Connection, transfer: &str) -> Result<Option<String>, String> {
    db.query_row(
        "SELECT package FROM moved_thread WHERE transfer = ?1",
        [transfer],
        |row| row.get(0),
    )
    .optional()
    .map_err(failed)
    .map(Option::flatten)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Preparing,
    Handed,
    Sealed,
    Released,
}

pub(crate) fn owned_by(db: &Connection, transfer: &str, thread: &str) -> Result<(), String> {
    let owner: Option<String> = db
        .query_row(
            "SELECT thread FROM moved_thread WHERE transfer = ?1",
            [transfer],
            |row| row.get(0),
        )
        .optional()
        .map_err(failed)?;
    match owner {
        Some(owner) if owner != thread => {
            Err("This transfer belongs to another session, so nothing changed.".to_owned())
        }
        _ => Ok(()),
    }
}

pub(crate) fn begin(
    db: &Connection,
    thread: &str,
    root: &str,
    transfer: &str,
    target: (&str, &str, &str),
    at_ms: u64,
) -> Result<(), String> {
    db.execute(
        "INSERT INTO moved_thread (transfer, thread, root, state, target_address, target_name, \
         target_environment, since_ms) VALUES (?1, ?2, ?3, 'preparing', ?4, ?5, ?6, ?7)",
        params![
            transfer,
            thread,
            root,
            target.0,
            target.1,
            target.2,
            sql(at_ms)
        ],
    )
    .map(|_| ())
    .map_err(failed)
}

pub(crate) fn hand(
    db: &Connection,
    transfer: &str,
    package: &str,
    at_ms: u64,
) -> Result<(), String> {
    let handed = db
        .execute(
            "UPDATE moved_thread SET state = 'handed', package = ?2, since_ms = ?3 \
             WHERE transfer = ?1 AND state = 'preparing'",
            params![transfer, package, sql(at_ms)],
        )
        .map_err(failed)?;
    if handed == 0 {
        return Err(MOVE_ENDED.to_owned());
    }
    Ok(())
}

pub(crate) fn release(db: &Connection, transfer: &str) -> Result<(), String> {
    match phase(db, transfer)? {
        Some(Phase::Preparing | Phase::Handed) => {}
        Some(Phase::Released) => return Ok(()),
        Some(Phase::Sealed) => {
            return Err("This move was sealed, so it cannot be released.".to_owned());
        }
        None => return Err(UNKNOWN_MOVE.to_owned()),
    }
    db.execute(
        "UPDATE moved_thread SET state = 'released', queued = '[]' WHERE transfer = ?1",
        [transfer],
    )
    .map(|_| ())
    .map_err(failed)
}

pub(crate) fn seal(
    db: &Connection,
    transfer: &str,
    to_host: &str,
    at_ms: u64,
) -> Result<(), String> {
    let tx = db.unchecked_transaction().map_err(failed)?;
    match phase(&tx, transfer)? {
        Some(Phase::Handed) => {
            tx.execute(
                "UPDATE moved_thread SET state = 'sealed', to_host = ?2, since_ms = ?3, queued = '[]' \
                 WHERE transfer = ?1",
                params![transfer, to_host, sql(at_ms)],
            )
            .map_err(failed)?;
        }
        Some(Phase::Sealed) => {}
        Some(Phase::Preparing) => {
            return Err("This move is not handed over yet, so it cannot be sealed.".to_owned());
        }
        Some(Phase::Released) => {
            return Err("This move was released, so it cannot be sealed.".to_owned());
        }
        None => return Err(UNKNOWN_MOVE.to_owned()),
    }
    tx.execute(
        "INSERT OR REPLACE INTO pause_pending (thread, root) \
         SELECT thread, root FROM moved_thread WHERE transfer = ?1",
        [transfer],
    )
    .map_err(failed)?;
    tx.commit().map_err(failed)
}

pub(crate) fn sealed_threads(db: &Connection) -> Result<HashSet<String>, String> {
    let mut statement = db
        .prepare("SELECT thread FROM moved_thread WHERE state = 'sealed'")
        .map_err(failed)?;
    let rows = statement.query_map([], |row| row.get(0)).map_err(failed)?;
    rows.collect::<Result<_, _>>().map_err(failed)
}

pub(crate) fn put_record(
    db: &Connection,
    thread: &str,
    transfer: &str,
    record: &MoveRecord,
) -> Result<(), String> {
    let transcript = serde_json::to_string(&record.transcript).map_err(|e| e.to_string())?;
    db.execute(
        "INSERT OR REPLACE INTO move_record (thread, transfer, from_host, to_host, at_ms, transcript) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            thread,
            transfer,
            record.from_host,
            record.to_host,
            sql(record.at_ms),
            transcript
        ],
    )
    .map(|_| ())
    .map_err(failed)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArrivalRow {
    pub state: String,
    pub root: String,
    pub thread: String,
    pub queued: Vec<String>,
    pub delivered: u64,
}

impl ArrivalRow {
    pub(crate) fn undelivered(&self) -> bool {
        usize::try_from(self.delivered).is_ok_and(|delivered| delivered < self.queued.len())
    }
}

pub(crate) fn arrival(db: &Connection, transfer: &str) -> Result<Option<ArrivalRow>, String> {
    let row: Option<(String, String, String, String, i64)> = db
        .query_row(
            "SELECT state, root, thread, queued, delivered FROM arrival WHERE transfer = ?1",
            [transfer],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(failed)?;
    row.map(|(state, root, thread, queued, delivered)| {
        Ok(ArrivalRow {
            state,
            root,
            thread,
            queued: serde_json::from_str(&queued).map_err(|e| e.to_string())?,
            delivered: count(delivered),
        })
    })
    .transpose()
}

pub(crate) fn set_arrival(
    db: &Connection,
    transfer: &str,
    row: &ArrivalRow,
    at_ms: u64,
) -> Result<(), String> {
    let queued = serde_json::to_string(&row.queued).map_err(|e| e.to_string())?;
    db.execute(
        "INSERT OR REPLACE INTO arrival (transfer, state, root, thread, queued, delivered, at_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            transfer,
            row.state,
            row.root,
            row.thread,
            queued,
            sql(row.delivered),
            sql(at_ms)
        ],
    )
    .map(|_| ())
    .map_err(failed)
}

pub(crate) fn set_delivered(
    db: &Connection,
    transfer: &str,
    delivered: usize,
) -> Result<(), String> {
    let delivered = u64::try_from(delivered).map_err(|e| e.to_string())?;
    db.execute(
        "UPDATE arrival SET delivered = ?2 WHERE transfer = ?1",
        params![transfer, sql(delivered)],
    )
    .map(|_| ())
    .map_err(failed)
}

pub(crate) fn pause_done(db: &Connection, thread: &str) -> Result<(), String> {
    db.execute(
        "DELETE FROM pause_pending WHERE thread = ?1",
        params![thread],
    )
    .map(|_| ())
    .map_err(failed)
}

pub(crate) fn pauses_pending(db: &Connection) -> Result<Vec<(String, String)>, String> {
    let mut statement = db
        .prepare("SELECT thread, root FROM pause_pending ORDER BY thread")
        .map_err(failed)?;
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(failed)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(failed)
}

pub(crate) fn record(db: &Connection, thread: &str) -> Result<Option<MoveRecord>, String> {
    let row = db
        .query_row(
            "SELECT from_host, to_host, at_ms, transcript FROM move_record WHERE thread = ?1",
            [thread],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(failed)?;
    row.map(|(from_host, to_host, at_ms, transcript)| {
        Ok(MoveRecord {
            from_host,
            to_host,
            at_ms: u64::try_from(at_ms).unwrap_or_default(),
            transcript: serde_json::from_str(&transcript).map_err(|e| e.to_string())?,
        })
    })
    .transpose()
}

pub(crate) fn handoff_id(transfer: &str) -> String {
    format!("handoff-{transfer}")
}

pub(crate) fn queued_id(transfer: &str, index: usize) -> String {
    format!("queued-{transfer}-{index}")
}

pub(crate) fn returned_id(transfer: &str, turn: u64) -> String {
    format!("returned-{transfer}-{turn}")
}

pub(crate) fn within_budget(package: &MovePackage) -> Result<(), String> {
    if package.handoff.len() > HANDOFF_BYTES {
        return Err(format!(
            "The handoff is {} bytes; a move carries at most {HANDOFF_BYTES} bytes.",
            package.handoff.len()
        ));
    }
    Ok(())
}
