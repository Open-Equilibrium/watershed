use super::*;

pub(crate) const AGENT: &str = "scripted";

pub(crate) fn t3() -> PathBuf {
    PathBuf::from(
        std::env::var_os("META_HARNESS_T3")
            .expect("META_HARNESS_T3 names the pinned t3 executable"),
    )
}

pub(crate) fn transfer() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    format!(
        "{:016x}{:016x}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

pub(crate) fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

pub(crate) fn seed_repo(repo: &Path) {
    std::fs::create_dir(repo).unwrap();
    git(repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "contract\n").unwrap();
    git(repo, &["config", "user.name", "Contract"]);
    git(repo, &["config", "user.email", "contract@example.invalid"]);
    git(repo, &["add", "."]);
    git(repo, &["commit", "-qm", "start"]);
}

pub(crate) fn fixture() -> (tempdir::Dir, PathBuf, PathBuf) {
    let dir = tempdir::Dir::new();
    let repo = dir.path().join("repo");
    seed_repo(&repo);
    let home = dir.path().join("t3");
    (dir, repo, home)
}

pub(crate) async fn host(home: &Path) -> Host {
    Host::start(HostConfig {
        binary: t3(),
        home: home.to_owned(),
        acp_agents: agents(home),
    })
    .await
    .expect("T3 starts")
}

pub(crate) async fn start_service(home: &Path) -> (Service, remote::Host) {
    start_with(service_config(home), home).await
}

pub(crate) async fn start_with(config: ServiceConfig, home: &Path) -> (Service, remote::Host) {
    let service =
        Service::start_with_remote(config, Some(vec![std::net::Ipv4Addr::LOCALHOST.into()]))
            .await
            .expect("the service starts");
    let host = remote::Host::connect(home, "contract")
        .await
        .expect("a client connects");
    (service, host)
}

pub(crate) fn agents(home: &Path) -> Vec<AcpAgent> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scripted_acp_agent.py");
    vec![AcpAgent {
        id: AGENT.to_owned(),
        command: PathBuf::from("/usr/bin/python3"),
        args: vec![
            script.to_string_lossy().into_owned(),
            home.to_string_lossy().into_owned(),
        ],
        models: vec![AGENT.to_owned()],
    }]
}

pub(crate) fn service_config(home: &Path) -> ServiceConfig {
    ServiceConfig {
        home: home.to_owned(),
        t3: t3(),
        acp_agents: agents(home),
    }
}

pub(crate) fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

pub(crate) async fn remote_on_free(host: &remote::Host) -> u16 {
    for _ in 0..5 {
        let port = free_port();
        match host.remote_on(port).await {
            Ok(_) => return port,
            Err(error) if error.contains("already in use") => continue,
            Err(error) => panic!("{error}"),
        }
    }
    panic!("no free port for remote access");
}

pub(crate) fn worktree() -> Workspace {
    Workspace::NewWorktree {
        base: "main".to_owned(),
    }
}

pub(crate) fn start(text: &str, workspace: Workspace, permissions: Permissions) -> StartThread {
    StartThread {
        message_id: None,
        text: text.to_owned(),
        model: ModelChoice {
            instance: AGENT.to_owned(),
            model: AGENT.to_owned(),
            options: Vec::new(),
        },
        workspace,
        permissions,
    }
}

pub(crate) async fn wait(thread: &remote::Thread, done: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let waited = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let (snapshot, mut events) = thread.subscribe().await.unwrap();
            if done(&snapshot) {
                return snapshot;
            }
            events.recv().await;
        }
    })
    .await;
    match waited {
        Ok(snapshot) => snapshot,
        Err(_) => panic!("not there within a minute: {:#?}", thread.snapshot().await),
    }
}

pub(crate) fn completed(n: usize) -> impl Fn(&Snapshot) -> bool {
    move |s| {
        s.turns
            .iter()
            .filter(|t| t.status == TurnStatus::Completed && t.checkpoint)
            .count()
            >= n
    }
}

