//! Liquid's local replica: the Workspace [`Store`] in one SQLite database, encrypted with
//! SQLCipher and in write-ahead-log mode (ADR-0237). SQL stays inside this crate (client
//! architecture rule 15).
//!
//! The database holds each document's chunks, Automerge changes and snapshots, every
//! recorded Action, and the attachments' parts (ADR-0305). A commit is one transaction that reaches the storage medium, so an
//! acknowledged write survives the app being killed and a power loss.

#![deny(missing_docs)]

use std::fmt::Write as _;
use std::path::Path;

use liquid_core::{Action, ActionId, AttachmentId, Commit, Document, Error, Store, Write};
use rusqlite::{Connection, ErrorCode, OptionalExtension, params};

/// The schema version this crate writes; [`Sqlite::open`] creates it in a new database and
/// refuses a newer one.
const VERSION: i64 = 2;

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

/// What schema version 2 adds: attachments, each one upload's parts under the SHA-256 of
/// its bytes. Parts of an upload no attachment names are unfinished.
const ATTACHMENTS: &str = "
    CREATE TABLE parts (
        seq INTEGER PRIMARY KEY,
        upload TEXT NOT NULL,
        idx INTEGER NOT NULL,
        bytes BLOB NOT NULL,
        UNIQUE (upload, idx)
    );
    CREATE TABLE attachments (
        id TEXT PRIMARY KEY,
        upload TEXT NOT NULL,
        size INTEGER NOT NULL
    );
";

/// A Workspace store in an encrypted SQLite database.
pub struct Sqlite {
    connection: Connection,
}

fn failed(error: impl std::fmt::Display) -> Error {
    Error::Storage(error.to_string())
}

fn held() -> Error {
    Error::Storage("another Liquid session holds this replica".into())
}

impl Sqlite {
    /// Opens the database at `path`, creating it if it does not exist, with the 256-bit
    /// `key` that the operating system's keystore holds. A wrong key is refused, and so is
    /// a database another session holds: each session keeps its documents in memory, so
    /// one session at a time owns the replica until it is dropped.
    pub fn open(path: &Path, key: &[u8; 32]) -> Result<Self, Error> {
        let connection = Connection::open(path).map_err(failed)?;
        let mut hex = String::with_capacity(64);
        for byte in key {
            let _ = write!(hex, "{byte:02x}");
        }
        connection
            .execute_batch(&format!(
                "PRAGMA key = \"x'{hex}'\"; PRAGMA locking_mode = EXCLUSIVE;"
            ))
            .map_err(failed)?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|error| match error.sqlite_error_code() {
                Some(ErrorCode::DatabaseBusy) => held(),
                _ => Error::Storage("wrong key, or not a Liquid database".into()),
            })?;
        // The first write takes the exclusive lock, which the connection keeps.
        connection
            .execute_batch("BEGIN EXCLUSIVE; COMMIT;")
            .map_err(|error| match error.sqlite_error_code() {
                Some(ErrorCode::DatabaseBusy) => held(),
                _ => failed(error),
            })?;
        let mode: String = connection
            .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
            .map_err(failed)?;
        if mode != "wal" {
            return Err(Error::Storage(format!("journal mode {mode}, not WAL")));
        }
        // Full synchronization, and on Apple systems F_FULLFSYNC, so a commit reaches the
        // storage medium rather than the drive's cache and survives a power loss.
        connection
            .execute_batch(
                "PRAGMA synchronous = FULL; PRAGMA fullfsync = ON; PRAGMA checkpoint_fullfsync = ON;",
            )
            .map_err(failed)?;
        let create = match version {
            0 => format!("{SCHEMA} {ATTACHMENTS}"),
            1 => ATTACHMENTS.to_string(),
            VERSION => String::new(),
            newer => {
                return Err(Error::Storage(format!(
                    "schema version {newer} is newer than this Liquid"
                )));
            }
        };
        // An upload the last session never finished is dropped.
        connection
            .execute_batch(&format!(
                "BEGIN; {create} PRAGMA user_version = {VERSION}; \
                 DELETE FROM parts WHERE upload NOT IN (SELECT upload FROM attachments); COMMIT;"
            ))
            .map_err(failed)?;
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

    fn put_part(&mut self, upload: &str, index: u64, bytes: &[u8]) -> Result<(), Error> {
        // SQLite keeps i64; a larger index would wrap into a negative one.
        let index = i64::try_from(index).map_err(failed)?;
        self.connection
            .prepare_cached("INSERT INTO parts (upload, idx, bytes) VALUES (?1, ?2, ?3)")
            .and_then(|mut insert| insert.execute(params![upload, index, bytes]))
            .map_err(failed)?;
        Ok(())
    }

