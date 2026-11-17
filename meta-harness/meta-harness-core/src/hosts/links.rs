//! The hosts this one links to (ADR-0285, ADR-0288): each host's tunnel, which runs while the
//! service does, and the T3 link that uses it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use super::{Access, ClientInfo};
use crate::config::{Store, count, failed, now_ms, sql};
use crate::util::lock;
use crate::{remote, t3, tunnel};

/// How long after a link was made it is made again: T3 links last 30 days (ADR-0288).
const RELINK_AFTER_MS: u64 = 25 * 24 * 3_600_000;

/// A host this one links to: the address and host token of its remote listener, the access its
/// agents have there, and the local port its tunnel listens on.
struct LinkedHost {
    name: String,
    address: String,
    token: String,
    access: Access,
    tunnel_port: u16,
    linked_ms: Option<u64>,
}

/// A linked host, as `meta host list` shows it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LinkedHostInfo {
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

/// The hosts this one links to and the clients paired with it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct HostList {
    /// The hosts this one links to.
    pub hosts: Vec<LinkedHostInfo>,
    /// The clients paired with this one.
    pub clients: Vec<ClientInfo>,
}

fn linked_host_from(
    row: (String, String, String, String, i64, Option<i64>),
) -> Result<LinkedHost, String> {
    let (name, address, token, access, tunnel_port, linked_ms) = row;
    Ok(LinkedHost {
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

fn linked_host_row(
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

const LINKED_HOST_COLUMNS: &str = "name, address, token, access, tunnel_port, linked_ms";

fn linked(db: &Connection) -> Result<Vec<LinkedHost>, String> {
    let mut statement = db
        .prepare(&format!(
            "SELECT {LINKED_HOST_COLUMNS} FROM peer ORDER BY name"
        ))
        .map_err(failed)?;
    let rows = statement.query_map([], linked_host_row).map_err(failed)?;
    rows.map(|row| linked_host_from(row.map_err(failed)?))
        .collect()
}

fn linked_named(db: &Connection, name: &str) -> Result<Option<LinkedHost>, String> {
    db.query_row(
        &format!("SELECT {LINKED_HOST_COLUMNS} FROM peer WHERE name = ?1"),
        [name],
        linked_host_row,
    )
    .optional()
    .map_err(failed)?
    .map(linked_host_from)
    .transpose()
}

fn insert_linked_host(db: &Connection, host: &LinkedHost) -> Result<(), String> {
    if linked_named(db, &host.name)?.is_some() {
        return Err(format!("{} is linked already.", host.name));
    }
    db.execute(
        &format!("INSERT INTO peer ({LINKED_HOST_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6)"),
        params![
            host.name,
            host.address,
            host.token,
            host.access.as_str(),
            host.tunnel_port,
            host.linked_ms.map(sql)
        ],
    )
    .map(|_| ())
    .map_err(failed)
}

fn remove_linked_host(db: &Connection, name: &str) -> Result<(), String> {
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

/// The address of the tunnel that reaches the host linked on `port`, as T3 lists its links.
fn tunnel_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// Whether a link made at `linked`, or never, is due to be made again at `now`.
fn is_due(linked: Option<u64>, now: u64) -> bool {
    linked.is_none_or(|at| now.saturating_sub(at) > RELINK_AFTER_MS)
}

/// The hosts this one links to: each host's tunnel and the T3 link through it.
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
        let linked_hosts = linked(&store.db())?;
        for host in linked_hosts {
            match TcpListener::bind(("127.0.0.1", host.tunnel_port)).await {
                Ok(socket) => self.tunnel(socket, &host),
                Err(error) => eprintln!("meta: the tunnel to {} is off: {error}", host.name),
            }
        }
        Ok(())
    }

    /// Links the host at `address` (`ws://…`), which `code` pairs with, at `access`, or at the
    /// level the other host allows when none is asked for. Its tunnel starts, and its T3 links this
    /// one. A link that fails leaves nothing behind here, but the other host may keep the pairing,
    /// which its `meta host revoke` removes. Answers the host's name and the level it links at.
    pub(crate) async fn add(
        &self,
        store: &Store,
        address: &str,
        code: &str,
        access: Option<Access>,
    ) -> Result<(String, Access), String> {
        let paired = remote::pair(address, code, &self.name).await?;
        let access = access.unwrap_or(paired.access);
        let name = paired.name;
        match self
            .link_paired(store, address, &name, paired.token, access)
            .await
        {
            Ok(()) => Ok((name, access)),
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
        let host = LinkedHost {
            name: name.to_owned(),
            address: address.to_owned(),
            token,
            access,
            tunnel_port,
            linked_ms: None,
        };
        insert_linked_host(&store.db(), &host)?;
        self.tunnel(socket, &host);
        if let Err(error) = self.link_t3(&host).await {
            self.forget(store, name)?;
            return Err(error);
        }
        set_linked(&store.db(), name, now_ms())
    }

    /// Unlinks the host `name`: its T3 forgets the link, its tunnel stops and it is removed.
    pub(crate) async fn remove(&self, store: &Store, name: &str) -> Result<(), String> {
        let host = linked_named(&store.db(), name)?
            .ok_or_else(|| format!("There is no linked host named {name}."))?;
        let url = tunnel_url(host.tunnel_port);
        let t3_links = t3::links(&self.t3, &self.t3_home).await?;
        if let Some(link) = t3_links.iter().find(|link| link.urls.contains(&url)) {
            t3::unlink(&self.t3, &self.t3_home, &link.environment_id).await?;
        }
        self.forget(store, name)
    }

    /// The linked hosts and the clients paired with this host, with whether each host's T3
    /// answers through its tunnel now.
    pub(crate) async fn list(&self, store: &Store) -> Result<HostList, String> {
        let t3_links = t3::links(&self.t3, &self.t3_home).await?;
        let hosts = linked(&store.db())?
            .into_iter()
            .map(|host| {
                let url = tunnel_url(host.tunnel_port);
                LinkedHostInfo {
                    reachable: t3_links
                        .iter()
                        .any(|link| link.status == "reachable" && link.urls.contains(&url)),
                    name: host.name,
                    address: host.address,
                    access: host.access,
                    linked_ms: host.linked_ms,
                }
            })
            .collect();
        Ok(HostList {
            hosts,
            clients: super::pairing::clients(&store.db())?,
        })
    }

    /// Links again each host whose link is due (see [`is_due`]). A host that does not answer is
    /// tried again at the next pass, and reported meanwhile.
    pub(crate) async fn relink_due(&self, store: &Store) {
        let due: Vec<LinkedHost> = match linked(&store.db()) {
            Ok(hosts) => hosts
                .into_iter()
                .filter(|host| is_due(host.linked_ms, now_ms()))
                .collect(),
            Err(error) => {
                eprintln!("meta: could not read the linked hosts: {error}");
                return;
            }
        };
        for host in due {
            match self.link_t3(&host).await {
                Ok(()) => {
                    if let Err(error) = set_linked(&store.db(), &host.name, now_ms()) {
                        eprintln!("meta: could not record the link to {}: {error}", host.name);
                    }
                }
                Err(error) => eprintln!("meta: could not link {} again: {error}", host.name),
            }
        }
    }

    /// Stops every tunnel, as the service stops.
    pub(crate) fn stop(&self) {
        for (_, task) in lock(&self.tunnels).drain() {
            task.abort();
        }
    }

    /// A pairing code from this host's T3 Code, which another host's link redeems at `access`
    /// (ADR-0288, ADR-0300).
    pub(crate) async fn pairing_code(&self, access: Access) -> Result<String, String> {
        t3::pairing_code(&self.t3, &self.t3_home, access).await
    }

    /// Asks the peer for a pairing code over its host token and has T3 link through the tunnel.
    async fn link_t3(&self, host: &LinkedHost) -> Result<(), String> {
        let other = remote::Host::connect_to(&host.address, &host.token, &self.name).await?;
        let code = other.t3_pairing().await?;
        t3::link(
            &self.t3,
            &self.t3_home,
            &tunnel_url(host.tunnel_port),
            &code,
            host.access.as_str(),
        )
        .await
    }

    /// Runs the tunnel of `host` on `socket` until it is removed.
    fn tunnel(&self, socket: TcpListener, host: &LinkedHost) {
        let task = tokio::spawn(tunnel::listen(
            socket,
            host.address.clone(),
            host.token.clone(),
        ));
        if let Some(earlier) = lock(&self.tunnels).insert(host.name.clone(), task) {
            earlier.abort();
        }
    }

    /// Stops the tunnel of the host `name` and forgets it here.
    fn forget(&self, store: &Store, name: &str) -> Result<(), String> {
        if let Some(task) = lock(&self.tunnels).remove(name) {
            task.abort();
        }
        remove_linked_host(&store.db(), name)
    }
}