pub(crate) fn says(transcript: &[ChatItem], text: &str) -> bool {
    transcript.iter().any(|item| item.text == text)
}

pub(crate) fn every_hour(title: &str, prompt: &str) -> NewRoutine {
    NewRoutine {
        title: title.to_owned(),
        prompt: prompt.to_owned(),
        schedule: Schedule {
            kind: ScheduleKind::Interval,
            every_ms: Some(3_600_000),
            time_of_day: None,
            weekdays: Vec::new(),
            run_at: None,
        },
    }
}

pub(crate) async fn until_running(thread: &remote::Thread) {
    wait(thread, |s| {
        s.turns.iter().any(|t| t.status == TurnStatus::Running)
    })
    .await;
}

pub(crate) async fn hold_turn(thread: &remote::Thread) {
    thread.send("wait", false).await.unwrap();
    until_running(thread).await;
}

pub(crate) async fn send_queued(thread: &remote::Thread, text: &str) {
    thread.send(text, false).await.unwrap();
    wait(thread, |s| {
        s.turns
            .iter()
            .any(|t| t.status == TurnStatus::Queued && t.prompt == text)
    })
    .await;
}

pub(crate) async fn ran(thread: &remote::Thread, text: &str) {
    wait(thread, |s| {
        s.turns.iter().any(|t| {
            t.prompt == text && matches!(t.status, TurnStatus::Running | TurnStatus::Completed)
        })
    })
    .await;
}

pub(crate) async fn move_out(
    thread: &remote::Thread,
    transfer: &str,
    note: Option<&str>,
) -> Result<MovePackage, String> {
    thread
        .move_out(
            transfer,
            "host-b",
            "studio",
            "environment-target",
            true,
            note.map(str::to_owned),
        )
        .await
}

pub(crate) async fn handoff(thread: &remote::Thread) -> MovePackage {
    move_out(thread, &transfer(), Some("Append")).await.unwrap()
}

pub(crate) async fn arrival(host: &remote::Host, transfer: &str) -> Arrival {
    host.move_status(transfer).await.unwrap()
}

pub(crate) async fn withdraw(host: &remote::Host, transfer: &str) -> Arrival {
    host.move_abort(transfer).await.unwrap()
}

pub(crate) async fn release(thread: &remote::Thread, transfer: &str) {
    thread
        .move_finish(transfer, MoveFinish::Release)
        .await
        .unwrap();
}

pub(crate) async fn appended(project: &remote::Project) -> remote::Thread {
    let thread = project.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    thread
}

pub(crate) fn pushed_branch(repo: &Path, name: &str) {
    git(repo, &["checkout", "-q", "-b", name]);
    git(repo, &["push", "-q", "-u", "origin", name]);
}

pub(crate) fn local_from_origin(repo: &Path, name: &str) {
    git(repo, &["fetch", "-q", "origin"]);
    git(repo, &["branch", name, &format!("origin/{name}")]);
}

pub(crate) async fn settle<'a, A, T, F, Fut>(arg: &'a A, what: &str, mut step: F) -> T
where
    F: FnMut(&'a A) -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if let Some(found) = step(arg).await {
                return found;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("not {what} within a minute"))
}

pub(crate) mod tempdir {
    pub struct Dir(std::path::PathBuf);

