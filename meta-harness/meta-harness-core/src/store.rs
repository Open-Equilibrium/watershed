use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;

use crate::config::{
    Action, Blocks, Current, LogEntry, MAX_ENTRIES, MAX_SUMMARY, Proposal, Proposed, apply,
    check_change, check_size, check_text, json,
};

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

pub(crate) struct Store {
    db: Mutex<Connection>,
    reconciled: AtomicBool,
}

impl Store {
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
        db.execute_batch(&format!(
            "BEGIN; {schema} {} {} {} COMMIT;",
            crate::hosts::SCHEMA,
            crate::host_move::SCHEMA,
            crate::agents::SCHEMA
        ))
        .map_err(failed)?;
        Ok(Self {
            db: Mutex::new(db),
            reconciled: AtomicBool::new(true),
        })
    }

    pub fn reconciled(&self) -> bool {
        self.reconciled.load(Ordering::SeqCst)
    }

    pub fn set_reconciled(&self, reconciled: bool) {
        self.reconciled.store(reconciled, Ordering::SeqCst);
    }

    pub(crate) fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        crate::util::lock(&self.db)
    }

    pub fn current(&self) -> Result<Current, String> {
        current(&self.db())
    }

    pub fn propose(&self, client: &str, summary: &str, proposed: Proposed) -> Result<u64, String> {
        check_text("The summary", summary, MAX_SUMMARY)?;
        let mut db = self.db();
        let base = current(&db)?;
        let next = match &proposed {
            Proposed::Changes(changes) => {
                if changes.len() > MAX_ENTRIES {
                    return Err(format!("A proposal holds at most {MAX_ENTRIES} changes."));
                }
                changes.iter().try_for_each(check_change)?;
                let mut blocks = base.blocks.clone();
                changes
                    .iter()
                    .cloned()
                    .for_each(|change| apply(&mut blocks, change));
                blocks
            }
            Proposed::Restore(revision) => blocks_of(&db, *revision)?
                .ok_or_else(|| format!("There is no revision {revision}."))?,
        };
        if next == base.blocks {
            return Err(format!(
                "The proposal would not change revision {}.",
                base.revision
            ));
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

    pub fn approve(
        &self,
        client: &str,
        number: u64,
        write: impl Fn(&Blocks) -> Result<(), String>,
    ) -> Result<Current, String> {
        let mut db = self.db();
        let tx = db.transaction().map_err(failed)?;
        let proposed: Proposed = parse(&undecided(&tx, number)?)?;
        let base = current(&tx)?;
        let blocks = match proposed {
            Proposed::Changes(changes) => {
                let mut blocks = base.blocks.clone();
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
        let recorded = write(&blocks).and_then(|()| {
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
            tx.commit().map_err(failed)
        });
        if let Err(error) = recorded {
            return Err(match write(&base.blocks) {
                Ok(()) => {
                    self.set_reconciled(true);
                    error
                }
                Err(restore) => {
                    self.set_reconciled(false);
                    format!("{error}. The harness homes could not be restored: {restore}")
                }
            });
        }
        self.set_reconciled(true);
        Ok(Current { revision, blocks })
    }

    pub fn reject(&self, client: &str, number: u64) -> Result<(), String> {
        let mut db = self.db();
        let tx = db.transaction().map_err(failed)?;
        undecided(&tx, number)?;
        log(&tx, now_ms(), client, Action::Rejected, number, None)?;
        tx.commit().map_err(failed)
    }

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

pub(crate) fn sql(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

pub(crate) fn count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

fn parse<T: for<'a> Deserialize<'a>>(text: &str) -> Result<T, String> {
    serde_json::from_str(text).map_err(|e| format!("The Meta-Harness store is damaged: {e}"))
}

pub(crate) fn failed(error: rusqlite::Error) -> String {
    format!("The Meta-Harness store failed: {error}")
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}
