//! The remote listener (ADR-0299): after `meta remote on`, the service also listens on the
//! Tailscale addresses, where other hosts and devices reach it. Its connections take the handshake
//! and sessions of the local listener (see [`super::admit`]), with the pairing and token rules of
//! [`super::Caller`].

use std::net::{IpAddr, SocketAddr};
use std::process::Stdio;
use std::sync::Arc;

use serde_json::Value;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use super::admit::Listener;
use super::{Shared, accept, to};
use crate::api::{RemoteCall, RemoteStatus};
use crate::hosts;

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

fn store(shared: &Shared, listening: Option<Listening>) {
    // The previous listener, if any, stops as it is replaced.
    *crate::util::lock(&shared.remote) = listening;
}

/// Listens on `port` at start, when remote access was on.
pub(super) async fn start(shared: &Arc<Shared>, port: u16) -> Result<(), String> {
    let listening = serve_on(shared, port, sockets(shared, port).await?)?;
    store(shared, Some(listening));
    Ok(())
}

/// Turns remote access on at `port`, as `meta remote on` asks. A port already on is kept, and
/// a listener that cannot bind leaves the previous state as it was.
pub(super) async fn on(shared: &Arc<Shared>, port: u16) -> Result<RemoteStatus, String> {
    if let Some(status) = status(shared).filter(|status| status.port == port) {
        return Ok(status);
    }
    let listening = serve_on(shared, port, sockets(shared, port).await?)?;
    let status = listening.status();
    hosts::set_remote_port(&shared.store.db(), Some(port))?;
    store(shared, Some(listening));
    Ok(status)
}

/// Turns remote access off, which stops the listener and keeps it off at the next start.
pub(super) fn off(shared: &Arc<Shared>) -> Result<(), String> {
    hosts::set_remote_port(&shared.store.db(), None)?;
    store(shared, None);
    Ok(())
}

/// Where the remote listener answers, while it is on.
pub(super) fn status(shared: &Shared) -> Option<RemoteStatus> {
    crate::util::lock(&shared.remote)
        .as_ref()
        .map(Listening::status)
}

/// Answers a call to the remote listener.
pub(super) async fn call(shared: &Arc<Shared>, call: RemoteCall) -> Result<Value, String> {
    match call {
        RemoteCall::On { port } => to(on(shared, port).await?),
        RemoteCall::Off => to(off(shared)?),
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
        let Ok(output) = child.wait_with_output().await else {
            continue;
        };
        if output.status.success() {
            return Some(String::from_utf8_lossy(&output.stdout).into_owned());
        }
    }
    None
}

/// The addresses in `text`, one per line; lines that are no address are skipped.
fn parse_addresses(text: &str) -> Vec<IpAddr> {
    text.lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect()
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
}
