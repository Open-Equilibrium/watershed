//! The remote listener (ADR-0299): after `meta remote on`, the service also listens on the
//! Tailscale addresses, where other hosts and devices reach it. Its connections take the handshake
//! and sessions of the local listener (see [`super::admit`]), with the pairing and token rules of
//! [`super::Caller`].

use std::net::{IpAddr, SocketAddr};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use super::admit::Listener;
use super::{Shared, accept, to};
use crate::api::{RemoteCall, RemoteStatus};
use crate::hosts;
use crate::util::lock;

/// How often remote access that is on but does not listen binds again (ADR-0299).
const RETRY: Duration = Duration::from_secs(30);

/// Remote access as the service holds it: the port it is on at, and its listener while it listens.
#[derive(Default)]
pub(super) struct Remote {
    /// The port remote access is on at; it stays on across restarts (ADR-0299).
    port: Option<u16>,
    pub(super) listening: Option<Listening>,
    /// Why it is on but does not listen yet.
    waiting: Option<String>,
}

/// The remote listener while it is on. Dropping it stops its accept tasks.
pub(super) struct Listening {
    port: u16,
    addresses: Vec<SocketAddr>,
    tasks: Vec<JoinHandle<()>>,
    /// The connections it admitted watch this channel, which closes when the listener is dropped.
    ends: tokio::sync::watch::Sender<bool>,
}

