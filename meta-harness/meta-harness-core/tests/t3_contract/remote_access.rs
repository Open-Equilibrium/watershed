use super::*;

async fn connects(url: &str, token: &str, name: &str) -> bool {
    remote::Host::connect_to(url, token, name).await.is_ok()
}

async fn may_tunnel(url: &str, token: &str) -> bool {
    let headers = [("meta-harness-api", "1"), ("meta-harness-tunnel", "t3")];
    tokio_tungstenite::connect_async(upgrade(url, token, &headers))
        .await
        .is_ok()
}

#[tokio::test(flavor = "multi_thread")]
async fn pairing_names_host_addresses_and_the_handshake_limit_are_checked_before_use() {
    let (dir, _repo, _home) = fixture();
    let (_service, local) = start_service(&dir.path().join("meta")).await;
    let port = remote_on_free(&local).await;
    let url = format!("ws://127.0.0.1:{port}");
    let code = local
        .invite(ClientKind::Device, Access::Auto)
        .await
        .unwrap();
    remote::pair(&url, &code, "phone\tlaptop").await.unwrap();
    let clients = local.hosts().await.unwrap().clients;
    assert_eq!(
        clients[0].name, "phonelaptop",
        "meta host list prints the name"
    );
    local.host_agents(true).await.unwrap();
    let error = local
        .host_add("127.0.0.1:3774", "AAAA-AAAA", None)
        .await
        .unwrap_err();
    assert_eq!(
        error,
        "Enter the address as ws://HOST:PORT, as meta invite prints it."
    );
    assert_handshake_cap(&format!("127.0.0.1:{port}")).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_access_refuses_port_zero_and_binds_a_busy_port_once_it_is_free() {
    let (dir, _repo, _home) = fixture();
    let meta = dir.path().join("meta");
    let (service, local) = start_service(&meta).await;
    assert!(
        local.remote_on(0).await.is_err(),
        "port 0 is no port to listen on"
    );
    assert!(local.remote_status().await.unwrap().is_none());
    let port = remote_on_free(&local).await;
    drop(local);
    service.stop().await;
    exec_sql(
        &meta,
        "INSERT INTO peer (name, address, token, access, tunnel_port, linked_ms, client) \
         VALUES ('bad', 'ws://127.0.0.1:1', 'token', 'unknown', 1, NULL, 'client')",
    );
    let failed = Service::start_with_remote(
        service_config(&meta),
        Some(vec![std::net::Ipv4Addr::LOCALHOST.into()]),
    )
    .await;
    assert!(
        failed.is_err(),
        "a linked host with an unknown access level"
    );
    assert!(
        std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
        "nothing listens after the failed start"
    );
    exec_sql(&meta, "DELETE FROM peer WHERE name = 'bad'");
    let holder = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    let (service, local) = start_service(&meta).await;
    let waiting = local.remote_status().await.unwrap().expect("still on");
    assert!(waiting.addresses.is_empty());
    assert!(waiting.waiting.is_some(), "on, not listening yet");
    drop(holder);
    settle(&local, "bound again", |local| async move {
        let status = local.remote_status().await.unwrap().expect("still on");
        status.waiting.is_none().then_some(())
    })
    .await;
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_ok());
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_pairs_once_keeps_its_access_across_a_restart_and_ends_with_remote_access() {
    let (_dir, _repo, home) = fixture();
    let (service, local) = start_service(&home).await;
    let port = free_port();
    let status = local.remote_on(port).await.unwrap();
    let url = format!("ws://127.0.0.1:{port}");
    assert_eq!(status.addresses, [url.as_str()]);

    let code = local
        .invite(ClientKind::Device, Access::Auto)
        .await
        .unwrap();
    let token = remote::pair(&url, &code, "phone").await.unwrap().token;
    let used = remote::pair(&url, &code, "tablet").await.unwrap_err();
    assert_eq!(used, "That pairing code is wrong, already used or expired.");
    let wrong = remote::pair(&url, "AAAA-AAAA", "tablet").await.unwrap_err();
    assert_eq!(wrong, used);
    let second = local
        .invite(ClientKind::Device, Access::Auto)
        .await
        .unwrap();
    let refused = remote::pair(&url, &second, "phone").await.unwrap_err();
    assert_eq!(
        refused,
        "A client named phone is paired already; pair under another name."
    );
    let tablet_token = remote::pair(&url, &second, "tablet")
        .await
        .expect("the code was not spent")
        .token;

    let device = remote::Host::connect_to(&url, &token, "impostor")
        .await
        .unwrap();
    device.status().await.unwrap();
    device.config().await.unwrap();
    propose_rules(&device).await;
    let log = local.log().await.unwrap();
    assert!(
        !log.is_empty() && log.iter().all(|entry| entry.client == "phone"),
        "{log:?}"
    );
    assert!(device.remote_status().await.is_err());
    assert!(
        device
            .invite(ClientKind::Device, Access::Auto)
            .await
            .is_err()
    );
    assert!(device.hosts().await.is_err());
    assert!(device.host_agents(true).await.is_err());
    assert!(!may_tunnel(&url, &token).await, "a device is no tunnel");

    drop(local);
    service.stop().await;
    let (_service, local) = start_service(&home).await;
    let status = local.remote_status().await.unwrap().expect("still on");
    assert_eq!(status.port, port);
    assert!(connects(&url, &token, "phone").await);
    local.revoke_client("phone").await.unwrap();
    assert!(!connects(&url, &token, "phone").await);

    let host_code = invite(&local, ClientKind::Host, Access::Auto).await;
    let paired = remote::pair(&url, &host_code, "other").await.unwrap();
    let (name, host_token) = (paired.name, paired.token);
    assert_eq!(name, local.environment_name().await.unwrap().to_lowercase());
    let host = remote::Host::connect_to(&url, &host_token, "other")
        .await
        .unwrap();
    assert!(
        host.status().await.is_err(),
        "a host asks for a T3 pairing code only"
    );
    assert!(!host.t3_pairing().await.unwrap().is_empty());
    assert!(!connects(&url, &"0".repeat(64), "other").await);

    let tablet = remote::Host::connect_to(&url, &tablet_token, "tablet")
        .await
        .unwrap();
    tablet.status().await.unwrap();
    local.remote_off().await.unwrap();
    assert!(local.remote_status().await.unwrap().is_none());
    settle(
        &tablet,
        "the device's connection ended with remote access",
        |tablet| async move { tablet.status().await.is_err().then_some(()) },
    )
    .await;
}
