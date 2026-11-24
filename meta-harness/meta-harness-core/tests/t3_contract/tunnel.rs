use super::*;

const FORM: &[(&str, &str)] = &[("Content-Type", "application/x-www-form-urlencoded")];
const JSON: &[(&str, &str)] = &[("Content-Type", "application/json")];
const REDIRECT: &str = "http://127.0.0.1/t3code-peer-link";
const PKCE_CHALLENGE: &str = "_-BU_nrgy23GXDr5th1SCfQ5hR20PQulmXM33xVGaOs";

fn http(addr: &str, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> String {
    http_at(addr, addr, method, path, headers, body)
}

fn http_at(
    connect: &str,
    host: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> String {
    let mut stream = std::net::TcpStream::connect(connect).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
    std::io::Write::write_all(&mut stream, request.as_bytes()).unwrap();
    let mut answer = String::new();
    let _ = std::io::Read::read_to_string(&mut stream, &mut answer);
    answer
}

fn post(addr: &str, path: &str, headers: &[(&str, &str)], body: &str) -> serde_json::Value {
    let answer = http(addr, "POST", path, headers, body);
    assert!((200..300).contains(&status_of(&answer)), "{path}: {answer}");
    body_json(&answer)
}

fn status_of(answer: &str) -> u16 {
    answer
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0)
}

fn body_json(answer: &str) -> serde_json::Value {
    let body = &answer[answer.find('{').unwrap()..=answer.rfind('}').unwrap()];
    serde_json::from_str(body).unwrap()
}

fn t3_address(home: &Path) -> String {
    let runtime = json_file(&home.join("t3/userdata/server-runtime.json"));
    runtime["origin"]
        .as_str()
        .unwrap()
        .trim_start_matches("http://")
        .to_owned()
}

fn exchange(origin: &str, credential: &str, scope: Option<&str>) -> String {
    let mut form = format!(
        "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Atoken-exchange\
         &subject_token={credential}\
         &subject_token_type=urn%3At3%3Aparams%3Aoauth%3Atoken-type%3Aenvironment-bootstrap\
         &requested_token_type=urn%3Aietf%3Aparams%3Aoauth%3Atoken-type%3Aaccess_token"
    );
    if let Some(scope) = scope {
        form.push_str(&format!("&scope={scope}"));
    }
    http(origin, "POST", "/oauth/token", FORM, &form)
}

fn form_value(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn query(url: &str, key: &str) -> String {
    let query = url.split_once('?').unwrap().1;
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(&format!("{key}=")))
        .unwrap()
        .to_owned()
}

fn register_client(addr: &str) -> String {
    let registered = post(
        addr,
        "/oauth/mcp/register",
        JSON,
        &serde_json::json!({"client_name": "tester", "redirect_uris": [REDIRECT]}).to_string(),
    );
    registered["client_id"].as_str().unwrap().to_owned()
}

fn decide(addr: &str, client_id: &str, pairing: &str) -> String {
    http(
        addr,
        "POST",
        "/oauth/mcp/decision",
        JSON,
        &serde_json::json!({
            "authorization": {
                "response_type": "code",
                "client_id": client_id,
                "redirect_uri": REDIRECT,
                "code_challenge": PKCE_CHALLENGE,
                "code_challenge_method": "S256",
                "state": "state",
                "resource": format!("http://{addr}/mcp"),
            },
            "decision": {"_tag": "pairing-code", "access": "auto", "code": pairing},
        })
        .to_string(),
    )
}

fn approve(addr: &str, client_id: &str, pairing: &str) -> String {
    let answer = decide(addr, client_id, pairing);
    assert_eq!(status_of(&answer), 200, "{answer}");
    query(body_json(&answer)["redirectTo"].as_str().unwrap(), "code")
}

fn code_form(code: &str, client_id: &str, addr: &str) -> String {
    format!(
        "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}&resource={}",
        form_value(code),
        form_value(REDIRECT),
        form_value(client_id),
        "a".repeat(64),
        form_value(&format!("http://{addr}/mcp")),
    )
}

fn sign_in(addr: &str, pairing: &str) -> String {
    let client_id = register_client(addr);
    let code = approve(addr, &client_id, pairing);
    let token = post(
        addr,
        "/oauth/mcp/token",
        FORM,
        &code_form(&code, &client_id, addr),
    );
    format!("Bearer {}", token["access_token"].as_str().unwrap())
}

fn mcp_answer(connect: &str, host: &str, bearer: &str) -> String {
    let initialize = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "tester", "version": "0"}},
    });
    http_at(
        connect,
        host,
        "POST",
        "/mcp",
        &[
            ("Authorization", bearer),
            ("Accept", "application/json, text/event-stream"),
            ("Content-Type", "application/json"),
        ],
        &initialize.to_string(),
    )
}

