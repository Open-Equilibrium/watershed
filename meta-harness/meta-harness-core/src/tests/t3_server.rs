use std::path::Path;
use std::time::Duration;

use crate::t3::server::{Server, issue, listening_origin};
use crate::t3::t3_version;

#[cfg(unix)]
#[tokio::test]
async fn a_server_that_ends_during_its_start_is_started_again() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("t3-start-{}", std::process::id()));
    let home = dir.join("home");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(home.join("userdata")).unwrap();
    let binary = dir.join("t3");
    let script = format!(
        "#!/bin/sh\n\
         [ \"$1\" = serve ] || exit 1\n\
         echo start >> \"$T3CODE_HOME/../starts\"\n\
         if [ ! -e \"$T3CODE_HOME/../started\" ]; then : > \"$T3CODE_HOME/../started\"; exit 1; fi\n\
         printf '{{\"pid\":%s,\"serverVersion\":\"{version}\",\"origin\":\"http://127.0.0.1:1\"}}' $$ \
         > \"$T3CODE_HOME/userdata/server-runtime.json\"\n\
         exec sleep 30\n",
        version = t3_version(),
    );
    std::fs::write(&binary, script).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();

    let started = std::time::Instant::now();
    let mut server = Server::start(&binary, &home)
        .await
        .expect("the second start serves");
    assert_eq!(server.origin, "http://127.0.0.1:1");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    let starts = std::fs::read_to_string(dir.join("starts")).unwrap();
    assert_eq!(starts.lines().count(), 2, "started again once");
    server.stop().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[tokio::test]
async fn a_token_issue_that_never_ends_is_killed_at_its_wait() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("t3-issue-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let binary = dir.join("t3");
    std::fs::write(&binary, "#!/bin/sh\nexec sleep 60\n").unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();

    let started = std::time::Instant::now();
    let result = issue(binary, dir.clone(), Duration::from_millis(200)).await;
    assert!(result.is_err(), "{result:?}");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[tokio::test]
async fn stopping_t3_ends_the_processes_it_started_too() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("t3-tree-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let home = dir.join("home");
    std::fs::create_dir_all(home.join("userdata")).unwrap();
    let binary = dir.join("t3");
    let script = format!(
        "#!/bin/sh\n\
         sleep 60 &\n\
         echo $! > \"$T3CODE_HOME/../started\"\n\
         printf '{{\"pid\":%s,\"serverVersion\":\"{version}\",\"origin\":\"http://127.0.0.1:1\"}}' $$ \
         > \"$T3CODE_HOME/userdata/server-runtime.json\"\n\
         wait\n",
        version = t3_version(),
    );
    std::fs::write(&binary, script).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut server = Server::start(&binary, &home).await.expect("T3 starts");
    let started = std::fs::read_to_string(dir.join("started")).unwrap();
    let agent = rustix::process::Pid::from_raw(started.trim().parse().unwrap()).unwrap();
    server.stop().await;
    assert!(
        crate::util::until(Duration::from_secs(5), Duration::from_millis(50), || {
            rustix::process::test_kill_process(agent).is_err()
        })
        .await,
        "a process T3 started outlives its stop"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_stale_pid_counts_as_t3_only_when_it_serves_the_folder_with_the_binary() {
    let (binary, home) = (Path::new("/x/t3"), Path::new("/data/a"));
    for (command, runs) in [
        (
            "/x/t3 serve --host 127.0.0.1 --no-browser --base-dir /data/a\n",
            true,
        ),
        (
            "/x/t3 serve --host 127.0.0.1 --no-browser --base-dir /data/b\n",
            false,
        ),
        (
            "/x/t3 serve --host 127.0.0.1 --no-browser --base-dir /data/a2\n",
            false,
        ),
        (
            "/x/t3backup serve --host 127.0.0.1 --base-dir /data/a\n",
            false,
        ),
        ("/x/t3-helper serve --other\n", false),
        ("/x/t3 auth session issue\n", false),
    ] {
        let base = home.to_string_lossy();
        let arguments = [
            "serve",
            "--host",
            "127.0.0.1",
            "--no-browser",
            "--base-dir",
            &base,
        ];
        assert_eq!(
            crate::platform::matches_command(command, binary, &arguments),
            runs,
            "{command}"
        );
    }
}

#[test]
fn a_server_of_another_release_fails_at_once_naming_both() {
    let text = r#"{"pid":7,"serverVersion":"0.0.0","origin":"http://127.0.0.1:1"}"#;
    assert_eq!(
        listening_origin(text, Some(7)),
        Err(format!(
            "The t3 executable runs T3 0.0.0; Meta-Harness needs {}",
            t3_version()
        ))
    );
}
