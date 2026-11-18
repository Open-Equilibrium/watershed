use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use super::{Access, ClientKind, digest, new_token, same};
use crate::config::{count, failed, sql};

const CODE_TTL_MS: u64 = 5 * 60_000;

const MAX_CODES: usize = 5;

const CODE_LETTERS: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

const REFUSED_CODE: &str = "That pairing code is wrong, already used or expired.";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    pub name: String,
    pub kind: ClientKind,
    pub access: Access,
    pub since_ms: u64,
}

fn new_code() -> String {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("the operating system provides random bytes");
    let letters: String = bytes
        .iter()
        .map(|b| char::from(CODE_LETTERS[usize::from(*b) % CODE_LETTERS.len()]))
        .collect();
    format!("{}-{}", &letters[..4], &letters[4..])
}

pub(crate) fn printable(name: &str) -> String {
    let cleaned: String = name.chars().filter(|c| !c.is_control()).collect();
    match fitted(&cleaned, 64, 100) {
        text if text.is_empty() => "unnamed".to_owned(),
        text => text,
    }
}

pub(crate) fn fitted(text: &str, chars: usize, units: usize) -> String {
    let mut used = 0;
    let fitting: String = text
        .trim_matches(is_space)
        .chars()
        .take(chars)
        .take_while(|c| {
            used += c.len_utf16();
            used <= units
        })
        .collect();
    fitting.trim_matches(is_space).to_owned()
}

fn is_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

fn normal(code: &str) -> String {
    code.chars()
        .filter(|c| *c != '-' && !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

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
        for row in rows {
            let (hash, kind, access) = row.map_err(failed)?;
            if same(hash.as_bytes(), wanted.as_bytes()) {
                found = ClientKind::parse(&kind).zip(Access::parse(&access));
            }
        }
    }
    let (kind, access) = found.ok_or(REFUSED_CODE)?;
    let name = printable(name);
    let taken: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM client WHERE name = ?1",
            [&name],
            |row| row.get(0),
        )
        .map_err(failed)?;
    if taken > 0 {
        return Err(format!(
            "A client named {name} is paired already; pair under another name."
        ));
    }
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
            name,
            sql(now)
        ],
    )
    .map_err(failed)?;
    tx.commit().map_err(failed)?;
    Ok((token, access))
}

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

pub(crate) fn revoke(db: &Connection, name: &str) -> Result<bool, String> {
    let removed = db
        .execute("DELETE FROM client WHERE name = ?1", [name])
        .map_err(failed)?;
    Ok(removed > 0)
}

pub(crate) fn clients(db: &Connection) -> Result<Vec<ClientInfo>, String> {
    let mut statement = db
        .prepare("SELECT name, kind, access, created_ms FROM client ORDER BY name")
        .map_err(failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(failed)?;
    rows.map(|row| {
        let (name, kind, access, since) = row.map_err(failed)?;
        Ok(ClientInfo {
            name,
            kind: ClientKind::parse(&kind)
                .ok_or("The configuration store holds an unknown client kind.")?,
            access: Access::parse(&access)
                .ok_or("The configuration store holds an unknown access level.")?,
            since_ms: count(since),
        })
    })
    .collect()
}