fn t3_sessions(home: &Path) -> Vec<serde_json::Value> {
    t3_json(home, &["auth", "session", "list", "--json"])
}

#[tokio::test(flavor = "multi_thread")]
async fn the_tunnel_passes_only_the_paths_a_link_uses() {
    let (dir, _repo, _home) = fixture();
    let (_link, name) = linked(dir.path()).await;
    let port = peer_row(&dir.path().join("a"), &name).0;
    let addr = format!("127.0.0.1:{port}");
    for (method, path) in [
        ("GET", "/api/auth/session"),
        ("POST", "/api/auth/browser-session"),
        ("POST", "/oauth/token"),
        ("POST", "/api/auth/websocket-ticket"),
        ("GET", "/api/projects"),
        ("GET", "/api/orchestration/shell"),
        ("GET", "/ws"),
        ("POST", "/.well-known/t3/environment"),
        ("GET", "/oauth/mcp/token"),
        ("PUT", "/mcp"),
        ("GET", "/"),
    ] {
        let answer = http(&addr, method, path, &[], "");
        assert_eq!(status_of(&answer), 403, "{method} {path}");
    }
    let descriptor = http(&addr, "GET", "/.well-known/t3/environment", &[], "");
    assert_eq!(status_of(&descriptor), 200, "the link's descriptor");
    let mcp = http(&addr, "GET", "/mcp", &[], "");
    assert_ne!(status_of(&mcp), 403, "the link's MCP path");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pairing_code_from_a_host_token_redeems_only_orchestration_scopes() {
    let (dir, _repo, _home) = fixture();
    let home_b = dir.path().join("b");
    let (_b, b) = start_service(&home_b).await;
    let port_b = remote_on_free(&b).await;
    let url = format!("ws://127.0.0.1:{port_b}");
    let origin = t3_address(&home_b);
    for (access, name, scope) in [
        (
            Access::Auto,
            "other",
            "orchestration:read orchestration:operate",
        ),
        (Access::ReadOnly, "reader", "orchestration:read"),
    ] {
        let code = invite(&b, ClientKind::Host, access).await;
        let paired = remote::pair(&url, &code, name).await.unwrap();
        assert_eq!(paired.access, access);
        let host = remote::Host::connect_to(&url, &paired.token, name)
            .await
            .unwrap();
        for requested in ["terminal:operate", "filesystem:write", "settings:write"] {
            let credential = host.t3_pairing().await.unwrap();
            let answer = exchange(&origin, &credential, Some(requested));
            assert_ne!(status_of(&answer), 200, "{requested} is refused: {answer}");
        }
        let answer = exchange(&origin, &host.t3_pairing().await.unwrap(), None);
        assert_eq!(status_of(&answer), 200, "{answer}");
        assert_eq!(body_json(&answer)["scope"], scope);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_client_closes_the_connections_it_has_open() {
    let (dir, _repo, _home) = fixture();
    let (link, name) = linked(dir.path()).await;
    let (_b, b) = &link.b;
    let device = connect_paired(b, &link.url_b, ClientKind::Device, "phone").await;
    device.status().await.unwrap();
    let port = peer_row(&dir.path().join("a"), &name).0;
    let mut tunnel = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let request =
        format!("GET /.well-known/t3/environment HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
    tunnel.write_all(request.as_bytes()).await.unwrap();
    let mut answer = [0u8; 12];
    tunnel.read_exact(&mut answer).await.unwrap();
    assert_eq!(&answer, b"HTTP/1.1 200");

    b.revoke_client("phone").await.unwrap();
    let host = b
        .hosts()
        .await
        .unwrap()
        .clients
        .into_iter()
        .find(|client| client.kind == ClientKind::Host)
        .unwrap();
    b.revoke_client(&host.name).await.unwrap();
    assert!(
        device.status().await.is_err(),
        "the device's connection ended"
    );
    let ended = tokio::time::timeout(Duration::from_secs(20), async {
        let mut rest = Vec::new();
        tunnel.read_to_end(&mut rest).await
    })
    .await;
    assert!(ended.is_ok(), "the tunnel's connection ended");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_linked_agent_reaches_t3_another_pairing_is_refused_and_revoking_its_host_ends_its_sign_ins()
 {
    let (dir, _repo, _home) = fixture();
    let (link, name_a) = linked(dir.path()).await;
    let a_clients = host_client_names(&link.b.1).await;
    assert_eq!(a_clients.len(), 1, "{a_clients:?}");
    let a_client = a_clients[0].clone();
    let home_c = dir.path().join("c");
    let (_c, c) = start_service(&home_c).await;
    c.host_agents(true).await.unwrap();
    let code = link
        .b
        .1
        .invite(ClientKind::Host, Access::Auto)
        .await
        .unwrap();
    let (name_c, _) = c.host_add(&link.url_b, &code, None).await.unwrap();
    let addr_a = tunnel_addr(&dir.path().join("a"), &name_a);
    let addr_c = tunnel_addr(&home_c, &name_c);
    let tester = connect_paired(&link.b.1, &link.url_b, ClientKind::Host, "tester").await;
    let bearer = sign_in(&addr_a, &tester.t3_pairing().await.unwrap());
    let answer = mcp_answer(&addr_a, &addr_a, &bearer);
    assert_eq!(status_of(&answer), 200, "{answer}");
    assert!(answer.contains("protocolVersion"), "{answer}");

    let client_id = register_client(&addr_a);
    let refused = decide(&addr_c, &client_id, &tester.t3_pairing().await.unwrap());
    assert_eq!(status_of(&refused), 403, "a decision: {refused}");
    let code = approve(&addr_a, &client_id, &tester.t3_pairing().await.unwrap());
    let answer = http_at(
        &addr_c,
        &addr_a,
        "POST",
        "/oauth/mcp/token",
        FORM,
        &code_form(&code, &client_id, &addr_a),
    );
    assert_eq!(status_of(&answer), 403, "a token exchange: {answer}");
    assert_eq!(status_of(&mcp_answer(&addr_c, &addr_a, &bearer)), 200);

    link.b.1.revoke_client(&a_client).await.unwrap();
    assert!(
        t3_sessions(&dir.path().join("b"))
            .iter()
            .all(|session| session["client"]["label"] != a_client.as_str()),
        "A's T3 sessions end with its pairing"
    );
    let refused = mcp_answer(&addr_c, &addr_a, &bearer);
    assert_eq!(
        status_of(&refused),
        401,
        "T3 refuses the revoked session's token: {refused:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_name_with_no_client_still_ends_the_sign_ins_made_under_it() {
    let (dir, _repo, _home) = fixture();
    let (link, name) = linked(dir.path()).await;
    let (_b, b) = &link.b;
    let home_b = dir.path().join("b");
    let addr_a = tunnel_addr(&dir.path().join("a"), &name);
    let clients = host_client_names(b).await;
    let [client] = clients.as_slice() else {
        panic!("one host pairs with b: {clients:?}")
    };
    let tester = connect_paired(b, &link.url_b, ClientKind::Host, "tester").await;
    let _ = sign_in(&addr_a, &tester.t3_pairing().await.unwrap());
    assert!(
        t3_sessions(&home_b)
            .iter()
            .any(|s| s["client"]["label"] == client.as_str())
    );
    db(&home_b)
        .execute("DELETE FROM client WHERE name = ?1", [client])
        .unwrap();
    b.revoke_client(client).await.unwrap();
    assert!(
        t3_sessions(&home_b)
            .iter()
            .all(|s| s["client"]["label"] != client.as_str()),
        "the name's sessions end"
    );
}
