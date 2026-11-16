//! Agent hosts other than this computer's (ADR-0295, ADR-0299). Liquid pairs with one through its
//! address and a pairing code, files the device token the host issues under the address in the
//! keychain, and on removal forgets that token here only. Which hosts are saved, and which one is
//! active, is Liquid's settings file, which never holds a token.

use meta_harness_core::remote;

use super::session::{AgentHost, run};
use crate::keys::{DeviceTokens, Tokens};

/// The name Liquid pairs under; `meta host revoke` names a paired client by it.
const CLIENT: &str = "liquid";

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
/// `meta invite --device`. Files the token it issues under `address` and answers the host's name.
/// Nothing is kept when pairing fails.
pub fn pair_agent_host(address: String, code: String) -> Result<String, String> {
    pair_with(&DeviceTokens {}, &address, &code)
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

pub(crate) fn pair_with(tokens: &dyn Tokens, address: &str, code: &str) -> Result<String, String> {
    if tokens.read(address)?.is_some() {
        return Err(format!(
            "Liquid already holds a device token for {address}; forget that agent host before pairing it again."
        ));
    }
    let (name, token) = run(remote::pair(address, code, CLIENT))?;
    tokens.write(address, &token).map_err(|error| {
        format!(
            "{error} The host keeps this device paired; `meta host revoke {CLIENT}` there removes it."
        )
    })?;
    Ok(name)
}

pub(crate) fn forget_with(tokens: &dyn Tokens, address: &str) -> Result<(), String> {
    tokens.delete(address)
}

pub(crate) fn reach_remote_with(tokens: &dyn Tokens, address: &str) -> Result<BridgeReach, String> {
    reach(AgentHost::connect_remote_with(tokens, address).map(drop))
}
