//! Hosts and remote devices (ADR-0285, ADR-0299, ADR-0300): the single-use pairing codes this
//! service issues, the tokens it admits and what each token may do. The tables live in the
//! configuration store's SQLite file, beside its append-only tables, which they never touch.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::config::{Store, count, failed, now_ms, sql};
use crate::{remote, t3, tunnel};

/// The port the remote listener takes unless `meta remote on --port` names another (ADR-0299).
pub const DEFAULT_REMOTE_PORT: u16 = 3774;

/// How long after a link was made it is made again: T3 links last 30 days (ADR-0288).
const RELINK_AFTER_MS: u64 = 25 * 24 * 3_600_000;

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

/// The kind of client that `token` belongs to and the name it paired with, if a client holds it.
pub(crate) fn client(db: &Connection, token: &str) -> Result<Option<(Kind, String)>, String> {
    let wanted = digest(token);
    let mut statement = db
        .prepare("SELECT token_hash, kind, name FROM client")
        .map_err(failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(failed)?;
    let mut found = None;
    for row in rows {
        let (hash, kind, name) = row.map_err(failed)?;
        if same(hash.as_bytes(), wanted.as_bytes()) {
            found = Kind::parse(&kind).map(|kind| (kind, name));
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

/// What a linked host's agents may do here, as T3 names the levels (ADR-0300).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
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

/// A host this one links to: the address and host token of its remote listener, the access its
/// agents have there, and the local port its tunnel listens on.
struct Peer {
    name: String,
    address: String,
    token: String,
    access: Access,
    tunnel_port: u16,
    linked_ms: Option<u64>,
}

/// A linked host, as `meta host list` shows it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PeerInfo {
    /// The name the host answers to.
    pub name: String,
    /// Its remote listener's address, as `ws://<ip>:<port>`.
    pub address: String,
    /// The access its agents have here.
    pub access: Access,
    /// When its T3 link was made, in milliseconds since the Unix epoch.
    pub linked_ms: Option<u64>,
    /// Whether its T3 answers through the tunnel now.
    pub reachable: bool,
}

/// A client paired with this host, as `meta host list` shows it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    /// The name the client paired with.
    pub name: String,
    /// What it paired as.
    pub kind: Kind,
    /// When it paired, in milliseconds since the Unix epoch.
    pub since_ms: u64,
}

/// The hosts this one links to and the clients paired with it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct HostList {
    /// The hosts this one links to.
    pub peers: Vec<PeerInfo>,
    /// The clients paired with this one.
    pub clients: Vec<ClientInfo>,
}

fn peer_from(row: (String, String, String, String, i64, Option<i64>)) -> Result<Peer, String> {
    let (name, address, token, access, tunnel_port, linked_ms) = row;
    Ok(Peer {
        name,
        address,
        token,
        access: Access::parse(&access)
            .ok_or("The configuration store holds an unknown access level.")?,
        tunnel_port: u16::try_from(tunnel_port)
            .map_err(|_| "The configuration store holds a bad tunnel port.")?,
        linked_ms: linked_ms.map(count),
    })
}

fn peer_row(
    row: &rusqlite::Row,
) -> rusqlite::Result<(String, String, String, String, i64, Option<i64>)> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}

const PEER_COLUMNS: &str = "name, address, token, access, tunnel_port, linked_ms";

fn peers(db: &Connection) -> Result<Vec<Peer>, String> {
    let mut statement = db
        .prepare(&format!("SELECT {PEER_COLUMNS} FROM peer ORDER BY name"))
        .map_err(failed)?;
    let rows = statement.query_map([], peer_row).map_err(failed)?;
    rows.map(|row| peer_from(row.map_err(failed)?)).collect()
}

fn peer_named(db: &Connection, name: &str) -> Result<Option<Peer>, String> {
    db.query_row(
        &format!("SELECT {PEER_COLUMNS} FROM peer WHERE name = ?1"),
        [name],
        peer_row,
    )
    .optional()
    .map_err(failed)?
    .map(peer_from)
    .transpose()
}

