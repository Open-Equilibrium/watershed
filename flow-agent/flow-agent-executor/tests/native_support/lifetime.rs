use super::{Fixture, NEXT, assert_success, limits};
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::DirBuilderExt,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};

struct Channel {
    root: PathBuf,
    listener: UnixListener,
}

impl Channel {
    fn new() -> Self {
        // Darwin's ordinary temporary directory can exceed sockaddr_un's limit.
        let root = PathBuf::from(format!(
            "/tmp/flow-child-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let listener = UnixListener::bind(root.join("channel")).unwrap();
        listener.set_nonblocking(true).unwrap();
        Self { root, listener }
    }

    fn accept(&self) -> UnixStream {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    // Darwin inherits the listener's nonblocking flag; the
                    // connected protocol uses blocking I/O with bounded waits.
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(10)))
                        .unwrap();
                    stream
                        .set_write_timeout(Some(Duration::from_secs(10)))
                        .unwrap();
                    return stream;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "detached child did not connect");
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("detached child channel failed: {error}"),
            }
        }
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.root.join("channel"));
        let _ = fs::remove_dir(&self.root);
    }
}

#[cfg(target_os = "linux")]
fn peer_pid(stream: &UnixStream) -> i32 {
    use std::os::fd::AsRawFd;
    // SO_PEERCRED returns the child's PID in the controller's PID namespace.
    // The child creates this socket after fork, before signalling its parent.
    let mut credentials = [0_i32; 3];
    let mut length = std::mem::size_of_val(&credentials) as u32;
    let result = unsafe {
        getsockopt(
            stream.as_raw_fd(),
            1,
            17,
            credentials.as_mut_ptr().cast(),
            &mut length,
        )
    };
    assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
    assert_eq!(length as usize, std::mem::size_of_val(&credentials));
    assert!(credentials[0] > 0);
    credentials[0]
}

#[cfg(target_os = "linux")]
fn assert_child_exited(pid: i32) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        // Signal zero observes this known fixture child; it never sends a signal.
        let result = unsafe { kill(pid, 0) };
        if result < 0 {
            assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(3));
            return;
        }
        assert!(
            Instant::now() < deadline,
            "closed channel is not proof child {pid} exited"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn getsockopt(
        fd: i32,
        level: i32,
        option: i32,
        value: *mut std::ffi::c_void,
        length: *mut u32,
    ) -> i32;
    fn kill(pid: i32, signal: i32) -> i32;
}

#[test]
fn new_session_helper_remains_protected_after_observed_tool_root_exit() {
    for guarded in [false, true] {
        let fixture = Fixture::new();
        let channel = Channel::new();
        fs::write(
            fixture.project.join("detached.py"),
            include_str!("detached.py"),
        )
        .unwrap();
        let mut request = fixture.request(
            "exec python3 detached.py \"$1/AGENTS.md\" \"$FLOW_TEST_SOCKET\"",
            limits(4096, 4096, 15_000),
        );
        request.resolved_policy.environment.insert(
            "FLOW_TEST_SOCKET".to_owned(),
            channel.root.join("channel").to_str().unwrap().to_owned(),
        );
        request.policy_digest = proto::resolved_policy_digest_v0(&request.resolved_policy).unwrap();
        let mut running = guarded.then(|| {
            let mut running = fixture.spawn(&request);
            running.ready(&request);
            running.start(&request);
            running
        });
        if !guarded {
            fixture.baseline(&request, &[]);
        }
        let mut stream = BufReader::new(channel.accept());
        #[cfg(target_os = "linux")]
        let pid = peer_pid(stream.get_ref());
        let mut line = String::new();
        stream.read_line(&mut line).unwrap();
        assert_eq!(line, "ready\n");
        if let Some(running) = &mut running {
            assert_success(&running.completed(&request));
        }
        // Both paths have now observed the actual Tool-root's successful exit.
        // Release cannot race ahead of that observation.
        let released = stream.get_mut().write_all(b"G");
        if let Err(error) = &released {
            assert!(
                matches!(
                    error.kind(),
                    std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
                ),
                "{error}"
            );
        }
        line.clear();
        stream.read_line(&mut line).unwrap();
        if guarded && cfg!(target_os = "linux") && line.is_empty() {
            // A private PID namespace may terminate this child at root exit.
            // EOF alone is insufficient: observe this exact child gone, not a
            // claim that root termination cleans up arbitrary descendants.
            #[cfg(target_os = "linux")]
            assert_child_exited(pid);
        } else {
            released.unwrap();
            let result: Value =
                serde_json::from_str(&line).expect("surviving child must report its write attempt");
            assert_eq!(result.as_object().unwrap().len(), 1);
            if guarded {
                assert!(
                    matches!(result["errno"].as_i64(), Some(1 | 13 | 30)),
                    "{result}"
                );
            } else {
                assert_eq!(result["errno"], 0);
            }
        }
        assert_eq!(
            fs::read(fixture.home.join("AGENTS.md")).unwrap(),
            if guarded {
                b"global instructions".as_slice()
            } else {
                b"changed"
            }
        );
    }
}
