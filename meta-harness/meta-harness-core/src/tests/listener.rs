use std::net::IpAddr;

use crate::service::listener::parse_addresses;

#[test]
fn tailscale_prints_one_address_per_line_and_only_tailscale_addresses_are_listened_on() {
    assert_eq!(
        parse_addresses("100.64.0.2\nfd7a:115c:a1e0::2\n\nnot one\n"),
        ["100.64.0.2", "fd7a:115c:a1e0::2"].map(|a| a.parse::<IpAddr>().unwrap())
    );
    assert_eq!(
        parse_addresses("192.168.1.5\n100.128.0.1\nfe80::1\n100.127.255.255\nfd7a:115c:a1e0::9\n"),
        ["100.127.255.255", "fd7a:115c:a1e0::9"].map(|a| a.parse::<IpAddr>().unwrap())
    );
}
