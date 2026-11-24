use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use super::{Access, ClientInfo, fitted, printable, random};
use crate::store::{Store, count, failed, now_ms, sql};
use crate::util::{TIMEOUT, hex, lock};
use crate::{remote, t3, tunnel};

const RELINK_AFTER_MS: u64 = 25 * 24 * 3_600_000;

#[derive(Clone)]
struct LinkedHost {
    name: String,
    address: String,
    token: String,
    access: Access,
    tunnel_port: u16,
    linked_ms: Option<u64>,
    client: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LinkedHostInfo {
    pub name: String,
    pub client: String,
    pub address: String,
    pub access: Access,
    pub linked_ms: Option<u64>,
    pub reachable: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct HostList {
    pub hosts: Vec<LinkedHostInfo>,
    pub clients: Vec<ClientInfo>,
}

type LinkedHostRow = (String, String, String, String, i64, Option<i64>, String);

fn linked_host_from(row: LinkedHostRow) -> Result<LinkedHost, String> {
    let (name, address, token, access, tunnel_port, linked_ms, client) = row;
    Ok(LinkedHost {
        name,
        address,
        token,
        access: Access::parse(&access)
            .ok_or("The Meta-Harness store holds an unknown access level.")?,
        tunnel_port: u16::try_from(tunnel_port)
            .map_err(|_| "The Meta-Harness store holds a bad tunnel port.")?,
        linked_ms: linked_ms.map(count),
        client,
    })
}

fn linked_host_row(row: &rusqlite::Row) -> rusqlite::Result<LinkedHostRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ))
}

const LINKED_HOST_COLUMNS: &str = "name, address, token, access, tunnel_port, linked_ms, client";

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
        &format!("INSERT INTO peer ({LINKED_HOST_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"),
        params![
            host.name,
            host.address,
            host.token,
            host.access.as_str(),
            host.tunnel_port,
            host.linked_ms.map(sql),
            host.client
        ],
    )
    .map(|_| ())
    .map_err(failed)
}

fn linked_at(db: &Connection, address: &str) -> Result<Option<String>, String> {
    db.query_row(
        "SELECT name FROM peer WHERE address = ?1",
        [address],
        |row| row.get(0),
    )
    .optional()
    .map_err(failed)
}

fn pairing_name(host: &str) -> String {
    format!("{}-{}", fitted(host, 59, 95), hex(&random::<2>()))
}

fn remove_linked_host(db: &Connection, name: &str) -> Result<(), String> {
    db.execute("DELETE FROM peer WHERE name = ?1", [name])
        .map(|_| ())
        .map_err(failed)
}

fn set_linked(db: &Connection, name: &str, at: Option<u64>) -> Result<(), String> {
    db.execute(
        "UPDATE peer SET linked_ms = ?2 WHERE name = ?1",
        params![name, at.map(sql)],
    )
    .map(|_| ())
    .map_err(failed)
}

fn tunnel_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

async fn loopback() -> Result<(TcpListener, u16), String> {
    let socket = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Could not listen on the loopback: {e}"))?;
    let port = socket
        .local_addr()
        .map_err(|e| format!("Could not listen on the loopback: {e}"))?
        .port();
    Ok((socket, port))
}

fn record_link(store: &Store, name: &str) {
    if let Err(error) = set_linked(&store.db(), name, Some(now_ms())) {
        eprintln!("meta: could not record the link to {name}: {error}");
    }
}

fn is_due(linked: Option<u64>, now: u64) -> bool {
    linked.is_none_or(|at| now.saturating_sub(at) > RELINK_AFTER_MS)
}

pub(crate) struct Links {
    name: String,
    t3: PathBuf,
    t3_home: PathBuf,
    tunnels: Mutex<HashMap<String, JoinHandle<()>>>,
    serial: tokio::sync::Mutex<()>,
    pairing: tokio::sync::Semaphore,
}

impl Links {
    pub(crate) fn new(name: String, t3: PathBuf, t3_home: PathBuf) -> Self {
        Self {
            name,
            t3,
            t3_home,
            tunnels: Mutex::default(),
            serial: tokio::sync::Mutex::default(),
            pairing: tokio::sync::Semaphore::new(1),
        }
    }

    pub(crate) async fn start_tunnels(&self, store: &Store) -> Result<(), String> {
        let linked_hosts = linked(&store.db())?;
        for host in linked_hosts {
            if let Err(error) = self.ensure_tunnel(store, &host).await {
                eprintln!("meta: the tunnel to {} is off: {error}", host.name);
            }
        }
        Ok(())
    }

    async fn ensure_tunnel(&self, store: &Store, host: &LinkedHost) -> Result<LinkedHost, String> {
        if lock(&self.tunnels)
            .get(&host.name)
            .is_some_and(|task| !task.is_finished())
        {
            return Ok(host.clone());
        }
        match TcpListener::bind(("127.0.0.1", host.tunnel_port)).await {
            Ok(socket) => {
                self.tunnel(socket, host);
                Ok(host.clone())
            }
            Err(error) => self
                .move_tunnel(store, host)
                .await
                .map_err(|moving| format!("{error}; {moving}")),
        }
    }

