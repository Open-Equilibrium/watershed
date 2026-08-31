use super::valid_policy_artifact;
use crate::{NetworkAllowEntry, NetworkAllowKind, NetworkTransport, PolicyArtifact};

#[test]
fn policy_artifact_rejects_malformed_network_allow_entries() {
    for cidr in [
        "example.com",
        "192.0.2.42",
        "192.0.2.42/24",
        "192.0.2.0/33",
        "10.0.0.0/01",
        "2001:db8::1/32",
        "2001:DB8::/32",
    ] {
        let artifact = policy_artifact_with_network_allow(cidr, 443);

        let err = artifact
            .validate()
            .expect_err("malformed network allow entry must fail validation");

        assert!(
            err.to_string().contains(cidr),
            "{cidr:?} should be named in {err}"
        );
        assert!(
            err.to_string().contains("canonical CIDR"),
            "{cidr:?} should report the CIDR contract"
        );
    }
}

#[test]
fn policy_artifact_rejects_non_empty_bubblewrap_network_allow_entries() {
    let artifact = policy_artifact_with_network_allow("192.0.2.0/24", 443);

    let err = artifact
        .validate()
        .expect_err("linux artifacts must reject network allowlists");

    assert_eq!(
        err.to_string(),
        "tool network-tool network allow must be empty for linux-bubblewrap-seccomp policy artifacts"
    );
}

#[test]
fn policy_artifact_rejects_zero_network_allow_port() {
    let artifact = policy_artifact_with_network_allow("192.0.2.0/24", 0);

    let err = artifact
        .validate()
        .expect_err("port zero must fail validation");

    assert_eq!(
        err.to_string(),
        "tool network-tool network allow entry 192.0.2.0/24 must use port 1-65535"
    );
}

fn policy_artifact_with_network_allow(cidr: &str, port: u16) -> PolicyArtifact {
    let mut artifact = valid_policy_artifact("network-tool");
    artifact.commands[0].network.allow = vec![NetworkAllowEntry {
        cidr: cidr.to_owned(),
        kind: NetworkAllowKind::Cidr,
        port,
        transport: NetworkTransport::Tcp,
    }];
    artifact
}
