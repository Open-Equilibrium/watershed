//! Hosts and remote devices (ADR-0285, ADR-0299, ADR-0300): the single-use pairing codes this
//! service issues, the tokens it admits and what each token may do. The tables live in the
//! configuration store's SQLite file, beside its append-only tables, which they never touch.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{failed, sql};

/// The tables of hosts and devices, created with the configuration store's.
pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS pairing (
    code_hash TEXT PRIMARY KEY, kind TEXT NOT NULL, expires_ms INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS client (
    token_hash TEXT PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL,
    created_ms INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS peer (
    name TEXT PRIMARY KEY, address TEXT NOT NULL, token TEXT NOT NULL, access TEXT NOT NULL,
    tunnel_port INTEGER NOT NULL, linked_ms INTEGER);
CREATE TABLE IF NOT EXISTS setting (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

/// How long a pairing code stays valid.
const CODE_TTL_MS: u64 = 5 * 60_000;

/// How many pairing codes may be live at once.
const MAX_CODES: usize = 5;

/// The letters of a pairing code: without I, L, O, 0 and 1, which look like others.
const CODE_LETTERS: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

/// The refusal of a pairing code that is wrong, used or expired.
const REFUSED_CODE: &str = "That pairing code is wrong, already used or expired.";

/// What a client paired as. A host's token reaches only what T3's peer link needs; a device's
/// reaches the whole API.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A device, such as Liquid on another computer.
    Device,
    /// Another host's Meta-Harness.
    Host,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Device => "device",
            Self::Host => "host",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "device" => Some(Self::Device),
            "host" => Some(Self::Host),
            _ => None,
        }
    }
}

/// A new pairing code, shown as `XXXX-XXXX`, from the operating system's randomness.
fn new_code() -> String {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("the operating system provides random bytes");
    let letters: String = bytes
        .iter()
        .map(|b| char::from(CODE_LETTERS[usize::from(*b) % CODE_LETTERS.len()]))
        .collect();
    format!("{}-{}", &letters[..4], &letters[4..])
}

/// A token of 256 random bits, in hex.
pub(crate) fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the operating system provides random bytes");
    hex(&bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The digest a code or token is stored and compared by.
fn digest(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

/// A code as it is compared: without its dash or spaces, in capitals, so a typed one matches.
fn normal(code: &str) -> String {
    code.chars()
        .filter(|c| *c != '-' && !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Whether `given` equals `expected`, in time that does not depend on where they differ.
pub(crate) fn same(given: &[u8], expected: &[u8]) -> bool {
    given.len() == expected.len()
        && given
            .iter()
            .zip(expected)
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

/// Issues a pairing code for `kind` that is valid for five minutes from `now`. Expired codes are
/// dropped first; no code is issued while [`MAX_CODES`] are live.
pub(crate) fn invite(db: &Connection, kind: Kind, now: u64) -> Result<String, String> {
    db.execute("DELETE FROM pairing WHERE expires_ms <= ?1", [sql(now)])
        .map_err(failed)?;
    let live: i64 = db
        .query_row("SELECT COUNT(*) FROM pairing", [], |row| row.get(0))
        .map_err(failed)?;
    if live >= i64::try_from(MAX_CODES).unwrap_or(i64::MAX) {
        return Err(format!(
            "At most {MAX_CODES} pairing codes are live; pair with one of them or wait 5 minutes."
        ));
    }
    let code = new_code();
    db.execute(
        "INSERT INTO pairing (code_hash, kind, expires_ms) VALUES (?1, ?2, ?3)",
        params![
            digest(&normal(&code)),
            kind.as_str(),
            sql(now + CODE_TTL_MS)
        ],
    )
    .map_err(failed)?;
    Ok(code)
}

/// Redeems `code` at `now` for a client named `name`: the code is spent, and the client gets a
/// new token, which this returns. The code's kind decides what the token admits.
pub(crate) fn pair(db: &Connection, code: &str, name: &str, now: u64) -> Result<String, String> {
    let tx = db.unchecked_transaction().map_err(failed)?;
    let wanted = digest(&normal(code));
    let mut kind = None;
    {
        let mut statement = tx
            .prepare("SELECT code_hash, kind FROM pairing WHERE expires_ms > ?1")
            .map_err(failed)?;
        let rows = statement
            .query_map([sql(now)], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(failed)?;
        // Every row is compared, so the time taken does not show where a match is.
        for row in rows {
            let (hash, found) = row.map_err(failed)?;
            if same(hash.as_bytes(), wanted.as_bytes()) {
                kind = Kind::parse(&found);
            }
        }
    }
    let kind = kind.ok_or(REFUSED_CODE)?;
    tx.execute("DELETE FROM pairing WHERE code_hash = ?1", [&wanted])
        .map_err(failed)?;
    let token = new_token();
    tx.execute(
        "INSERT INTO client (token_hash, kind, name, created_ms) VALUES (?1, ?2, ?3, ?4)",
        params![digest(&token), kind.as_str(), name, sql(now)],
    )
    .map_err(failed)?;
    tx.commit().map_err(failed)?;
    Ok(token)
}

/// The kind of client that `token` belongs to, if a client holds it.
pub(crate) fn client_kind(db: &Connection, token: &str) -> Result<Option<Kind>, String> {
    let wanted = digest(token);
    let mut statement = db
        .prepare("SELECT token_hash, kind FROM client")
        .map_err(failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(failed)?;
    let mut found = None;
    for row in rows {
        let (hash, kind) = row.map_err(failed)?;
        if same(hash.as_bytes(), wanted.as_bytes()) {
            found = Kind::parse(&kind);
        }
    }
    Ok(found)
}

/// Revokes the client named `name`: its token is refused from its next connection on.
pub(crate) fn revoke(db: &Connection, name: &str) -> Result<(), String> {
    let removed = db
        .execute("DELETE FROM client WHERE name = ?1", [name])
        .map_err(failed)?;
    if removed == 0 {
        Err(format!("There is no client named {name}."))
    } else {
        Ok(())
    }
}

/// The port the remote listener is set to listen on, while remote access is on.
pub(crate) fn remote_port(db: &Connection) -> Result<Option<u16>, String> {
    let value: Option<String> = db
        .query_row(
            "SELECT value FROM setting WHERE key = 'remote_port'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(failed)?;
    Ok(value.and_then(|value| value.parse().ok()))
}

/// Sets, or with `None` clears, the port the remote listener listens on.
pub(crate) fn set_remote_port(db: &Connection, port: Option<u16>) -> Result<(), String> {
    match port {
        Some(port) => db.execute(
            "INSERT INTO setting (key, value) VALUES ('remote_port', ?1) \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [port.to_string()],
        ),
        None => db.execute("DELETE FROM setting WHERE key = 'remote_port'", []),
    }
    .map(|_| ())
    .map_err(failed)
}
