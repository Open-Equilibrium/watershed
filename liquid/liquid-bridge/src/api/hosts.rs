//! Agent hosts other than this computer's (ADR-0295, ADR-0299). Liquid pairs with one through its
//! address and a pairing code, under a name of its own that names this computer, files the device
//! token the host issues under the address in the keychain, and on removal forgets that token here
//! only. Which hosts are saved, and which one is active, is Liquid's settings file, which never
//! holds a token.

use std::process::Command;

use meta_harness_core::remote;

use super::session::{AgentHost, run};
use crate::keys::{DeviceTokens, Tokens};

/// What pairing with an agent host gave: the host's name, and the name this device pairs under,
/// which `meta host revoke` names to remove it there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePairing {
    /// The host's name.
    pub host: String,
    /// This device's name on the host.
    pub client: String,
}

/// Whether an agent host answers, as Settings shows it. A check that fails for another reason, such
/// as a missing token, is an error that names the reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeReach {
    /// It answers.
    Connected,
    /// Its Meta-Harness service is not running or cannot be reached.
    Offline,
}

/// Pairs with the agent host at `address` (`ws://…`) using `code`, its pairing code from
/// `meta invite --device`, under this computer's name. Files the token it issues under `address`.
/// Nothing is kept when pairing fails.
pub fn pair_agent_host(address: String, code: String) -> Result<BridgePairing, String> {
    let client = client_name(&computer_name())?;
    pair_with(&DeviceTokens {}, &address, &code, &client)
}

/// The name Liquid pairs under on an agent host: `liquid-` and this computer's name in lower case,
/// each run of other characters one hyphen, so `meta host revoke` names exactly this device.
pub(crate) fn client_name(computer: &str) -> Result<String, String> {
    let words: Vec<String> = computer
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    if words.is_empty() {
        return Err("This computer has no name to pair under.".into());
    }
    Ok(format!("liquid-{}", words.join("-")))
}

/// This computer's name as its operating system shows it: the computer name on macOS, the host
/// name on Linux and the computer name on Windows. Empty when the system does not show one.
fn computer_name() -> String {
    if cfg!(target_os = "macos") {
        Command::new("scutil")
            .args(["--get", "ComputerName"])
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default()
    } else if cfg!(windows) {
        std::env::var("COMPUTERNAME").unwrap_or_default()
    } else {
        std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default()
    }
}

/// Forgets the device token filed under `address`. The host keeps this device paired until its
/// own `meta host revoke` removes it.
pub fn forget_agent_host(address: String) -> Result<(), String> {
    forget_with(&DeviceTokens {}, &address)
}

/// Whether this computer's Meta-Harness service answers; `home` is its data folder, else the
/// default one.
pub fn reach_local_agent_host(home: Option<String>) -> Result<BridgeReach, String> {
    reach(AgentHost::connect(home).map(drop))
}

/// Whether the agent host at `address` answers with the token filed under it.
pub fn reach_remote_agent_host(address: String) -> Result<BridgeReach, String> {
    reach_remote_with(&DeviceTokens {}, &address)
}

fn reach(connected: Result<(), String>) -> Result<BridgeReach, String> {
    match connected {
        Ok(()) => Ok(BridgeReach::Connected),
        Err(error) if remote::connection_lost(&error) => Ok(BridgeReach::Offline),
        Err(error) => Err(error),
    }
}

pub(crate) fn pair_with(
    tokens: &dyn Tokens,
    address: &str,
    code: &str,
    client: &str,
) -> Result<BridgePairing, String> {
    if tokens.read(address)?.is_some() {
        return Err(format!(
            "Liquid already holds a device token for {address}; forget that agent host before pairing it again."
        ));
    }
    let (host, token) = run(remote::pair(address, code, client))?;
    tokens.write(address, &token).map_err(|error| {
        format!(
            "{error} The host keeps this device paired; `meta host revoke {client}` there removes it."
        )
    })?;
    Ok(BridgePairing {
        host,
        client: client.to_owned(),
    })
}

pub(crate) fn forget_with(tokens: &dyn Tokens, address: &str) -> Result<(), String> {
    tokens.delete(address)
}

pub(crate) fn reach_remote_with(tokens: &dyn Tokens, address: &str) -> Result<BridgeReach, String> {
    reach(AgentHost::connect_remote_with(tokens, address).map(drop))
}
