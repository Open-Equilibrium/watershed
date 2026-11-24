use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_stops_t3_and_the_agent_processes_it_started() {
    let (_dir, repo, home) = fixture();
    let (service, host) = start_service(&home).await;
    let project = host.project(&repo).await.unwrap();
    let thread = project
        .start(start("wait", Workspace::Checkout, Permissions::FullAccess))
        .await
        .unwrap();
    until_running(&thread).await;
    let mut agents = Vec::new();
    for _ in 0..100 {
        agents = processes_naming(&home);
        if !agents.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!agents.is_empty(), "T3 started its agent");
    service.stop().await;
    for _ in 0..50 {
        agents.retain(|pid| running(*pid));
        if agents.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    for pid in &agents {
        kill(*pid);
    }
    assert!(agents.is_empty(), "agent processes outlive shutdown");
}

fn processes_naming(home: &Path) -> Vec<u32> {
    let listing = Command::new("ps")
        .args(["-axww", "-o", "pid=,command="])
        .output()
        .unwrap();
    let marker = home.to_string_lossy();
    String::from_utf8_lossy(&listing.stdout)
        .lines()
        .filter(|line| line.contains(&*marker))
        .filter_map(|line| line.split_whitespace().next()?.parse().ok())
        .collect()
}

fn running(pid: u32) -> bool {
    rustix::process::Pid::from_raw(pid as i32)
        .is_some_and(|pid| rustix::process::test_kill_process(pid).is_ok())
}

fn kill(pid: u32) {
    if let Some(pid) = rustix::process::Pid::from_raw(pid as i32) {
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restart_stops_the_t3_server_a_dead_host_left_behind() {
    let (_dir, _repo, home) = fixture();
    let first = host(&home).await;
    std::mem::forget(first);
    let second = host(&home).await;
    second.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stale_server_record_never_stops_another_process() {
    let (dir, _repo, home) = fixture();
    let other_home = dir.path().join("other");
    let other = host(&other_home).await;
    let record = runtime_record(&other_home);
    let address = record["origin"]
        .as_str()
        .unwrap()
        .trim_start_matches("http://");
    let mut sleeper = Command::new("sleep").arg("60").spawn().unwrap();
    for pid in [sleeper.id().into(), record["pid"].clone()] {
        std::fs::create_dir_all(home.join("userdata")).unwrap();
        std::fs::write(
            home.join("userdata").join("server-runtime.json"),
            serde_json::json!({ "pid": pid }).to_string(),
        )
        .unwrap();
        let host = host(&home).await;
        assert!(
            sleeper.try_wait().unwrap().is_none(),
            "the other program still runs"
        );
        assert!(
            std::net::TcpStream::connect(address).is_ok(),
            "the other folder's server still listens"
        );
        host.shutdown().await;
    }
    other.shutdown().await;
    sleeper.kill().unwrap();
    sleeper.wait().unwrap();
}

fn runtime_record(home: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(home.join("userdata").join("server-runtime.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_host_names_its_machine_kind_and_keeps_one_writer_model() {
    let (_dir, _repo, home) = fixture();
    std::fs::create_dir_all(home.join("t3/userdata")).unwrap();
    std::fs::write(
        home.join("t3/userdata/settings.json"),
        r#"{"environmentIcon":"laptop"}"#,
    )
    .unwrap();
    let (service, host) = start_service(&home).await;
    assert_eq!(host.environment_machine().await.unwrap(), "laptop");
    let writer = ModelChoice {
        instance: "claudeAgent".to_owned(),
        model: "claude-haiku-5-5".to_owned(),
        options: vec![("effort".to_owned(), "high".to_owned())],
    };
    host.set_writer_model(&writer).await.unwrap();
    assert_eq!(host.writer_model().await.unwrap(), Some(writer));
    assert!(
        host.providers()
            .await
            .unwrap()
            .iter()
            .any(|p| p.instance == AGENT)
    );
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_service_admits_only_clients_with_its_token_and_owns_its_folder() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, _repo, _) = fixture();
    let home = dir.path().join("meta");
    let service = Service::start(service_config(&home)).await.unwrap();
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&home), 0o700);
    assert_eq!(mode(&home.join(CONNECTION_FILE)), 0o600);

    assert!(
        remote::Host::connect_to(&service.url, "mh-wrong", "test")
            .await
            .is_err()
    );
    let client = remote::Host::connect(&home, "test").await.unwrap();
    assert_eq!(client.status().await.unwrap().t3_version, t3_version());
    let second = Service::start(service_config(&home)).await;
    assert!(second.is_err_and(|e| e.contains("Another Meta-Harness service")));

    service.stop().await;
    assert!(!home.join(CONNECTION_FILE).exists());
    assert!(
        client.status().await.is_err(),
        "a stopped service answers nothing"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_approved_configuration_reaches_the_harness_homes_t3_runs_them_in() {
    let (dir, _repo, _) = fixture();
    let home = dir.path().join("meta");
    let service = Service::start(service_config(&home)).await.unwrap();
    let client = remote::Host::connect(&home, "liquid").await.unwrap();
    let number = client
        .propose(
            "Test first",
            vec![Change::Instruction {
                name: "rules".to_owned(),
                text: Some("Run the tests.".to_owned()),
            }],
        )
        .await
        .unwrap();
    let claude = home.join("harness/claude");
    let codex = home.join("harness/codex");
    assert!(
        !claude.join("CLAUDE.md").exists(),
        "no file before approval"
    );
    assert_eq!(client.approve(number).await.unwrap().revision, 1);
    assert_eq!(
        std::fs::read_to_string(claude.join("CLAUDE.md")).unwrap(),
        "Run the tests.\n"
    );
    assert_eq!(
        std::fs::read_to_string(codex.join("AGENTS.md")).unwrap(),
        "Run the tests.\n"
    );
    let log = client.log().await.unwrap();
    assert!(log.iter().all(|entry| entry.client == "liquid"), "{log:?}");

    let settings: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.join("t3/userdata/settings.json")).unwrap(),
    )
    .unwrap();
    let instances = &settings["providerInstances"];
    assert_eq!(
        instances["codex"]["config"]["homePath"],
        codex.to_string_lossy().as_ref()
    );
    assert_eq!(
        instances["claudeAgent"]["config"]["homePath"],
        claude.to_string_lossy().as_ref()
    );
    assert_eq!(
        instances[AGENT]["driver"], "acpRegistry",
        "the ACP agent stays"
    );
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_or_dropped_service_ends_its_streams_and_a_client_goes_on_after_a_restart() {
    let (dir, repo, _) = fixture();
    let home = dir.path().join("meta");
    let (service, client) = start_service(&home).await;
    let mut statuses = client
        .project(&repo)
        .await
        .unwrap()
        .git()
        .watch()
        .await
        .unwrap();
    let before = client.status().await.unwrap();

    service.stop().await;
    assert!(client.status().await.is_err(), "nothing answers meanwhile");
    let ended = tokio::time::timeout(Duration::from_secs(10), async {
        while statuses.recv().await.is_some() {}
    })
    .await;
    assert!(ended.is_ok(), "a stream of the lost connection ends");

    let service = Service::start(service_config(&home)).await.unwrap();
    let after = client.status().await.unwrap();
    assert_eq!(after.revision, before.revision);
    assert!(client.project(&repo).await.is_ok());

    let _statuses = client
        .project(&repo)
        .await
        .unwrap()
        .git()
        .watch()
        .await
        .unwrap();
    let (url, token) = (service.url.clone(), service.token.clone());
    drop(service);
    assert!(!home.join(CONNECTION_FILE).exists());
    tokio::time::timeout(Duration::from_secs(10), async {
        while client.status().await.is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        while remote::Host::connect_to(&url, &token, "contract")
            .await
            .is_ok()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("a dropped service ends its connections and stops listening");
    settle(
        &home,
        "the folder is still held by the dropped service's stream",
        |home| async move { Service::start(service_config(home)).await.ok() },
    )
    .await
    .stop()
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_service_refuses_a_client_that_names_no_api_version_or_another() {
    use tokio_tungstenite::tungstenite::{Error, client::IntoClientRequest, http::StatusCode};
    let (dir, _repo, _) = fixture();
    let home = dir.path().join("meta");
    let service = Service::start(service_config(&home)).await.unwrap();
    for version in [None, Some("2")] {
        let mut request = service.url.as_str().into_client_request().unwrap();
        request.headers_mut().insert(
            "authorization",
            format!("Bearer {}", service.token).parse().unwrap(),
        );
        if let Some(version) = version {
            request
                .headers_mut()
                .insert("meta-harness-api", version.parse().unwrap());
        }
        let refused = match tokio_tungstenite::connect_async(request).await {
            Err(Error::Http(response)) => response,
            Ok(_) => panic!("the service admits a client of version {version:?}"),
            Err(other) => panic!("{other}"),
        };
        assert_eq!(refused.status(), StatusCode::UPGRADE_REQUIRED);
        assert_eq!(refused.headers()["meta-harness-api"], "1");
    }
    service.stop().await;
}

async fn open_client(
    service: &Service,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = service.url.as_str().into_client_request().unwrap();
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", service.token).parse().unwrap(),
    );
    request
        .headers_mut()
        .insert("meta-harness-api", "1".parse().unwrap());
    tokio_tungstenite::connect_async(request).await.unwrap().0
}

async fn disconnected_after_flood(
    service: &Service,
    text: &str,
    limit: Duration,
) -> Option<Duration> {
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::Message;
    let mut socket = open_client(service).await;
    let started = std::time::Instant::now();
    tokio::time::timeout(limit, async {
        loop {
            if socket
                .send(Message::Text(text.to_owned().into()))
                .await
                .is_err()
            {
                return;
            }
        }
    })
    .await
    .ok()
    .map(|()| started.elapsed())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_stops_reading_is_disconnected_and_one_that_reads_gets_every_reply() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let (dir, _repo, _home) = fixture();
    let (service, _local) = start_service(&dir.path().join("meta")).await;
    for text in [
        r#"{"id":1,"bogus":true}"#,
        r#"{"id":1,"call":{"host":"status"}}"#,
    ] {
        assert!(
            disconnected_after_flood(&service, text, Duration::from_secs(8))
                .await
                .is_some(),
            "a client that never reads its replies is disconnected: {text}"
        );
    }
    let mut socket = open_client(&service).await;
    for id in [1u64, 2] {
        let bogus = serde_json::json!({"id": id, "bogus": true});
        socket
            .send(Message::Text(bogus.to_string().into()))
            .await
            .unwrap();
    }
    for id in [1u64, 2] {
        let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
            panic!("a reply is text");
        };
        let reply: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(reply["id"], id);
        assert!(reply.get("error").is_some(), "{reply}");
    }
    for id in 3..=302u64 {
        let call = serde_json::json!({"id": id, "call": {"host": "status"}});
        socket
            .send(Message::Text(call.to_string().into()))
            .await
            .unwrap();
        let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
            panic!("a reply is text");
        };
        let reply: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(reply["id"], id);
        assert!(reply.get("ok").is_some(), "{reply}");
    }
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_path_that_is_not_utf8_is_refused_before_it_is_sent() {
    use std::os::unix::ffi::OsStrExt;
    let (_dir, repo, home) = fixture();
    let (service, host) = start_service(&home).await;
    let not_utf8 = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff"));
    let refused = host.project(not_utf8).await.err().unwrap();
    assert!(refused.contains("not valid UTF-8"), "{refused}");
    let project = host.project(&repo).await.unwrap();
    let thread = project
        .start(start("wait", Workspace::Checkout, Permissions::FullAccess))
        .await
        .unwrap();
    let refused = thread
        .set_branch("main", Some(not_utf8))
        .await
        .err()
        .unwrap();
    assert!(refused.contains("not valid UTF-8"), "{refused}");
    service.stop().await;
}
