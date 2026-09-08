mod native_support;

use native_support::{Fixture, limits};
use proto::{
    ExecutorResponseV0, ExecutorToolClassificationV0 as Classification,
    ExecutorToolStatusV0 as Status, decode_executor_stream_v0,
};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

#[test]
fn protection_blocks_own_objects_and_descendants_but_allows_project_work() {
    let fixture = Fixture::new();
    fs::write(fixture.project.join("AGENTS.md"), "local instructions").unwrap();
    let script = r#"
set -eu
home=$1
printf updated > AGENTS.md
printf scratch > scratch
test "$(cat "$home/AGENTS.md")" = 'global instructions'
if (printf bad > "$home/AGENTS.md") 2>/dev/null; then exit 10; fi
if rm "$home/AGENTS.md" 2>/dev/null; then exit 11; fi
if mv scratch "$home/AGENTS.md" 2>/dev/null; then exit 12; fi
if mkdir "$home/created" 2>/dev/null; then exit 13; fi
if mv "$home" "$home-moved" 2>/dev/null; then exit 14; fi
ln -s "$home/AGENTS.md" symlink
if (printf bad > symlink) 2>/dev/null; then exit 15; fi
if ln "$home/AGENTS.md" hardlink 2>/dev/null; then exit 16; fi
ln scratch project-hardlink
test "$(cat project-hardlink)" = scratch
if sh -c 'printf bad > "$1/AGENTS.md"' child "$home" 2>/dev/null; then exit 17; fi
if (printf bad > "$home/../flow-executor") 2>/dev/null; then exit 18; fi
if mv "$home/.." "$home/../../relocated" 2>/dev/null; then exit 19; fi
if (printf bad > /dev/tty) 2>/dev/null; then exit 20; fi
printf protected
"#;
    let result = fixture.run(script, limits(1024, 4096, 5000));
    assert_eq!(result.status, Status::Completed, "{result:?}");
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(
        decode_executor_stream_v0(&result.stdout_base64).unwrap(),
        b"protected"
    );
    assert_eq!(
        fs::read(fixture.home.join("AGENTS.md")).unwrap(),
        b"global instructions"
    );
    assert_eq!(
        fs::read(fixture.project.join("AGENTS.md")).unwrap(),
        b"updated"
    );
    assert!(fixture.image.is_file());
}

#[test]
fn ready_does_not_run_a_tool_and_replaced_objects_fail_before_start() {
    let fixture = Fixture::new();
    let request = fixture.request("printf executed > marker", limits(1024, 1024, 2000));
    let mut running = fixture.spawn(&request);
    running.ready(&request);
    assert!(!fixture.project.join("marker").exists());
    fs::rename(&fixture.home, fixture.root.join("retained-home")).unwrap();
    fs::create_dir(&fixture.home).unwrap();
    running.start(&request);
    assert!(matches!(
        running.finish(&request),
        ExecutorResponseV0::Error { .. }
    ));
    assert!(!fixture.project.join("marker").exists());
}

#[test]
fn a_mismatched_protected_descriptor_is_rejected_before_ready() {
    let fixture = Fixture::new();
    let mut request = fixture.request("printf executed > marker", limits(1024, 1024, 2000));
    request.resolved_policy.protected_objects[0].identity.inode += 1;
    request.policy_digest = proto::resolved_policy_digest_v0(&request.resolved_policy).unwrap();
    let running = fixture.spawn(&request);
    assert!(matches!(
        proto::parse_executor_preflight_v0(
            format!("{}\n", running.record()).as_bytes(),
            &request.request_id
        )
        .unwrap(),
        proto::ExecutorPreflightV0::Error { .. }
    ));
    assert!(!fixture.project.join("marker").exists());
}