fn insert_peer(db: &Connection, peer: &Peer) -> Result<(), String> {
    if peer_named(db, &peer.name)?.is_some() {
        return Err(format!("{} is linked already.", peer.name));
    }
    db.execute(
        &format!("INSERT INTO peer ({PEER_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6)"),
        params![
            peer.name,
            peer.address,
            peer.token,
            peer.access.as_str(),
            peer.tunnel_port,
            peer.linked_ms.map(sql)
        ],
    )
    .map(|_| ())
    .map_err(failed)
}

fn remove_peer(db: &Connection, name: &str) -> Result<(), String> {
    db.execute("DELETE FROM peer WHERE name = ?1", [name])
        .map(|_| ())
        .map_err(failed)
}

fn set_linked(db: &Connection, name: &str, at: u64) -> Result<(), String> {
    db.execute(
        "UPDATE peer SET linked_ms = ?2 WHERE name = ?1",
        params![name, sql(at)],
    )
    .map(|_| ())
    .map_err(failed)
}

fn clients(db: &Connection) -> Result<Vec<ClientInfo>, String> {
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
            kind: Kind::parse(&kind)
                .ok_or("The configuration store holds an unknown client kind.")?,
            since_ms: count(since),
        })
    })
    .collect()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The address of the tunnel that reaches the host linked on `port`, as T3 lists its links.
fn tunnel_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// Whether a link made at `linked`, or never, is due to be made again at `now`.
fn is_due(linked: Option<u64>, now: u64) -> bool {
    linked.is_none_or(|at| now.saturating_sub(at) > RELINK_AFTER_MS)
}

/// The hosts this one links to (ADR-0285, ADR-0288): each host's tunnel, which runs while the
/// service does, and the T3 link that uses it.
pub(crate) struct Links {
    /// This host's name for the others.
    name: String,
    /// The pinned `t3` executable and T3's data folder.
    t3: PathBuf,
    t3_home: PathBuf,
    /// The tunnel listeners by host name; a host's tunnel stops when it is removed.
    tunnels: Mutex<HashMap<String, JoinHandle<()>>>,
}

impl Links {
    /// Links for a host named `name` whose T3 is `t3` in `t3_home`.
    pub(crate) fn new(name: String, t3: PathBuf, t3_home: PathBuf) -> Self {
        Self {
            name,
            t3,
            t3_home,
            tunnels: Mutex::default(),
        }
    }

    /// Starts the tunnels of the hosts already linked, as the service starts. A tunnel whose port
    /// is taken is reported, and its host stays linked until it is removed.
    pub(crate) async fn start_tunnels(&self, store: &Store) -> Result<(), String> {
        let linked = peers(&store.db())?;
        for peer in linked {
            match TcpListener::bind(("127.0.0.1", peer.tunnel_port)).await {
                Ok(socket) => self.tunnel(socket, &peer),
                Err(error) => eprintln!("meta: the tunnel to {} is off: {error}", peer.name),
            }
        }
        Ok(())
    }

    /// Links the host at `address` (`ws://…`), which `code` pairs with, at `access`: its tunnel
    /// starts, and its T3 links this one. A link that fails leaves nothing behind here, but the
    /// other host may keep the pairing, which its `meta host revoke` removes.
    pub(crate) async fn add(
        &self,
        store: &Store,
        address: &str,
        code: &str,
        access: Access,
    ) -> Result<String, String> {
        let (name, token) = remote::pair(address, code, &self.name).await?;
        match self.link_paired(store, address, &name, token, access).await {
            Ok(()) => Ok(name),
            Err(error) => Err(format!(
                "{error} The other host, {name}, may keep a paired entry for this host; \
                 `meta host revoke {}` there removes it.",
                self.name
            )),
        }
    }