    async fn move_tunnel(&self, store: &Store, host: &LinkedHost) -> Result<LinkedHost, String> {
        let (socket, port) = loopback().await?;
        self.unlink_url(tunnel_url(host.tunnel_port)).await?;
        store
            .db()
            .execute(
                "UPDATE peer SET tunnel_port = ?2, linked_ms = NULL WHERE name = ?1",
                params![host.name, port],
            )
            .map_err(failed)?;
        let moved = LinkedHost {
            tunnel_port: port,
            linked_ms: None,
            ..host.clone()
        };
        self.tunnel(socket, &moved);
        Ok(moved)
    }

    pub(crate) async fn add(
        &self,
        store: &Store,
        address: &str,
        code: &str,
        access: Option<Access>,
    ) -> Result<(String, Access), String> {
        remote::check_address(address)?;
        let _serial = self.serial.lock().await;
        if let Some(name) = linked_at(&store.db(), address)? {
            return Err(format!(
                "{address} is linked already, as {name}; \"meta host remove {name}\" unlinks it first."
            ));
        }
        let client = pairing_name(&self.name);
        let paired = remote::pair(address, code, &client).await?;
        let name = printable(&paired.name);
        let access = match access {
            Some(asked) if asked > paired.access => {
                return Err(format!(
                    "{name} invited this host at {}; ask for that or less. The pairing it made \
                     stays until \"meta host revoke {client}\" runs on {name}.",
                    paired.access.as_str()
                ));
            }
            Some(asked) => asked,
            None => paired.access,
        };
        match self
            .link_paired(store, address, &name, paired.token, access, &client)
            .await
        {
            Ok(()) => Ok((name, access)),
            Err(error) => Err(format!(
                "{error} The other host, {name}, may keep a paired entry for this host; \
                 \"meta host revoke {client}\" there removes it."
            )),
        }
    }

    async fn link_paired(
        &self,
        store: &Store,
        address: &str,
        name: &str,
        token: String,
        access: Access,
        client: &str,
    ) -> Result<(), String> {
        let (socket, tunnel_port) = loopback().await?;
        let host = LinkedHost {
            name: name.to_owned(),
            address: address.to_owned(),
            token,
            access,
            tunnel_port,
            linked_ms: None,
            client: client.to_owned(),
        };
        insert_linked_host(&store.db(), &host)?;
        self.tunnel(socket, &host);
        if let Err(error) = self.link_t3(&host).await {
            self.forget(store, name)?;
            return Err(error);
        }
        record_link(store, name);
        Ok(())
    }

    pub(crate) async fn remove(&self, store: &Store, name: &str) -> Result<String, String> {
        let _serial = self.serial.lock().await;
        let host = linked_named(&store.db(), name)?
            .ok_or_else(|| format!("There is no linked host named {name}."))?;
        self.unlink_url(tunnel_url(host.tunnel_port)).await?;
        if let Err(error) = self.forget(store, name) {
            if let Err(stored) = set_linked(&store.db(), name, None) {
                eprintln!("meta: could not mark {name} for linking again: {stored}");
            }
            return Err(error);
        }
        Ok(host.client)
    }

    pub(crate) async fn end_sessions(&self, client: &str) -> Result<usize, String> {
        let mut ended = 0;
        for session in t3::sessions(&self.t3, &self.t3_home).await? {
            if session.is_link_of(client) {
                t3::revoke_session(&self.t3, &self.t3_home, &session.session_id).await?;
                ended += 1;
            }
        }
        Ok(ended)
    }