    fn finish_upload(
        &mut self,
        upload: &str,
        attachment: &AttachmentId,
        size: u64,
        parts: u64,
    ) -> Result<(), Error> {
        let size = i64::try_from(size).map_err(failed)?;
        let transaction = self.connection.transaction().map_err(failed)?;
        // Parts run from 0 without a gap: the lowest index 0 and the highest one less
        // than their count.
        let (kept, first, last): (i64, Option<i64>, Option<i64>) = transaction
            .query_row(
                "SELECT count(*), min(idx), max(idx) FROM parts WHERE upload = ?1",
                [upload],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(failed)?;
        let gapless = kept == 0 || (first == Some(0) && last == Some(kept - 1));
        if kept as u64 != parts || !gapless {
            return Err(Error::Storage(format!(
                "the upload has {kept} of its {parts} parts"
            )));
        }
        let added = transaction
            .execute(
                "INSERT OR IGNORE INTO attachments (id, upload, size) VALUES (?1, ?2, ?3)",
                params![attachment.as_str(), upload, size],
            )
            .map_err(failed)?;
        // A copy of a kept attachment goes, unless it is the upload the attachment names.
        if added == 0 {
            transaction
                .execute(
                    "DELETE FROM parts WHERE upload = ?1 \
                     AND NOT EXISTS (SELECT 1 FROM attachments WHERE upload = ?1)",
                    [upload],
                )
                .map_err(failed)?;
        }
        transaction.commit().map_err(failed)
    }

    fn discard_upload(&mut self, upload: &str) -> Result<(), Error> {
        self.connection
            .execute(
                // A finished attachment's parts stay whatever is asked.
                "DELETE FROM parts WHERE upload = ?1 \
                 AND NOT EXISTS (SELECT 1 FROM attachments WHERE upload = ?1)",
                [upload],
            )
            .map_err(failed)?;
        Ok(())
    }

    fn attachment_size(&mut self, attachment: &AttachmentId) -> Result<Option<u64>, Error> {
        let size: Option<i64> = self
            .connection
            .prepare_cached("SELECT size FROM attachments WHERE id = ?1")
            .and_then(|mut query| {
                query
                    .query_row([attachment.as_str()], |row| row.get(0))
                    .optional()
            })
            .map_err(failed)?;
        size.map(|size| u64::try_from(size).map_err(failed))
            .transpose()
    }

    fn part(&mut self, attachment: &AttachmentId, index: u64) -> Result<Option<Vec<u8>>, Error> {
        self.connection
            .prepare_cached(
                "SELECT parts.bytes FROM attachments JOIN parts \
                 ON parts.upload = attachments.upload AND parts.idx = ?2 \
                 WHERE attachments.id = ?1",
            )
            .and_then(|mut query| {
                query
                    .query_row(params![attachment.as_str(), index as i64], |row| row.get(0))
                    .optional()
            })
            .map_err(failed)
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

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "liquid-storage-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn parts(store: &Sqlite) -> i64 {
        store
            .connection
            .query_row("SELECT count(*) FROM parts", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn a_file_added_twice_is_kept_once_and_an_unfinished_one_goes_at_the_next_open() {
        let dir = scratch("parts");
        let path = dir.join("replica.db");
        let mut store = Sqlite::open(&path, &[1; 32]).unwrap();
        let id = AttachmentId::parse(&"a".repeat(64)).unwrap();
        for upload in ["first", "second"] {
            store.put_part(upload, 0, b"tram").unwrap();
            store.put_part(upload, 1, b"28").unwrap();
            store.finish_upload(upload, &id, 6, 2).unwrap();
        }
        assert_eq!(parts(&store), 2, "the second copy's parts are dropped");
        store.finish_upload("first", &id, 6, 2).unwrap();
        assert_eq!(parts(&store), 2, "finishing again keeps the kept parts");
        store.discard_upload("first").unwrap();
        assert_eq!(parts(&store), 2, "a finished attachment's parts stay");
        store.put_part("gap", 0, b"a").unwrap();
        store.put_part("gap", 2, b"c").unwrap();
        let other = AttachmentId::parse(&"b".repeat(64)).unwrap();
        assert!(
            store.finish_upload("gap", &other, 2, 2).is_err(),
            "parts must run from 0 without a gap"
        );
        store.discard_upload("gap").unwrap();
        // An index past i64 is refused, so it never wraps into a negative one.
        assert!(store.put_part("wrap", u64::MAX, b"z").is_err());
        store.put_part("unfinished", 0, b"castle").unwrap();
        store.put_part("discarded", 0, b"river").unwrap();
        store.discard_upload("discarded").unwrap();
        assert_eq!(parts(&store), 3);
        drop(store);

        let mut store = Sqlite::open(&path, &[1; 32]).unwrap();
        assert_eq!(parts(&store), 2, "the unfinished upload is gone");
        assert_eq!(store.part(&id, 1).unwrap().as_deref(), Some(&b"28"[..]));
        assert_eq!(store.attachment_size(&id).unwrap(), Some(6));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_replica_of_the_first_schema_gains_attachments() {
        let dir = scratch("upgrade");
        let path = dir.join("replica.db");
        let connection = Connection::open(&path).unwrap();
        let hex = "01".repeat(32);
        connection
            .execute_batch(&format!(
                "PRAGMA key = \"x'{hex}'\"; CREATE TABLE chunks (seq INTEGER PRIMARY KEY, \
                 document TEXT NOT NULL, bytes BLOB NOT NULL); CREATE TABLE actions (seq \
                 INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE, record TEXT NOT NULL); \
                 PRAGMA user_version = 1;"
            ))
            .unwrap();
        drop(connection);

        let mut store = Sqlite::open(&path, &[1; 32]).unwrap();
        let version: i64 = store
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, VERSION);
        let id = AttachmentId::parse(&"b".repeat(64)).unwrap();
        store.put_part("upload", 0, b"tram").unwrap();
        store.finish_upload("upload", &id, 4, 1).unwrap();
        assert_eq!(store.attachment_size(&id).unwrap(), Some(4));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
