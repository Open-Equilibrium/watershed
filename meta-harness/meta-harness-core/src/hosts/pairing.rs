//! Pairing codes and the clients they pair (ADR-0299): a code is single use and expires, and a
//! client's token admits only the kind of client that paired with it.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::{Access, ClientKind, digest, new_token, same};
use crate::config::{count, failed, sql};

/// How long a pairing code stays valid.
const CODE_TTL_MS: u64 = 5 * 60_000;

/// How many pairing codes may be live at once.
const MAX_CODES: usize = 5;

/// The letters of a pairing code: without I, L, O, 0 and 1, which look like others.
const CODE_LETTERS: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

/// The refusal of a pairing code that is wrong, used or expired.
const REFUSED_CODE: &str = "That pairing code is wrong, already used or expired.";

/// A client paired with this host, as `meta host list` shows it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    /// The name the client paired with.
    pub name: String,
    /// What it paired as.
    pub kind: ClientKind,
    /// When it paired, in milliseconds since the Unix epoch.
    pub since_ms: u64,
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

/// A client's name as the store keeps it and `meta host list` prints it: without control characters,
/// and at most 64 of the rest.
pub(crate) fn printable(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(64).collect()
}

/// A code as it is compared: without its dash or spaces, in capitals, so a typed one matches.
fn normal(code: &str) -> String {
    code.chars()
        .filter(|c| *c != '-' && !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Issues a pairing code for `kind` that is valid for five minutes from `now`, and gives the client
/// that redeems it `access` (ADR-0300). Expired codes are dropped first; no code is issued while
/// [`MAX_CODES`] are live.
pub(crate) fn invite(
    db: &Connection,
    kind: ClientKind,
    access: Access,
    now: u64,
) -> Result<String, String> {
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
        "INSERT INTO pairing (code_hash, kind, access, expires_ms) VALUES (?1, ?2, ?3, ?4)",
        params![
            digest(&normal(&code)),
            kind.as_str(),
            access.as_str(),
            sql(now + CODE_TTL_MS)
        ],
    )
    .map_err(failed)?;
    Ok(code)
}

/// Redeems `code` at `now` for a client named `name`: the code is spent, and the client gets a
/// new token, which this returns with the access the code gave. The code's kind decides what the
/// token admits.
pub(crate) fn pair(
    db: &Connection,
    code: &str,
    name: &str,
    now: u64,
) -> Result<(String, Access), String> {
    let tx = db.unchecked_transaction().map_err(failed)?;
    let wanted = digest(&normal(code));
    let mut found = None;
    {
        let mut statement = tx
            .prepare("SELECT code_hash, kind, access FROM pairing WHERE expires_ms > ?1")
            .map_err(failed)?;
        let rows = statement
            .query_map([sql(now)], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(failed)?;
        // Every row is compared, so the time taken does not show where a match is.
        for row in rows {
            let (hash, kind, access) = row.map_err(failed)?;
            if same(hash.as_bytes(), wanted.as_bytes()) {
                found = ClientKind::parse(&kind).zip(Access::parse(&access));
            }
        }
    }
    let (kind, access) = found.ok_or(REFUSED_CODE)?;
    tx.execute("DELETE FROM pairing WHERE code_hash = ?1", [&wanted])
        .map_err(failed)?;
    let token = new_token();
    tx.execute(
        "INSERT INTO client (token_hash, kind, access, name, created_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            digest(&token),
            kind.as_str(),
            access.as_str(),
            printable(name),
            sql(now)
        ],
    )
    .map_err(failed)?;
    tx.commit().map_err(failed)?;
    Ok((token, access))
}

/// The kind of client that `token` belongs to, its access and the name it paired with, if a
/// client holds it.
pub(crate) fn client(
    db: &Connection,
    token: &str,
) -> Result<Option<(ClientKind, Access, String)>, String> {
    let wanted = digest(token);
    let mut statement = db
        .prepare("SELECT token_hash, kind, access, name FROM client")
        .map_err(failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(failed)?;
    let mut found = None;
    for row in rows {
        let (hash, kind, access, name) = row.map_err(failed)?;
        if same(hash.as_bytes(), wanted.as_bytes()) {
            found = ClientKind::parse(&kind)
                .zip(Access::parse(&access))
                .map(|(kind, access)| (kind, access, name));
        }
    }
    Ok(found)
}

/// The kind of the client named `name`, if one is paired under that name.
pub(crate) fn named_kind(db: &Connection, name: &str) -> Result<Option<ClientKind>, String> {
    let kind: Option<String> = db
        .query_row("SELECT kind FROM client WHERE name = ?1", [name], |row| {
            row.get(0)
        })
        .optional()
        .map_err(failed)?;
    Ok(kind.and_then(|kind| ClientKind::parse(&kind)))
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

/// The clients paired with this host, by name.
pub(crate) fn clients(db: &Connection) -> Result<Vec<ClientInfo>, String> {
    let mut statement = db
        .prepare("SELECT name, kind, created_ms FROM client ORDER BY name")
        .map_err(failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(failed)?;
    rows.map(|row| {
        let (name, kind, since) = row.map_err(failed)?;
        Ok(ClientInfo {
            name,
            kind: ClientKind::parse(&kind)
                .ok_or("The configuration store holds an unknown client kind.")?,
            since_ms: count(since),
        })
    })
    .collect()
}
