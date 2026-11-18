use std::process::Command;

use meta_harness_core::remote;

use super::session::{AgentHost, run};
use crate::keys::{DeviceTokens, Tokens, random_hex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePairing {
    pub host: String,
    pub client: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeReach {
    Connected,
    Offline,
}

pub fn pair_agent_host(address: String, code: String) -> Result<BridgePairing, String> {
    let client = client_name(&computer_name(), &tag()?)?;
    pair_with(&DeviceTokens {}, &address, &code, &client)
}

const NAME_CHARS: usize = 64;

pub(crate) fn client_name(computer: &str, tag: &str) -> Result<String, String> {
    let words: Vec<String> = computer
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    if words.is_empty() {
        return Err("This computer has no name to pair under.".into());
    }
    let room = NAME_CHARS.saturating_sub("liquid--".len() + tag.len());
    let computer: String = words.join("-").chars().take(room).collect();
    Ok(format!("liquid-{}-{tag}", computer.trim_end_matches('-')))
}

pub(crate) fn tag() -> Result<String, String> {
    Ok(random_hex()?[..4].to_owned())
}

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

pub fn forget_agent_host(address: String) -> Result<(), String> {
    DeviceTokens {}.delete(&address)
}

pub fn reach_local_agent_host(home: Option<String>) -> Result<BridgeReach, String> {
    reach(AgentHost::connect(home).map(drop))
}

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
    let paired = run(remote::pair(address, code, client))?;
    if let Err(error) = device_answers(address, &paired.token, client) {
        return Err(if error == REFUSED_CALL {
            format!(
                "That code pairs a host, not a device: enter a code from meta invite --device instead. The host keeps this device paired as {client}; run meta host revoke {client} there to remove that pairing."
            )
        } else {
            format!(
                "Could not check the pairing: {error} The host keeps this device paired as {client}; meta host revoke {client} there removes it."
            )
        });
    }
    tokens.write(address, &paired.token).map_err(|error| {
        format!(
            "{error} The host keeps this device paired; meta host revoke {client} there removes it."
        )
    })?;
    Ok(BridgePairing {
        host: paired.name,
        client: client.to_owned(),
    })
}

const REFUSED_CALL: &str = "This client may not make that call.";

fn device_answers(address: &str, token: &str, client: &str) -> Result<(), String> {
    let host = run(remote::Host::connect_to(address, token, client))?;
    run(host.status()).map(drop)
}

impl AgentHost {
    pub fn environment_machine(&self) -> Result<String, String> {
        run(self.host.environment_machine())
    }

    pub fn environment_id(&self) -> Result<String, String> {
        run(self.host.environment_id())
    }
}

pub(crate) fn reach_remote_with(tokens: &dyn Tokens, address: &str) -> Result<BridgeReach, String> {
    reach(AgentHost::connect_remote_with(tokens, address).and_then(|host| host.answers_devices()))
}