    /// Stores the host `name` that paired with `token`, starts its tunnel and has T3 link to it.
    async fn link_paired(
        &self,
        store: &Store,
        address: &str,
        name: &str,
        token: String,
        access: Access,
    ) -> Result<(), String> {
        let socket = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("Could not listen on the loopback: {e}"))?;
        let tunnel_port = socket
            .local_addr()
            .map_err(|e| format!("Could not listen on the loopback: {e}"))?
            .port();
        let peer = Peer {
            name: name.to_owned(),
            address: address.to_owned(),
            token,
            access,
            tunnel_port,
            linked_ms: None,
        };
        insert_peer(&store.db(), &peer)?;
        self.tunnel(socket, &peer);
        if let Err(error) = self.link_t3(&peer).await {
            self.forget(store, name)?;
            return Err(error);
        }
        set_linked(&store.db(), name, now_ms())
    }

    /// Unlinks the host `name`: its T3 forgets the link, its tunnel stops and it is removed.
    pub(crate) async fn remove(&self, store: &Store, name: &str) -> Result<(), String> {
        let peer = peer_named(&store.db(), name)?
            .ok_or_else(|| format!("There is no linked host named {name}."))?;
        let url = tunnel_url(peer.tunnel_port);
        let linked = t3::links(&self.t3, &self.t3_home).await?;
        if let Some(link) = linked.iter().find(|link| link.urls.contains(&url)) {
            t3::unlink(&self.t3, &self.t3_home, &link.environment_id).await?;
        }
        self.forget(store, name)
    }

    /// The linked hosts and the clients paired with this host, with whether each host's T3
    /// answers through its tunnel now.
    pub(crate) async fn list(&self, store: &Store) -> Result<HostList, String> {
        let linked = t3::links(&self.t3, &self.t3_home).await?;
        let linked_peers = peers(&store.db())?;
        let peers = linked_peers
            .into_iter()
            .map(|peer| {
                let url = tunnel_url(peer.tunnel_port);
                PeerInfo {
                    reachable: linked
                        .iter()
                        .any(|link| link.status == "reachable" && link.urls.contains(&url)),
                    name: peer.name,
                    address: peer.address,
                    access: peer.access,
                    linked_ms: peer.linked_ms,
                }
            })
            .collect();
        Ok(HostList {
            peers,
            clients: clients(&store.db())?,
        })
    }

    /// Links again each host whose link is due (see [`is_due`]). A host that does not answer is
    /// tried again at the next pass, and reported meanwhile.
    pub(crate) async fn relink_due(&self, store: &Store) {
        let due: Vec<Peer> = match peers(&store.db()) {
            Ok(linked) => linked
                .into_iter()
                .filter(|peer| is_due(peer.linked_ms, now_ms()))
                .collect(),
            Err(error) => {
                eprintln!("meta: could not read the linked hosts: {error}");
                return;
            }
        };
        for peer in due {
            match self.link_t3(&peer).await {
                Ok(()) => {
                    if let Err(error) = set_linked(&store.db(), &peer.name, now_ms()) {
                        eprintln!("meta: could not record the link to {}: {error}", peer.name);
                    }
                }
                Err(error) => eprintln!("meta: could not link {} again: {error}", peer.name),
            }
        }
    }

    /// Stops every tunnel, as the service stops.
    pub(crate) fn stop(&self) {
        for (_, task) in lock(&self.tunnels).drain() {
            task.abort();
        }
    }

    /// Asks the peer for a pairing code over its host token and has T3 link through the tunnel.
    async fn link_t3(&self, peer: &Peer) -> Result<(), String> {
        let other = remote::Host::connect_to(&peer.address, &peer.token, &self.name).await?;
        let code = other.t3_pairing().await?;
        t3::link(
            &self.t3,
            &self.t3_home,
            &tunnel_url(peer.tunnel_port),
            &code,
            peer.access.as_str(),
        )
        .await
    }

    /// Runs the tunnel of `peer` on `socket` until it is removed.
    fn tunnel(&self, socket: TcpListener, peer: &Peer) {
        let task = tokio::spawn(tunnel::listen(
            socket,
            peer.address.clone(),
            peer.token.clone(),
        ));
        if let Some(earlier) = lock(&self.tunnels).insert(peer.name.clone(), task) {
            earlier.abort();
        }
    }

    /// Stops the tunnel of the host `name` and forgets it here.
    fn forget(&self, store: &Store, name: &str) -> Result<(), String> {
        if let Some(task) = lock(&self.tunnels).remove(name) {
            task.abort();
        }
        remove_peer(&store.db(), name)
    }
}
