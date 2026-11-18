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

const RETRY: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(super) struct Remote {
    port: Option<u16>,
    pub(super) listening: Option<Listening>,
    waiting: Option<String>,
}

pub(super) struct Listening {
    port: u16,
    addresses: Vec<SocketAddr>,
    tasks: Vec<JoinHandle<()>>,
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
    async fn stop(mut self) {
        for task in self.tasks.drain(..) {
            task.abort();
            let _ = task.await;
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

pub(super) async fn off(shared: &Arc<Shared>) -> Result<(), String> {
    let _serial = shared.remote_serial.lock().await;
    hosts::set_remote_port(&shared.store.db(), None)?;
    let stopped = std::mem::take(&mut *lock(&shared.remote));
    if let Some(listening) = stopped.listening {
        listening.stop().await;
    }
    Ok(())
}

fn listening_status(shared: &Shared) -> Option<RemoteStatus> {
    lock(&shared.remote)
        .listening
        .as_ref()
        .map(Listening::status)
}

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

pub(super) async fn call(shared: &Arc<Shared>, call: RemoteCall) -> Result<Value, String> {
    match call {
        RemoteCall::On { port } => to(on(shared, port).await?),
        RemoteCall::Off => to(off(shared).await?),
        RemoteCall::Status => to(status(shared)),
    }
}

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

#[cfg(target_os = "macos")]
const TAILSCALE: [&str; 2] = [
    "tailscale",
    "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
];

#[cfg(not(target_os = "macos"))]
const TAILSCALE: [&str; 1] = ["tailscale"];

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

fn parse_addresses(text: &str) -> Vec<IpAddr> {
    text.lines()
        .filter_map(|line| line.trim().parse::<IpAddr>().ok())
        .filter(in_tailscale)
        .collect()
}

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
