mod links;
mod pairing;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::store::failed;
use crate::util::hex;

pub(crate) use links::Links;
pub use links::{HostList, LinkedHostInfo};
pub use pairing::ClientInfo;
pub(crate) use pairing::{client, fitted, invite, pair, printable, revoke};

pub const DEFAULT_REMOTE_PORT: u16 = 3774;

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS pairing (
    code_hash TEXT PRIMARY KEY, kind TEXT NOT NULL, access TEXT NOT NULL,
    expires_ms INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS client (
    token_hash TEXT PRIMARY KEY, kind TEXT NOT NULL, access TEXT NOT NULL, name TEXT NOT NULL,
    created_ms INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS peer (
    name TEXT PRIMARY KEY, address TEXT NOT NULL, token TEXT NOT NULL, access TEXT NOT NULL,
    tunnel_port INTEGER NOT NULL, linked_ms INTEGER, client TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS setting (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    Device,
    Host,
}

impl ClientKind {
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

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Access {
    ReadOnly,
    ApprovalRequired,
    AutoAcceptEdits,
    #[default]
    Auto,
    FullAccess,
}

impl Access {
    pub const ALL: [Self; 5] = [
        Self::ReadOnly,
        Self::ApprovalRequired,
        Self::AutoAcceptEdits,
        Self::Auto,
        Self::FullAccess,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::ApprovalRequired => "approval-required",
            Self::AutoAcceptEdits => "auto-accept-edits",
            Self::Auto => "auto",
            Self::FullAccess => "full-access",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|level| level.as_str() == text)
    }
}

pub(crate) fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("the operating system provides random bytes");
    bytes
}

pub(crate) fn new_token() -> String {
    hex(&random::<32>())
}

fn digest(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

pub(crate) fn same(given: &[u8], expected: &[u8]) -> bool {
    given.len() == expected.len()
        && given
            .iter()
            .zip(expected)
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_levels_order_from_read_only_to_full_access() {
        let mut levels = Access::ALL;
        levels.reverse();
        levels.sort();
        assert_eq!(levels, Access::ALL);
        assert!(Access::ApprovalRequired < Access::Auto);
    }
}
