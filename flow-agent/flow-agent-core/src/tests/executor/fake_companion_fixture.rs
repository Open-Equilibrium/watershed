use std::{
    env, fs,
    io::{self, BufRead as _, BufReader, Write as _},
    path::Path,
    process::{self, Command},
    thread,
    time::Duration,
};

unsafe extern "C" {
    fn setsid() -> i32;
}

fn platform() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "ubuntu-24.04-x86_64"
    }
    #[cfg(target_os = "macos")]
    {
        "macos-26-aarch64"
    }
}

fn backend() -> String {
    format!("fake-{}-{}", env::consts::OS, env::consts::ARCH)
}

fn probe() -> String {
    format!(
        "{{\"backend\":\"{}\",\"backend_version\":\"1\",\"executor\":\"fake-executor\",\"executor_version\":\"1\",\"platform\":\"{}\",\"protocol_versions\":[\"0\"],\"ready\":true,\"schema\":\"flow-executor-probe-v0\",\"supported_policy_features\":[\"flow-owned-write-protection\"]}}\n",
        backend(),
        platform(),
    )
}

fn mode(executable: &Path) -> &str {
    executable
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("fake-executor-"))
        .expect("fake Executor filename carries its mode")
}

fn field<'a>(request: &'a str, name: &str) -> &'a str {
    let needle = format!("\"{name}\":\"");
    let value = request
        .split_once(&needle)
        .expect("canonical request contains the field")
        .1;
    value.split_once('"').expect("field is closed").0
}

fn completed(request: &str, mode: &str) -> String {
    let request_id = field(request, "request_id");
    let policy_digest = field(request, "policy_digest");
    let policy_digest = if mode == "mismatched-evidence" {
        "0000000000000000000000000000000000000000000000000000000000000000"
    } else {
        policy_digest
    };
    let self_protection_active = mode != "inactive-evidence";
    let legacy_evidence = if mode == "legacy-evidence" {
        "\"isolation_active\":true,"
    } else {
        ""
    };
    let backend = if mode == "mismatched-identity" {
        "different-backend".to_owned()
    } else {
        backend()
    };
    let enforcement = format!(
        "\"enforcement\":{{\"applied_policy_digest\":\"{policy_digest}\",\"backend\":\"{backend}\",\"backend_version\":\"1\",\"executor\":\"fake-executor\",\"executor_version\":\"1\",{legacy_evidence}\"platform\":\"{}\",\"self_protection_active\":{self_protection_active}}},",
        platform(),
    );
    let enforcement = if mode == "missing-evidence" {
        ""
    } else {
        &enforcement
    };
    format!(
        "{{{enforcement}\"outcome\":\"completed\",\"request_id\":\"{request_id}\",\"schema\":\"flow-executor-result-v0\",\"tool_result\":{{\"classification\":null,\"exit_code\":0,\"status\":\"completed\",\"stderr_base64\":\"\",\"stdout_base64\":\"Cg==\"}}}}\n"
    )
}

fn hold_escaped_probe_output(executable: &Path) {
    // The child must not remain in the controller-killed Executor process group.
    assert_ne!(
        unsafe { setsid() },
        -1,
        "fixture child starts its own session"
    );
    fs::write(
        executable.with_extension("escaped-probe.pid"),
        process::id().to_string(),
    )
    .expect("escaped probe child PID is recorded");
    let release = executable.with_extension("release-escaped-probe-output");
    while !release.exists() {
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn spawn_escaped_probe_output(executable: &Path) {
    let mut child = Command::new(executable)
        .arg("--hold-probe-output")
        .spawn()
        .expect("escaped probe child starts");
    let pid_path = executable.with_extension("escaped-probe.pid");
    for _ in 0..100 {
        if pid_path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = child.kill();
    panic!("escaped probe child did not record its PID");
}

fn main() {
    let executable = env::current_exe().expect("fake Executor resolves itself");
    let mode = mode(&executable);
    if env::args().nth(1).as_deref() == Some("--hold-probe-output") {
        hold_escaped_probe_output(&executable);
        return;
    }
    if env::args().nth(1).as_deref() == Some("--probe") {
        let probe = probe();
        match mode {
            "unknown-version" => print!("{}", probe.replace("[\"0\"]", "[\"1\"]")),
            "closed-schema" => print!(
                "{}",
                probe.replace(
                    ",\"supported_policy_features\"",
                    ",\"unexpected\":true,\"supported_policy_features\""
                )
            ),
            "duplicate-member" => print!(
                "{}",
                probe.replace("\"schema\":", "\"schema\":\"duplicate\",\"schema\":")
            ),
            "malformed-probe" => println!("not-json"),
            "oversized-probe" => io::stdout()
                .write_all(&vec![b'x'; 65 * 1024])
                .expect("oversized probe is written"),
            "probe-stderr" => {
                eprintln!("private-fixture-diagnostic");
                process::exit(1);
            }
            "probe-escaped-output" => {
                spawn_escaped_probe_output(&executable);
                print!("{probe}");
            }
            _ => print!("{probe}"),
        }
        return;
    }

    let mut input = BufReader::new(io::stdin());
    let mut request = String::new();
    input.read_line(&mut request).expect("request is read");
    let request_id = field(&request, "request_id");
    if mode == "preflight-timeout" {
        thread::sleep(Duration::from_secs(10));
        return;
    }
    if matches!(mode, "unsupported-policy" | "preflight-trailing-output") {
        println!(
            "{{\"code\":\"executor_policy_unsupported\",\"message\":\"unsupported fake policy\",\"outcome\":\"error\",\"request_id\":\"{request_id}\",\"schema\":\"flow-executor-preflight-v0\"}}"
        );
        if mode == "preflight-trailing-output" {
            io::stdout().flush().expect("preflight error is flushed");
            thread::sleep(Duration::from_millis(100));
            println!("{{}}");
        }
        return;
    }
    println!(
        "{{\"outcome\":\"ready\",\"request_id\":\"{request_id}\",\"schema\":\"flow-executor-preflight-v0\"}}"
    );
    io::stdout().flush().expect("preflight is flushed");
    let mut start = String::new();
    if input.read_line(&mut start).expect("start is read") == 0 {
        return;
    }
    let expected_start =
        format!("{{\"request_id\":\"{request_id}\",\"schema\":\"flow-executor-start-v0\"}}\n");
    if start != expected_start {
        return;
    }
    if matches!(mode, "valid" | "productive-session") {
        fs::write(executable.with_extension("request.json"), &request)
            .expect("synthetic request is captured");
    }
    fs::write(executable.with_extension("tool-spawned"), b"spawned")
        .expect("dispatch marker is written");
    let valid = completed(&request, mode);
    match mode {
        "malformed-output" => println!("not-json"),
        "multiple-output" => print!("{valid}{valid}"),
        "mismatched-request-id" => print!(
            "{}",
            valid.replace("fake-companion-request", "different-request")
        ),
        "premature-exit" => {}
        "timeout" => thread::sleep(Duration::from_secs(10)),
        "oversized-output" => io::stdout()
            .write_all(&vec![b'x'; 12 * 1024 * 1024])
            .expect("oversized result is written"),
        "stderr-output" => {
            let mut diagnostic = b"private-fixture-diagnostic".to_vec();
            diagnostic.resize(5 * 1024, b'x');
            io::stderr()
                .write_all(&diagnostic)
                .expect("bounded stderr fixture is written");
            process::exit(1);
        }
        _ => print!("{valid}"),
    }
}