    pub(crate) async fn list(&self, store: &Store) -> Result<HostList, String> {
        let t3_links = t3::links(&self.t3, &self.t3_home).await?;
        let hosts = linked(&store.db())?
            .into_iter()
            .map(|host| {
                let url = tunnel_url(host.tunnel_port);
                LinkedHostInfo {
                    client: host.client.clone(),
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

    pub(crate) async fn relink_due(&self, store: &Store) {
        let _serial = self.serial.lock().await;
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
            let linked = match self.ensure_tunnel(store, &host).await {
                Ok(tunneled) => self.link_t3(&tunneled).await,
                Err(error) => Err(error),
            };
            match linked {
                Ok(()) => record_link(store, &host.name),
                Err(error) => eprintln!("meta: could not link {} again: {error}", host.name),
            }
        }
    }

    pub(crate) fn stop(&self) {
        for (_, task) in lock(&self.tunnels).drain() {
            task.abort();
        }
    }

    pub(crate) async fn pairing_code(&self, access: Access) -> Result<String, String> {
        let _slot = self.pairing.try_acquire().map_err(|_| {
            "Another pairing code is still being made; try again in a moment.".to_owned()
        })?;
        t3::pairing_code(&self.t3, &self.t3_home, access).await
    }

    async fn unlink_url(&self, url: String) -> Result<(), String> {
        let t3_links = t3::links(&self.t3, &self.t3_home).await?;
        if let Some(link) = t3_links.iter().find(|link| link.urls.contains(&url)) {
            t3::unlink(&self.t3, &self.t3_home, &link.environment_id).await?;
        }
        Ok(())
    }

    async fn link_t3(&self, host: &LinkedHost) -> Result<(), String> {
        let code = tokio::time::timeout(TIMEOUT, async {
            let other = remote::Host::connect_to(&host.address, &host.token, &self.name).await?;
            other.t3_pairing().await
        })
        .await
        .map_err(|_| format!("{} did not answer in time.", host.name))??;
        t3::link(
            &self.t3,
            &self.t3_home,
            &tunnel_url(host.tunnel_port),
            &code,
            host.access.as_str(),
        )
        .await
    }

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

    fn forget(&self, store: &Store, name: &str) -> Result<(), String> {
        remove_linked_host(&store.db(), name)?;
        if let Some(task) = lock(&self.tunnels).remove(name) {
            task.abort();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        Access, LinkedHost, Links, RELINK_AFTER_MS, insert_linked_host, is_due, linked_named,
    };
    use crate::store::Store;
    #[cfg(unix)]
    use crate::tests::{Dir, executable};
    use crate::util::until;

    #[test]
    fn a_link_is_made_again_only_once_it_is_older_than_25_days() {
        let now = 100 * RELINK_AFTER_MS;
        assert!(is_due(None, now));
        assert!(!is_due(Some(now - RELINK_AFTER_MS), now));
        assert!(is_due(Some(now - RELINK_AFTER_MS - 1), now));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_pairing_code_is_made_one_at_a_time_and_a_second_request_is_refused_meanwhile() {
        use std::time::Duration;
        let dir = Dir::new();
        let binary = dir.join("t3");
        executable(
            &binary,
            "#!/bin/sh\n: > \"$T3CODE_HOME/started\"\nsleep 1\nprintf '{\"credential\":\"CODE\"}'\n",
        );
        let links = Arc::new(Links::new("host".to_owned(), binary, dir.to_path_buf()));
        let first = tokio::spawn({
            let links = links.clone();
            async move { links.pairing_code(Access::ReadOnly).await }
        });
        assert!(
            until(Duration::from_secs(10), Duration::from_millis(10), || {
                dir.join("started").exists()
            })
            .await,
            "T3 started the first pairing code"
        );
        let refused = links.pairing_code(Access::ReadOnly).await;
        assert!(refused.is_err(), "{refused:?}");
        assert_eq!(first.await.unwrap(), Ok("CODE".to_owned()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_host_is_linked_only_once_its_tunnel_listens_again() {
        use futures_util::{SinkExt, StreamExt};
        use serde_json::json;
        use tokio_tungstenite::tungstenite::Message;

        let dir = Dir::new();
        let script = dir.join("t3");
        executable(
            &script,
            "#!/bin/sh\nif [ \"$2\" = list ]; then\n[ -e \"$T3CODE_HOME/down\" ] && exit 1\necho '[]'\nelse\necho \"$3\" > \"$T3CODE_HOME/linked\"\nfi\n",
        );
        let peer = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("ws://{}", peer.local_addr().unwrap());
        tokio::spawn(async move {
            let (socket, _) = peer.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let request: serde_json::Value = serde_json::from_str(&text).unwrap();
                let reply = json!({"id": request["id"], "ok": "CODE"});
                socket
                    .send(Message::Text(reply.to_string().into()))
                    .await
                    .unwrap();
            }
        });
        let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = occupied.local_addr().unwrap().port();
        let store = Store::open(&dir.join("meta-harness.sqlite")).unwrap();
        let host = LinkedHost {
            name: "peer".to_owned(),
            address,
            token: "token".to_owned(),
            access: Access::ReadOnly,
            tunnel_port: port,
            linked_ms: None,
            client: "host-1a2b".to_owned(),
        };
        insert_linked_host(&store.db(), &host).unwrap();
        let links = Links::new("host".to_owned(), script, dir.to_path_buf());
        std::fs::write(dir.join("down"), "").unwrap();
        links.start_tunnels(&store).await.unwrap();
        links.relink_due(&store).await;
        assert!(
            !dir.join("linked").exists(),
            "no link while the tunnel is off"
        );
        std::fs::remove_file(dir.join("down")).unwrap();
        links.relink_due(&store).await;
        let moved = linked_named(&store.db(), "peer")
            .unwrap()
            .unwrap()
            .tunnel_port;
        assert_ne!(moved, port, "the tunnel moved off the occupied port");
        let linked = std::fs::read_to_string(dir.join("linked")).unwrap();
        assert_eq!(linked.trim(), format!("http://127.0.0.1:{moved}"));
    }
}
