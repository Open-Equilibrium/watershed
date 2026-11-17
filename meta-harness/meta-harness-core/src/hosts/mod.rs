//! Hosts and remote devices (ADR-0285, ADR-0299, ADR-0300): the single-use pairing codes this
//! service issues and the tokens it admits ([`pairing`]), and the hosts this one links to
//! ([`links`]). The tables live in the configuration store's SQLite file, beside its append-only
//! tables, which they never touch.

mod links;
mod pairing;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::failed;
use crate::util::hex;

pub(crate) use links::Links;
pub use links::{HostList, LinkedHostInfo};
pub use pairing::ClientInfo;
pub(crate) use pairing::{client, invite, pair, revoke};

/// The port the remote listener takes unless `meta remote on --port` names another (ADR-0299).
pub const DEFAULT_REMOTE_PORT: u16 = 3774;

/// The tables of hosts and devices, created with the configuration store's.
pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS pairing (
    code_hash TEXT PRIMARY KEY, kind TEXT NOT NULL, access TEXT NOT NULL,
    expires_ms INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS client (
    token_hash TEXT PRIMARY KEY, kind TEXT NOT NULL, access TEXT NOT NULL, name TEXT NOT NULL,
    created_ms INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS peer (
    name TEXT PRIMARY KEY, address TEXT NOT NULL, token TEXT NOT NULL, access TEXT NOT NULL,
    tunnel_port INTEGER NOT NULL, linked_ms INTEGER);
CREATE TABLE IF NOT EXISTS setting (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

/// What a client paired as. A host's token reaches only what T3's peer link needs; a device's
/// reaches the whole API.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    /// A device, such as Liquid on another computer.
    Device,
    /// Another host's Meta-Harness.
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

/// What a linked host's agents may do here, as T3 names the levels (ADR-0300). The levels are
/// ordered from the least to the most access.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Access {
    /// Reads only.
    ReadOnly,
    /// Asks before each change.
    ApprovalRequired,
    /// Accepts file edits without asking.
    AutoAcceptEdits,
    /// The broadest mode below full access, the default.
    #[default]
    Auto,
    /// Full access, as the receiving host's user, without the fences of the other levels.
    FullAccess,
}

impl Access {
    /// Every level, in the order T3 lists them.
    pub const ALL: [Self; 5] = [
        Self::ReadOnly,
        Self::ApprovalRequired,
        Self::AutoAcceptEdits,
        Self::Auto,
        Self::FullAccess,
    ];

    /// The name of this level, as T3's command line takes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::ApprovalRequired => "approval-required",
            Self::AutoAcceptEdits => "auto-accept-edits",
            Self::Auto => "auto",
            Self::FullAccess => "full-access",
        }
    }

    /// The level that `text` names.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|level| level.as_str() == text)
    }
}

/// A token of 256 random bits, in hex.
pub(crate) fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the operating system provides random bytes");
    hex(&bytes)
}

/// The digest a code or token is stored and compared by.
fn digest(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
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
