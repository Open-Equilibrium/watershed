//! Liquid's local replica: the Workspace [`Store`] in one SQLite database, encrypted with
//! SQLCipher and in write-ahead-log mode (ADR-0237). SQL stays inside this crate (client
//! architecture rule 15).
//!
//! The database holds each document's chunks, Automerge changes and snapshots, and every
//! recorded Action. A commit is one transaction with full synchronization, so an
//! acknowledged write survives the app being killed.

#![deny(missing_docs)]

use std::fmt::Write as _;
use std::path::Path;

use liquid_core::{Action, ActionId, Commit, Document, Error, Store, Write};
use rusqlite::{Connection, OptionalExtension, params};

/// The schema version this crate writes; [`Sqlite::open`] migrates older databases to it.
const VERSION: i64 = 1;

const SCHEMA: &str = "
    CREATE TABLE chunks (
        seq INTEGER PRIMARY KEY,
        document TEXT NOT NULL,
        bytes BLOB NOT NULL
    );
    CREATE INDEX chunks_by_document ON chunks (document, seq);
    CREATE TABLE actions (
        seq INTEGER PRIMARY KEY,
        id TEXT NOT NULL UNIQUE,
        record TEXT NOT NULL
    );
";

/// A Workspace store in an encrypted SQLite database.
pub struct Sqlite {
    connection: Connection,
}

fn failed(error: impl std::fmt::Display) -> Error {
    Error::Storage(error.to_string())
}

impl Sqlite {
    /// Opens the database at `path`, creating it if it does not exist, with the 256-bit
    /// `key` that the operating system's keystore holds. A wrong key is refused.
    pub fn open(path: &Path, key: &[u8; 32]) -> Result<Self, Error> {
        let connection = Connection::open(path).map_err(failed)?;
        let mut hex = String::with_capacity(64);
        for byte in key {
            let _ = write!(hex, "{byte:02x}");
        }
        connection
            .execute_batch(&format!("PRAGMA key = \"x'{hex}'\";"))
            .map_err(failed)?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|_| Error::Storage("wrong key, or not a Liquid database".into()))?;
        let mode: String = connection
            .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
            .map_err(failed)?;
        if mode != "wal" {
            return Err(Error::Storage(format!("journal mode {mode}, not WAL")));
        }
        connection
            .execute_batch("PRAGMA synchronous = FULL;")
            .map_err(failed)?;
        match version {
            0 => connection
                .execute_batch(&format!(
                    "BEGIN; {SCHEMA} PRAGMA user_version = {VERSION}; COMMIT;"
                ))
                .map_err(failed)?,
            VERSION => {}
            newer => {
                return Err(Error::Storage(format!(
                    "schema version {newer} is newer than this Liquid"
                )));
            }
        }
        Ok(Self { connection })
    }
}

impl Store for Sqlite {
    fn chunks(&mut self, document: &Document) -> Result<Vec<Vec<u8>>, Error> {
        let mut query = self
            .connection
            .prepare_cached("SELECT bytes FROM chunks WHERE document = ?1 ORDER BY seq")
            .map_err(failed)?;
        let rows = query
            .query_map([document.key()], |row| row.get(0))
            .map_err(failed)?;
        rows.collect::<Result<_, _>>().map_err(failed)
    }

    fn commit(&mut self, commit: &Commit<'_>) -> Result<(), Error> {
        let record = serde_json::to_string(commit.action).map_err(failed)?;
        let transaction = self.connection.transaction().map_err(failed)?;
        for write in &commit.writes {
            match write {
                Write::Append { document, chunk } => {
                    transaction
                        .prepare_cached("INSERT INTO chunks (document, bytes) VALUES (?1, ?2)")
                        .and_then(|mut insert| insert.execute(params![document.key(), chunk]))
                        .map_err(failed)?;
                }
                Write::Replace { document, snapshot } => {
                    let key = document.key();
                    transaction
                        .execute("DELETE FROM chunks WHERE document = ?1", [&key])
                        .and_then(|_| {
                            transaction.execute(
                                "INSERT INTO chunks (document, bytes) VALUES (?1, ?2)",
                                params![key, snapshot],
                            )
                        })
                        .map_err(failed)?;
                }
            }
        }
        transaction
            .prepare_cached("INSERT INTO actions (id, record) VALUES (?1, ?2)")
            .and_then(|mut insert| insert.execute(params![commit.action.id.as_str(), record]))
            .map_err(failed)?;
        transaction.commit().map_err(failed)
    }

    fn history(&mut self, before: Option<&ActionId>, limit: usize) -> Result<Vec<Action>, Error> {
        let end: i64 = match before {
            None => i64::MAX,
            Some(id) => self
                .connection
                .query_row(
                    "SELECT seq FROM actions WHERE id = ?1",
                    [id.as_str()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(failed)?
                .ok_or(Error::NotFound)?,
        };
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut query = self
            .connection
            .prepare_cached("SELECT record FROM actions WHERE seq < ?1 ORDER BY seq DESC LIMIT ?2")
            .map_err(failed)?;
        let records = query
            .query_map(params![end, limit], |row| row.get::<_, String>(0))
            .map_err(failed)?;
        records
            .map(|record| serde_json::from_str(&record.map_err(failed)?).map_err(failed))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_engine_has_full_text_search_for_the_power_bar() {
        let dir = std::env::temp_dir().join(format!("liquid-storage-fts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Sqlite::open(&dir.join("replica.db"), &[1; 32]).unwrap();
        store
            .connection
            .execute_batch("CREATE VIRTUAL TABLE probe USING fts5(text);")
            .unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
