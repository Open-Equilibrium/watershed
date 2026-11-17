//! Agent hosts other than this computer's (ADR-0299): pairing, connecting, reaching and forgetting
//! a device token, against an in-process service on loopback. The keychain stands behind its test
//! seam; needs `META_HARNESS_T3`, as the agent tests do.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use meta_harness_core::{Access, ClientKind, Service, ServiceConfig, remote};

use crate::api::hosts::{
    BridgeReach, client_name, pair_with, reach_local_agent_host, reach_remote_with, tag,
};
use crate::api::session::{AgentHost, RUNTIME};
use crate::keys::{DeviceTokens, Tokens};

/// Device tokens in memory, one per address.
#[derive(Default)]
struct Memory(Mutex<HashMap<String, String>>);

impl Tokens for Memory {
    fn read(&self, address: &str) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap().get(address).cloned())
    }

    fn write(&self, address: &str, token: &str) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .insert(address.to_owned(), token.to_owned());
        Ok(())
    }

    fn delete(&self, address: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(address);
        Ok(())
    }
}

/// A temporary Meta-Harness data folder, removed on drop.
struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).expect("random bytes");
        let name: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        Self(std::env::temp_dir().join(format!("lh-{name}")))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The pinned T3 Code binary the tests run against.
fn t3_binary() -> PathBuf {
    std::env::var_os("META_HARNESS_T3")
        .expect("META_HARNESS_T3 names the pinned t3")
        .into()
}

/// Starts a service on `home` whose remote listener binds loopback.
fn start(home: &Path) -> Service {
    RUNTIME
        .block_on(Service::start_with_remote(
            ServiceConfig {
                home: home.to_owned(),
                t3: t3_binary(),
                acp_agents: Vec::new(),
            },
            Some(vec![Ipv4Addr::LOCALHOST.into()]),
        ))
        .expect("the service starts")
}

fn stop(service: Service) {
    RUNTIME.block_on(service.stop());
}

/// A free loopback port, for the remote listener.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("a loopback port")
        .local_addr()
        .expect("its address")
        .port()
}

#[test]
fn agent_hosts_pair_connect_reach_and_are_forgotten_here_only() {
    let home = Home::new();
    let service = start(home.path());
    let folder = Some(home.path().display().to_string());
    assert_eq!(
        reach_local_agent_host(folder.clone()),
        Ok(BridgeReach::Connected)
    );
    let local = RUNTIME
        .block_on(remote::Host::connect(home.path(), "test"))
        .expect("a local client connects");
    let port = free_port();
    RUNTIME
        .block_on(local.remote_on(port))
        .expect("remote access turns on");
    let url = format!("ws://127.0.0.1:{port}");
    let tokens = Memory::default();

    let client = "liquid-studio-s-mac";
    let refused = pair_with(&tokens, &url, "AAAA-AAAA", client).unwrap_err();
    assert!(!refused.is_empty());
    assert_eq!(
        tokens.read(&url).unwrap(),
        None,
        "a refused pairing keeps nothing"
    );

    let code = RUNTIME
        .block_on(local.invite(ClientKind::Device, Access::Auto))
        .expect("a code");
    let pairing = pair_with(&tokens, &url, &code, client).expect("the device pairs");
    assert!(!pairing.host.is_empty());
    assert_eq!(
        pairing.client, client,
        "the device pairs under its own name"
    );
    let token = tokens
        .read(&url)
        .unwrap()
        .expect("the token is filed under the address");

    let again = RUNTIME
        .block_on(local.invite(ClientKind::Device, Access::Auto))
        .expect("a code");
    assert!(
        pair_with(&tokens, &url, &again, client).is_err(),
        "an address that holds a token is not paired again"
    );
    assert_eq!(tokens.read(&url).unwrap(), Some(token.clone()));

    assert_eq!(reach_remote_with(&tokens, &url), Ok(BridgeReach::Connected));
    let host = AgentHost::connect_remote_with(&tokens, &url).expect("Liquid connects");
    assert!(!host.environment_name().expect("the host's name").is_empty());

    RUNTIME
        .block_on(local.remote_off())
        .expect("remote access turns off");
    assert_eq!(reach_remote_with(&tokens, &url), Ok(BridgeReach::Offline));

    tokens.delete(&url).expect("the token is forgotten");
    assert_eq!(tokens.read(&url).unwrap(), None);
    assert!(
        reach_remote_with(&tokens, &url).is_err(),
        "no token, no reach"
    );
    assert!(AgentHost::connect_remote_with(&tokens, &url).is_err());

    // Forgetting is local: the host still has the device paired and its token works.
    RUNTIME
        .block_on(local.remote_on(port))
        .expect("remote access turns on");
    RUNTIME
        .block_on(remote::Host::connect_to(&url, &token, "test"))
        .expect("the host keeps the device paired");
    // The host lists the device under the name it paired with, and revoking that name removes it.
    let paired = RUNTIME.block_on(local.hosts()).expect("the host's clients");
    assert!(paired.clients.iter().any(|device| device.name == client));
    RUNTIME
        .block_on(local.revoke_client(client))
        .expect("the device is revoked by name");
    assert!(
        RUNTIME
            .block_on(remote::Host::connect_to(&url, &token, "test"))
            .is_err()
    );
    stop(service);
    assert_eq!(reach_local_agent_host(folder), Ok(BridgeReach::Offline));
}

#[test]
fn a_device_pairs_under_liquid_the_computer_name_and_its_own_tag() {
    assert_eq!(
        client_name("Studio's Mac", "3f2a").unwrap(),
        "liquid-studio-s-mac-3f2a"
    );
    assert_eq!(
        client_name("  Lab   Mac mini ", "3f2a").unwrap(),
        "liquid-lab-mac-mini-3f2a"
    );
    // Each pairing draws its own tag, so computers of the same name pair as different devices.
    let drawn = tag().unwrap();
    assert!(
        drawn.len() == 4
            && drawn
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "{drawn}"
    );
    assert!(
        client_name(" !! ", "3f2a").is_err(),
        "a computer with no name cannot pair"
    );
}

#[test]
fn a_long_computer_name_keeps_its_tag_within_the_limit_an_agent_host_keeps() {
    // An agent host keeps a client name of 64 characters at most, so the tag must fit within it.
    assert_eq!(
        client_name(&"a".repeat(100), "3f2a").unwrap(),
        format!("liquid-{}-3f2a", "a".repeat(52))
    );
    // A cut that lands on a separator leaves none before the tag.
    assert_eq!(
        client_name(&format!("{} bee", "a".repeat(51)), "3f2a").unwrap(),
        format!("liquid-{}-3f2a", "a".repeat(51))
    );
}

/// Needs the operating system's keychain, which CI hosts lack; run it by hand with
/// `cargo test -p liquid_bridge -- --ignored keychain`.
#[test]
#[ignore = "needs the operating system's keychain"]
fn the_keychain_keeps_a_device_token_under_its_address() {
    let address = "ws://liquid-bridge-test.invalid:3774";
    let keys = DeviceTokens {};
    keys.delete(address).unwrap();
    assert_eq!(keys.read(address).unwrap(), None);
    keys.write(address, "token").unwrap();
    assert_eq!(keys.read(address).unwrap(), Some("token".to_owned()));
    keys.delete(address).unwrap();
    assert_eq!(keys.read(address).unwrap(), None);
}
