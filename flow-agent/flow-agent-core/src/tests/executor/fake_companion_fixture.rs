use std::{
    env, fs,
    io::{self, Read as _, Write as _},
    path::Path,
    process, thread,
    time::Duration,
};

const PROBE: &str = concat!(
    r#"{"backend":"fake-backend","backend_version":"1","executor":"fake-executor","executor_version":"1","platform":"ubuntu-24.04-x86_64","protocol_versions":["0"],"ready":true,"runtime_mounts":[{"executable":"/bin/echo","runtime_profile":"exact","source":"/usr/bin/echo","target":"/bin/echo"}],"schema":"flow-executor-probe-v0","supported_policy_features":["process-capacity"]}"#,
    "\n"
);

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

fn number_field(request: &str, name: &str) -> u32 {
    let needle = format!("\"{name}\":");
    request
        .split_once(&needle)
        .expect("canonical request contains the field")
        .1
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .expect("numeric request field is a u32")
}

fn completed(request: &str, mode: &str) -> String {
    let request_id = field(request, "request_id");
    let policy_digest = field(request, "policy_digest");
    let policy_digest = if mode == "mismatched-evidence" {
        "0000000000000000000000000000000000000000000000000000000000000000"
    } else {
        policy_digest
    };
    let isolation_active = mode != "inactive-evidence";
    let requested_capacity = number_field(request, "max_concurrent_processes_and_threads");
    let enforced_capacity = if mode == "mismatched-capacity" {
        requested_capacity + 1
    } else {
        requested_capacity
    };
    let backend = if mode == "mismatched-identity" {
        "different-backend"
    } else {
        "fake-backend"
    };
    let enforcement = format!(
        "\"enforcement\":{{\"applied_policy_digest\":\"{policy_digest}\",\"backend\":\"{backend}\",\"backend_version\":\"1\",\"executor\":\"fake-executor\",\"executor_version\":\"1\",\"isolation_active\":{isolation_active},\"max_concurrent_processes_and_threads\":{enforced_capacity},\"platform\":\"ubuntu-24.04-x86_64\",\"runtime_profile\":\"exact\"}},"
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

fn main() {
    let executable = env::current_exe().expect("fake Executor resolves itself");
    let mode = mode(&executable);
    if env::args().nth(1).as_deref() == Some("--probe") {
        match mode {
            "unknown-version" => print!("{}", PROBE.replace("[\"0\"]", "[\"1\"]")),
            "closed-schema" => print!(
                "{}",
                PROBE.replace(
                    ",\"supported_policy_features\"",
                    ",\"unexpected\":true,\"supported_policy_features\""
                )
            ),
            "duplicate-member" => print!(
                "{}",
                PROBE.replace("\"schema\":", "\"schema\":\"duplicate\",\"schema\":")
            ),
            "malformed-probe" => println!("not-json"),
            "oversized-probe" => io::stdout()
                .write_all(&vec![b'x'; 65 * 1024])
                .expect("oversized probe is written"),
            "probe-stderr" => {
                eprintln!("private-fixture-diagnostic");
                process::exit(1);
            }
            _ => print!("{PROBE}"),
        }
        return;
    }

    let mut request = String::new();
    io::stdin()
        .read_to_string(&mut request)
        .expect("request is read");
    if mode != "unsupported-policy" {
        fs::write(executable.with_extension("tool-spawned"), b"spawned")
            .expect("dispatch marker is written");
    }
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
        "unsupported-policy" => println!(
            "{{\"code\":\"executor_policy_unsupported\",\"message\":\"unsupported fake policy\",\"outcome\":\"error\",\"request_id\":\"{}\",\"schema\":\"flow-executor-result-v0\"}}",
            field(&request, "request_id")
        ),
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