    impl Dir {
        pub fn new() -> Self {
            let path = std::env::temp_dir().join(format!("mh-{}", super::transfer()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }

        pub fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

pub(crate) fn shared_remote(dir: &Path) -> (PathBuf, String) {
    let origin = dir.join("origin.git");
    git(dir, &["init", "-q", "--bare", "-b", "main", "origin.git"]);
    let seed = dir.join("seed");
    seed_repo(&seed);
    git(&seed, &["push", "-q", &origin.to_string_lossy(), "main"]);
    let transport = dir.join("fake-ssh");
    std::fs::write(
        &transport,
        "#!/bin/sh\nfor last; do :; done\nexec sh -c \"$last\"\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&transport, std::fs::Permissions::from_mode(0o755)).unwrap();
    let url = format!("git@example.invalid:{}", origin.display());
    (origin, url)
}

pub(crate) fn clone_project(dir: &Path, origin: &Path, url: &str, name: &str) -> PathBuf {
    let repo = dir.join(name);
    git(dir, &["clone", "-q", &origin.to_string_lossy(), name]);
    git(&repo, &["remote", "set-url", "origin", url]);
    let ssh = dir.join("fake-ssh").to_string_lossy().into_owned();
    git(&repo, &["config", "core.sshCommand", &ssh]);
    git(&repo, &["config", "user.name", "Contract"]);
    git(&repo, &["config", "user.email", "contract@example.invalid"]);
    repo
}

pub(crate) fn pushed_subject(origin: &Path, branch: &str) -> String {
    let log = Command::new("git")
        .args(["log", "-1", "--format=%s", branch])
        .current_dir(origin)
        .output()
        .unwrap();
    String::from_utf8_lossy(&log.stdout).trim().to_owned()
}

pub(crate) async fn start_codex(home: &Path) -> (Service, remote::Host) {
    let mut config = service_config(home);
    for agent in &mut config.acp_agents {
        agent.id = "codex".to_owned();
    }
    start_with(config, home).await
}

pub(crate) fn codex_start(text: &str) -> StartThread {
    let mut thread = start(text, Workspace::Checkout, Permissions::FullAccess);
    thread.model.instance = "codex".to_owned();
    thread
}

pub(crate) async fn codex_host(dir: &tempdir::Dir) -> (PathBuf, Service, remote::Host) {
    let (origin, url) = shared_remote(dir.path());
    let repo = clone_project(dir.path(), &origin, &url, "a");
    let (service, host) = start_codex(&dir.path().join("host-a")).await;
    (repo, service, host)
}

pub(crate) struct Pair {
    pub(crate) dir: tempdir::Dir,
    pub(crate) origin: PathBuf,
    pub(crate) url: String,
    pub(crate) repo_a: PathBuf,
    pub(crate) repo_b: PathBuf,
    pub(crate) service_a: Service,
    pub(crate) host_a: remote::Host,
    pub(crate) service_b: Service,
    pub(crate) host_b: remote::Host,
}

impl Pair {
    pub(crate) async fn codex() -> Self {
        let dir = tempdir::Dir::new();
        let (origin, url) = shared_remote(dir.path());
        let repo_a = clone_project(dir.path(), &origin, &url, "a");
        let repo_b = clone_project(dir.path(), &origin, &url, "b");
        let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
        let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
        Self {
            dir,
            origin,
            url,
            repo_a,
            repo_b,
            service_a,
            host_a,
            service_b,
            host_b,
        }
    }

    pub(crate) async fn projects(&self) -> (remote::Project, remote::Project) {
        (
            self.host_a.project(&self.repo_a).await.unwrap(),
            self.host_b.project(&self.repo_b).await.unwrap(),
        )
    }

    pub(crate) async fn stop(self) {
        self.service_a.stop().await;
        self.service_b.stop().await;
    }
}

pub(crate) struct Hosts {
    pub(crate) a: (Service, remote::Host),
    pub(crate) b: (Service, remote::Host),
    pub(crate) url_b: String,
}

pub(crate) async fn two_hosts(
    dir: &Path,
) -> ((Service, remote::Host), (Service, remote::Host), String) {
    let (a_service, a) = start_service(&dir.join("a")).await;
    let (b_service, b) = start_service(&dir.join("b")).await;
    remote_on_free(&a).await;
    let port_b = remote_on_free(&b).await;
    a.host_agents(true).await.unwrap();
    b.host_agents(true).await.unwrap();
    (
        (a_service, a),
        (b_service, b),
        format!("ws://127.0.0.1:{port_b}"),
    )
}

pub(crate) fn db(home: &Path) -> rusqlite::Connection {
    rusqlite::Connection::open(home.join("meta-harness.sqlite")).unwrap()
}

pub(crate) fn exec_sql(home: &Path, sql: &str) {
    db(home).execute_batch(sql).unwrap();
}

pub(crate) fn refuse(home: &Path, name: &str, on: &str) {
    exec_sql(
        home,
        &format!("CREATE TRIGGER {name} {on} BEGIN SELECT RAISE(ABORT, 'refused'); END;"),
    );
}

pub(crate) fn json_file(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

pub(crate) fn assert_refused<T>(result: Result<T, String>, text: &str) {
    let error = result.err().expect("the call is refused");
    assert!(error.contains(text), "{error}");
}

pub(crate) fn upgrade(
    url: &str,
    token: &str,
    headers: &[(&'static str, &str)],
) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = url.into_client_request().unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    for (name, value) in headers {
        request.headers_mut().insert(*name, value.parse().unwrap());
    }
    request
}

pub(crate) async fn assert_handshake_cap(address: &str) {
    let idle =
        futures_util::future::join_all((0..32).map(|_| tokio::net::TcpStream::connect(address)))
            .await;
    let mut extra = tokio::net::TcpStream::connect(address).await.unwrap();
    let read = tokio::time::timeout(Duration::from_secs(5), extra.read(&mut [0u8; 1])).await;
    assert_eq!(
        read.expect("closed without waiting for a handshake")
            .unwrap(),
        0,
        "the extra connection is closed"
    );
    drop(idle);
}

pub(crate) fn peer_row(home: &Path, name: &str) -> (u16, Option<i64>) {
    db(home)
        .query_row(
            "SELECT tunnel_port, linked_ms FROM peer WHERE name = ?1",
            [name],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

pub(crate) fn tunnel_addr(home: &Path, name: &str) -> String {
    format!("127.0.0.1:{}", peer_row(home, name).0)
}

pub(crate) async fn connect_paired(
    on: &remote::Host,
    url: &str,
    kind: ClientKind,
    name: &str,
) -> remote::Host {
    let code = invite(on, kind, Access::Auto).await;
    let token = remote::pair(url, &code, name).await.unwrap().token;
    remote::Host::connect_to(url, &token, name).await.unwrap()
}

pub(crate) async fn invite(host: &remote::Host, kind: ClientKind, access: Access) -> String {
    host.invite(kind, access).await.unwrap()
}

pub(crate) async fn propose_rules(host: &remote::Host) -> u64 {
    host.propose(
        "Test first",
        vec![Change::Instruction {
            name: "rules".to_owned(),
            text: Some("Run the tests.".to_owned()),
        }],
    )
    .await
    .unwrap()
}

pub(crate) async fn linked(dir: &Path) -> (Hosts, String) {
    let (a, b, url_b) = two_hosts(dir).await;
    let link = Hosts { a, b, url_b };
    let code = invite(&link.b.1, ClientKind::Host, Access::Auto).await;
    let (name, _) = link
        .a
        .1
        .host_add(&link.url_b, &code, Some(Access::Auto))
        .await
        .unwrap();
    (link, name)
}

pub(crate) async fn host_client_names(on: &remote::Host) -> Vec<String> {
    on.hosts()
        .await
        .unwrap()
        .clients
        .into_iter()
        .filter(|client| client.kind == ClientKind::Host)
        .map(|client| client.name)
        .collect()
}

pub(crate) fn t3_environments(home: &Path) -> Vec<serde_json::Value> {
    t3_json(home, &["environment", "list", "--json"])
}

pub(crate) fn t3_json(home: &Path, args: &[&str]) -> Vec<serde_json::Value> {
    let output = Command::new(t3())
        .args(args)
        .env("T3CODE_HOME", home.join("t3"))
        .env("T3CODE_TELEMETRY_ENABLED", "false")
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap()
}
