//! Agent hosts other than this computer's (ADR-0295, ADR-0299). Liquid pairs with one through its
//! address and a pairing code, under a name of its own that names this computer, files the device
//! token the host issues under the address in the keychain, and on removal forgets that token here
//! only. Which hosts are saved, and which one is active, is Liquid's settings file, which never
//! holds a token.

use std::process::Command;

use meta_harness_core::remote;

use super::session::{AgentHost, run};
use crate::keys::{DeviceTokens, Tokens, random_hex};

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
    let client = client_name(&computer_name(), &tag()?)?;
    pair_with(&DeviceTokens {}, &address, &code, &client)
}

/// The longest client name an agent host keeps unchanged: 64 characters, the limit its `printable`
/// applies. The name is ASCII, so its UTF-16 units are its characters too.
const NAME_CHARS: usize = 64;

/// The name Liquid pairs under on an agent host: `liquid-`, this computer's name in lower case with
/// each run of other characters one hyphen, cut so the name fits `NAME_CHARS`, and `tag`, so
/// `meta host revoke` names exactly this device even beside a computer of the same name.
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

/// Four random lowercase hex digits that tell this pairing apart from another of the same name.
pub(crate) fn tag() -> Result<String, String> {
    Ok(random_hex()?[..4].to_owned())
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
    DeviceTokens {}.delete(&address)
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
    let paired = run(remote::pair(address, code, client))?;
    // A host's code pairs a host, whose token may make none of a device's calls: it is refused, and
    // the pairing the other host made stays there until its `meta host revoke`.
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

/// The refusal of a call that a client's kind does not make, as the core words it (ADR-0299).
const REFUSED_CALL: &str = "This client may not make that call.";

/// Whether `token` answers a device's calls, checked before Liquid keeps it.
fn device_answers(address: &str, token: &str, client: &str) -> Result<(), String> {
    let host = run(remote::Host::connect_to(address, token, client))?;
    run(host.status()).map(drop)
}

pub(crate) fn reach_remote_with(tokens: &dyn Tokens, address: &str) -> Result<BridgeReach, String> {
    reach(AgentHost::connect_remote_with(tokens, address).and_then(|host| host.answers_devices()))
}