impl Drop for Listening {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl Listening {
    fn status(&self) -> RemoteStatus {
        RemoteStatus {
            port: self.port,
            addresses: self.addresses.iter().map(|a| format!("ws://{a}")).collect(),
            waiting: None,
        }
    }
}

/// Binds `port` on each address of the remote listener's. The sockets are bound here and served by
/// [`serve_on`], which keeps this module's async calls out of the connections' own types.
async fn sockets(shared: &Shared, port: u16) -> Result<Vec<std::net::TcpListener>, String> {
    let ips = match &shared.remote_bind {
        Some(ips) => ips.clone(),
        None => tailscale_addresses().await?,
    };
    ips.into_iter()
        .map(|ip| {
            let socket = std::net::TcpListener::bind((ip, port))
                .and_then(|socket| socket.set_nonblocking(true).map(|()| socket))
                .map_err(|e| format!("Could not listen on {ip}:{port}: {e}"))?;
            Ok(socket)
        })
        .collect()
}

/// Serves each bound socket of the remote listener on `port`.
fn serve_on(
    shared: &Arc<Shared>,
    port: u16,
    sockets: Vec<std::net::TcpListener>,
) -> Result<Listening, String> {
    let (ends, _) = tokio::sync::watch::channel(false);
    let mut listening = Listening {
        port,
        addresses: Vec::new(),
        tasks: Vec::new(),
        ends,
    };
    for socket in sockets {
        listening.addresses.extend(socket.local_addr().ok());
        let listener = TcpListener::from_std(socket)
            .map_err(|e| format!("Could not serve the remote listener: {e}"))?;
        listening.tasks.push(tokio::spawn(accept(
            listener,
            shared.clone(),
            Listener::Remote,
            listening.ends.subscribe(),
        )));
    }
    Ok(listening)
}

/// Binds the remote listener on `port` and keeps it. A port it cannot take is kept as the reason
/// remote access does not listen yet, so `meta remote status` can say so.
pub(super) async fn start(shared: &Arc<Shared>, port: u16) -> Result<(), String> {
    let bound = match sockets(shared, port).await {
        Ok(sockets) => serve_on(shared, port, sockets),
        Err(error) => Err(error),
    };
    let mut remote = lock(&shared.remote);
    remote.port = Some(port);
    match bound {
        Ok(listening) => {
            remote.listening = Some(listening);
            remote.waiting = None;
            Ok(())
        }
        Err(error) => {
            remote.waiting = Some(error.clone());
            Err(error)
        }
    }
}

/// Binds the port that remote access is on at again every [`RETRY`], while its listener is not
/// bound (ADR-0299). The reason of a failure stays in `meta remote status`.
pub(super) async fn retry(shared: Arc<Shared>) {
    loop {
        tokio::time::sleep(RETRY).await;
        let _serial = shared.remote_serial.lock().await;
        let pending = {
            let remote = lock(&shared.remote);
            remote.port.filter(|_| remote.listening.is_none())
        };
        if let Some(port) = pending {
            let _ = start(&shared, port).await;
        }
    }
}

/// Turns remote access on at `port`, as `meta remote on` asks. A port already on is kept, and a
/// listener that cannot bind leaves the previous state as it was. Port 0 names no port to listen
/// on, so it is refused.
pub(super) async fn on(shared: &Arc<Shared>, port: u16) -> Result<RemoteStatus, String> {
    if port == 0 {
        return Err("Remote access needs a port from 1 to 65535.".to_owned());
    }
    let _serial = shared.remote_serial.lock().await;
    if let Some(status) = listening_status(shared).filter(|status| status.port == port) {
        return Ok(status);
    }
    let listening = serve_on(shared, port, sockets(shared, port).await?)?;
    let status = listening.status();
    hosts::set_remote_port(&shared.store.db(), Some(port))?;
    let mut remote = lock(&shared.remote);
    remote.port = Some(port);
    remote.listening = Some(listening);
    remote.waiting = None;
    Ok(status)
}

/// Turns remote access off, which stops the listener, ends its connections and keeps it off at
/// the next start.
pub(super) async fn off(shared: &Arc<Shared>) -> Result<(), String> {
    let _serial = shared.remote_serial.lock().await;
    hosts::set_remote_port(&shared.store.db(), None)?;
    *lock(&shared.remote) = Remote::default();
    Ok(())
}

fn listening_status(shared: &Shared) -> Option<RemoteStatus> {
    lock(&shared.remote)
        .listening
        .as_ref()
        .map(Listening::status)
}

/// Where the remote listener answers, while remote access is on: its addresses, or the reason it
/// does not listen yet.
pub(super) fn status(shared: &Shared) -> Option<RemoteStatus> {
    let remote = lock(&shared.remote);
    let port = remote.port?;
    Some(match &remote.listening {
        Some(listening) => listening.status(),
        None => RemoteStatus {
            port,
            addresses: Vec::new(),
            waiting: Some(
                remote
                    .waiting
                    .clone()
                    .unwrap_or_else(|| "it has not tried yet".to_owned()),
            ),
        },
    })
}

/// Answers a call to the remote listener.
pub(super) async fn call(shared: &Arc<Shared>, call: RemoteCall) -> Result<Value, String> {
    match call {
        RemoteCall::On { port } => to(on(shared, port).await?),
        RemoteCall::Off => to(off(shared).await?),
        RemoteCall::Status => to(status(shared)),
    }
}

/// The addresses Tailscale gives this computer, IPv4 and IPv6. Without any, remote access cannot
/// be reached from other devices, so it is an error that says to install Tailscale.
async fn tailscale_addresses() -> Result<Vec<IpAddr>, String> {
    let mut found = Vec::new();
    for family in ["-4", "-6"] {
        if let Some(text) = tailscale_output(&["ip", family]).await {
            found.extend(parse_addresses(&text));
        }
    }
    if found.is_empty() {
        return Err(
            "Remote access needs Tailscale: install Tailscale and sign in, so this computer has a \
             Tailscale address."
                .to_owned(),
        );
    }
    Ok(found)
}

/// The Tailscale command: on macOS also the app's, which a PATH without it does not find.
#[cfg(target_os = "macos")]
const TAILSCALE: [&str; 2] = [
    "tailscale",
    "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
];

#[cfg(not(target_os = "macos"))]
const TAILSCALE: [&str; 1] = ["tailscale"];

/// What `tailscale` with `args` prints, if one of its commands runs and succeeds.
async fn tailscale_output(args: &[&str]) -> Option<String> {
    for program in TAILSCALE {
        let mut command = tokio::process::Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let Ok(child) = crate::spawn(&mut command) else {
            continue;
        };
        // A Tailscale that hangs, such as one waiting for its daemon, counts as absent.
        let Ok(Ok(output)) =
            tokio::time::timeout(std::time::Duration::from_secs(30), child.wait_with_output())
                .await
        else {
            continue;
        };
        if output.status.success() {
            return Some(String::from_utf8_lossy(&output.stdout).into_owned());
        }
    }
    None
}

/// The Tailscale addresses in `text`, one per line; other lines and addresses are skipped.
fn parse_addresses(text: &str) -> Vec<IpAddr> {
    text.lines()
        .filter_map(|line| line.trim().parse::<IpAddr>().ok())
        .filter(in_tailscale)
        .collect()
}

/// Whether `ip` is in Tailscale's ranges, 100.64.0.0/10 and fd7a:115c:a1e0::/48 (ADR-0299): the
/// only addresses the remote listener binds, so it is never reachable on another network.
fn in_tailscale(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [first, second, ..] = ip.octets();
            first == 100 && (64..128).contains(&second)
        }
        IpAddr::V6(ip) => {
            let [a, b, c, ..] = ip.segments();
            (a, b, c) == (0xfd7a, 0x115c, 0xa1e0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tailscale_prints_one_address_per_line() {
        assert_eq!(
            parse_addresses("100.64.0.2\nfd7a:115c:a1e0::2\n\nnot one\n"),
            ["100.64.0.2", "fd7a:115c:a1e0::2"].map(|a| a.parse::<IpAddr>().unwrap())
        );
    }

    #[test]
    fn only_tailscale_addresses_are_listened_on() {
        assert_eq!(
            parse_addresses(
                "192.168.1.5\n100.128.0.1\nfe80::1\n100.127.255.255\nfd7a:115c:a1e0::9\n"
            ),
            ["100.127.255.255", "fd7a:115c:a1e0::9"].map(|a| a.parse::<IpAddr>().unwrap())
        );
    }
}