#[test]
fn native_tools_keep_host_runtimes_network_and_only_the_explicit_environment() {
    let fixture = Fixture::new();
    fs::write(
        fixture.project.join("exercise.py"),
        r#"
import os, socket
assert set(os.environ) <= {"HOME", "PATH", "PWD", "SHLVL", "_", "LC_CTYPE"}, os.environ.keys()
assert os.getcwd() == os.environ["HOME"]
with socket.socket() as server:
    server.settimeout(2)
    server.bind(("127.0.0.1", 0))
    server.listen(1)
    with socket.create_connection(server.getsockname(), timeout=2) as client:
        client.sendall(b"host-network")
        peer, _ = server.accept()
        with peer:
            assert peer.recv(64) == b"host-network"
with socket.socket(socket.AF_UNIX) as server:
    server.bind("local.sock")
    server.listen(1)
    with socket.socket(socket.AF_UNIX) as client:
        client.connect("local.sock")
        client.sendall(b"local-service")
        peer, _ = server.accept()
        with peer:
            assert peer.recv(64) == b"local-service"
open("python-result", "w").write("host runtime works")
"#,
    )
    .unwrap();
    let result = fixture.run("exec python3 exercise.py", limits(1024, 4096, 5000));
    assert_eq!(result.status, Status::Completed, "{result:?}");
    assert_eq!(
        fs::read(fixture.project.join("python-result")).unwrap(),
        b"host runtime works"
    );
}

#[test]
fn npm_build_runs_its_lifecycle_and_generated_child_under_the_same_guard() {
    native_support::builds::npm_lifecycle("javascript");
}

#[test]
fn terminal_evidence_preserves_status_and_bounded_output() {
    let fixture = Fixture::new();
    for (script, class, code, output) in [
        ("printf ok", None, Some(0), b"ok".as_slice()),
        (
            "printf partial; exit 7",
            Some(Classification::NonzeroExit),
            Some(7),
            b"partial",
        ),
        (
            "exit 143",
            Some(Classification::NonzeroExit),
            Some(143),
            b"",
        ),
        (
            "kill -TERM $$",
            Some(Classification::SignalTermination),
            None,
            b"",
        ),
    ] {
        let result = fixture.run(script, limits(1024, 1024, 3000));
        assert_eq!(result.classification, class, "{result:?}");
        assert_eq!(result.exit_code, code);
        assert_eq!(
            decode_executor_stream_v0(&result.stdout_base64).unwrap(),
            output
        );
    }
    for (script, class, stdout, stderr) in [
        (
            "printf 12345",
            Classification::StdoutCapExceeded,
            b"1234".as_slice(),
            b"".as_slice(),
        ),
        (
            "printf 12345 >&2",
            Classification::StderrCapExceeded,
            b"",
            b"1234",
        ),
    ] {
        let result = fixture.run(script, limits(4, 4, 3000));
        assert_eq!(result.status, Status::Failed, "{result:?}");
        assert_eq!(result.classification, Some(class));
        assert_eq!(
            decode_executor_stream_v0(&result.stdout_base64).unwrap(),
            stdout
        );
        assert_eq!(
            decode_executor_stream_v0(&result.stderr_base64).unwrap(),
            stderr
        );
    }
}

#[test]
fn timeout_and_cancellation_reap_the_tool_root_after_term_grace() {
    for cancel in [false, true] {
        let fixture = Fixture::new();
        let request = fixture.request(
            "trap 'printf stopped > term-marker; exit 0' TERM; printf ready > root-ready; while :; do sleep 0.02; done",
            limits(1024, 1024, if cancel { 5000 } else { 1500 }));
        let mut running = fixture.spawn(&request);
        running.ready(&request);
        running.start(&request);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !fixture.project.join("root-ready").exists() {
            assert!(Instant::now() < deadline, "Tool did not start");
            thread::sleep(Duration::from_millis(5));
        }
        if cancel {
            let pid = rustix::process::Pid::from_raw(running.child.id() as i32).unwrap();
            rustix::process::kill_process(pid, rustix::process::Signal::TERM).unwrap();
        }
        let ExecutorResponseV0::Completed {
            enforcement,
            tool_result,
            ..
        } = running.finish(&request)
        else {
            panic!("cooperative Tool must have definitive terminal evidence");
        };
        proto::validate_enforcement_receipt_v0(&enforcement, &request.policy_digest).unwrap();
        assert_eq!(
            tool_result.status,
            if cancel {
                Status::Cancelled
            } else {
                Status::TimedOut
            }
        );
        assert_eq!(tool_result.exit_code, None);
        assert_eq!(
            fs::read(fixture.project.join("term-marker")).unwrap(),
            b"stopped"
        );
    }
}
