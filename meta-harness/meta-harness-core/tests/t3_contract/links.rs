use super::*;

fn linked_environments_setting(home: &Path) -> bool {
    json_file(&home.join("t3/userdata/settings.json"))["linkedEnvironmentsEnabled"]
        .as_bool()
        .unwrap_or(false)
}

#[allow(clippy::result_large_err)]
async fn silent_peer() -> String {
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::{
        Message,
        handshake::server::{Request, Response},
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            let mut pairing = false;
            let Ok(mut socket) = tokio_tungstenite::accept_hdr_async(
                socket,
                |request: &Request, response: Response| {
                    pairing = request
                        .headers()
                        .contains_key(meta_harness_core::api::PAIRING_HEADER);
                    Ok(response)
                },
            )
            .await
            else {
                continue;
            };
            if pairing {
                let answer = r#"{"name":"silent","token":"silent-token","access":"auto"}"#;
                let _ = socket.send(Message::Text(answer.into())).await;
                let _ = socket.close(None).await;
            } else {
                held.push(socket);
            }
        }
    });
    url
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_links_another_hosts_t3_through_a_tunnel_behind_the_switch_and_removing_it_unlinks_it()
 {
    let (dir, _repo, _home) = fixture();
    let (home_a, home_b) = (dir.path().join("a"), dir.path().join("b"));
    let (_a, a) = start_service(&home_a).await;
    let (_b, b) = start_service(&home_b).await;
    let (port_a, port_b) = (free_port(), free_port());
    a.remote_on(port_a).await.unwrap();
    b.remote_on(port_b).await.unwrap();
    let address = format!("ws://127.0.0.1:{port_b}");
    let waiting = invite(&b, ClientKind::Host, Access::Auto).await;
    assert_refused(
        a.host_add(&address, &waiting, Some(Access::Auto)).await,
        "meta host agents on",
    );
    a.host_agents(true).await.unwrap();
    b.host_agents(true).await.unwrap();
    assert!(linked_environments_setting(&home_a));

    let code = invite(&b, ClientKind::Host, Access::Auto).await;
    let (name, _) = a
        .host_add(&address, &code, Some(Access::Auto))
        .await
        .unwrap();
    assert_eq!(name, b.environment_name().await.unwrap().to_lowercase());
    let listed = a.hosts().await.unwrap();
    assert_eq!(listed.hosts.len(), 1);
    assert!(listed.hosts[0].reachable);
    assert_eq!(listed.hosts[0].access, Access::Auto);
    let linked = t3_environments(&home_a);
    let environment = std::fs::read_to_string(home_b.join("t3/userdata/environment-id")).unwrap();
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0]["environmentId"], environment.trim());
    assert_eq!(linked[0]["status"], "reachable");

    let clients = b.hosts().await.unwrap().clients.len();
    let code = invite(&b, ClientKind::Host, Access::Auto).await;
    assert_refused(a.host_add(&address, &code, Some(Access::Auto)).await, &name);
    assert_eq!(
        b.hosts().await.unwrap().clients.len(),
        clients,
        "no stray pairing"
    );
    assert_eq!(a.hosts().await.unwrap().hosts.len(), 1);

    let (tunnel, _) = peer_row(&home_a, &name);
    let mut open = tokio::net::TcpStream::connect(("127.0.0.1", tunnel))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    refuse(&home_a, "refuse_peer_delete", "BEFORE DELETE ON peer");
    assert!(a.host_remove(&name).await.is_err());
    assert_eq!(a.hosts().await.unwrap().hosts.len(), 1);
    assert!(std::net::TcpStream::connect(("127.0.0.1", tunnel)).is_ok());
    assert_eq!(
        peer_row(&home_a, &name).1,
        None,
        "a failed removal links again"
    );
    exec_sql(&home_a, "DROP TRIGGER refuse_peer_delete");

    let client = a.host_remove(&name).await.unwrap();
    assert!(host_client_names(&b).await.contains(&client), "{client}");
    assert!(t3_environments(&home_a).is_empty(), "T3 forgets the link");
    assert!(a.hosts().await.unwrap().hosts.is_empty());
    assert!(std::net::TcpStream::connect(("127.0.0.1", tunnel)).is_err());
    let ended = tokio::time::timeout(Duration::from_secs(10), async {
        let mut byte = [0u8; 1];
        open.read(&mut byte).await
    })
    .await
    .expect("the connection ends with its tunnel");
    assert_eq!(ended.unwrap(), 0, "the tunnel's connection is closed");
    a.host_agents(false).await.unwrap();
    assert!(!linked_environments_setting(&home_a));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_without_an_access_level_takes_the_level_its_host_invited() {
    let (dir, _repo, _home) = fixture();
    let ((_a, a), (_b, b), url_b) = two_hosts(dir.path()).await;
    let code = b
        .invite(ClientKind::Host, Access::FullAccess)
        .await
        .unwrap();
    let (_, access) = a.host_add(&url_b, &code, None).await.unwrap();
    assert_eq!(access, Access::FullAccess);
    let listed = a.hosts().await.unwrap();
    assert_eq!(listed.hosts[0].access, Access::FullAccess);
    assert!(listed.hosts[0].reachable, "a full-access link answers");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_takes_or_refuses_an_access_level_by_its_invite_and_names_the_pairing_it_made() {
    let (dir, _repo, _home) = fixture();
    let ((_a, a), (_b, b), url_b) = two_hosts(dir.path()).await;
    let code = b
        .invite(ClientKind::Host, Access::ApprovalRequired)
        .await
        .unwrap();
    let error = a
        .host_add(&url_b, &code, Some(Access::Auto))
        .await
        .unwrap_err();
    let me = a.environment_name().await.unwrap().to_lowercase();
    let pairing = b
        .hosts()
        .await
        .unwrap()
        .clients
        .into_iter()
        .find(|client| client.kind == ClientKind::Host)
        .expect("b paired with a")
        .name;
    let (host, tag) = pairing.rsplit_once('-').expect("the host name, then a tag");
    assert!(!host.is_empty() && me.trim().starts_with(host), "{pairing}");
    assert!(
        tag.len() == 4 && tag.chars().all(|c| c.is_ascii_hexdigit()),
        "{pairing}"
    );
    assert!(
        error.contains(&format!("meta host revoke {pairing}")),
        "{error}"
    );

    let code = invite(&b, ClientKind::Host, Access::ReadOnly).await;
    assert_refused(
        a.host_add(&url_b, &code, Some(Access::Auto)).await,
        "invited this host at read-only; ask for that or less.",
    );
    assert!(
        a.hosts().await.unwrap().hosts.is_empty(),
        "nothing is linked"
    );

    let code = b
        .invite(ClientKind::Host, Access::ApprovalRequired)
        .await
        .unwrap();
    a.host_add(&url_b, &code, Some(Access::ReadOnly))
        .await
        .unwrap();
    let listed = a.hosts().await.unwrap();
    assert_eq!(listed.hosts[0].access, Access::ReadOnly);
    assert!(listed.hosts[0].reachable);
    let clients = b.hosts().await.unwrap().clients;
    let linked_pairing = clients
        .iter()
        .find(|client| client.name == listed.hosts[0].client)
        .expect("the pairing the link made");
    assert_eq!(
        linked_pairing.access,
        Access::ApprovalRequired,
        "the level the host was invited at"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_link_is_made_again_on_restart_when_its_port_is_taken_or_it_is_older_than_25_days() {
    let (dir, _repo, _home) = fixture();
    let home_a = dir.path().join("a");
    let ((a_service, a), (_b, b), url_b) = two_hosts(dir.path()).await;
    let code = invite(&b, ClientKind::Host, Access::Auto).await;
    let (name, _) = a
        .host_add(&url_b, &code, Some(Access::ReadOnly))
        .await
        .unwrap();
    let old = peer_row(&home_a, &name).0;
    drop(a);
    a_service.stop().await;

    let aged = peer_row(&home_a, &name).1.unwrap() - 26 * 24 * 3_600_000;
    db(&home_a)
        .execute("UPDATE peer SET linked_ms = ?1", [aged])
        .unwrap();
    let holder = std::net::TcpListener::bind(("127.0.0.1", old)).unwrap();
    let (_a_service, a) = start_service(&home_a).await;
    let state = (&a, &home_a);
    settle(
        &state,
        "linked again through the new port",
        move |state| async move {
            let (a, home) = *state;
            let linked = a.hosts().await.unwrap().hosts[0].linked_ms.unwrap_or(0);
            let reachable = t3_environments(home)
                .iter()
                .any(|link| link["status"] == "reachable");
            (linked > aged as u64 && reachable).then_some(())
        },
    )
    .await;
    assert_ne!(peer_row(&home_a, &name).0, old, "the tunnel moved");
    assert!(a.hosts().await.unwrap().hosts[0].reachable);
    drop(holder);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_made_is_reported_made_even_when_its_time_cannot_be_stored() {
    let (dir, _repo, _home) = fixture();
    let home_a = dir.path().join("a");
    let ((_a, a), (_b, b), url_b) = two_hosts(dir.path()).await;
    refuse(&home_a, "refuse_link_time", "BEFORE UPDATE ON peer");
    let code = invite(&b, ClientKind::Host, Access::Auto).await;
    let (name, _) = a
        .host_add(&url_b, &code, None)
        .await
        .expect("the link is made, so the add reports it");
    assert!(a.hosts().await.unwrap().hosts[0].reachable, "{name}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peer_that_never_answers_a_link_is_given_up_on_and_holds_no_other_change() {
    let (dir, _repo, _home) = fixture();
    let ((_a, a), (_b, b), url_b) = two_hosts(dir.path()).await;
    let silent = silent_peer().await;
    let code = invite(&b, ClientKind::Host, Access::Auto).await;
    let given_up = tokio::time::timeout(Duration::from_secs(60), a.host_add(&silent, &code, None))
        .await
        .expect("the link gives up on a peer that never answers");
    assert!(given_up.is_err());
    let code = invite(&b, ClientKind::Host, Access::Auto).await;
    let linked = tokio::time::timeout(Duration::from_secs(60), a.host_add(&url_b, &code, None))
        .await
        .expect("a link is not held behind the silent one");
    assert!(linked.is_ok(), "{linked:?}");
}
