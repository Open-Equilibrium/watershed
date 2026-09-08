mod acceptance;
mod artifact;
pub mod builds;
mod lifetime;
mod pty;

use proto::{ExecutorLimitsV0, ExecutorRequestV0, ExecutorResponseV0};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{fs::MetadataExt, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Output, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub struct Fixture {
    pub root: PathBuf,
    pub home: PathBuf,
    pub project: PathBuf,
    pub image: PathBuf,
    objects: Vec<proto::ExecutorProtectedObjectV0>,
    handles: Vec<std::os::fd::OwnedFd>,
}

impl Fixture {
    pub fn new() -> Self {
        Self::with_image_name("flow-executor")
    }

    pub fn with_image_name(image_name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "flow-native-contract-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let home = root.join("flow-home");
        let project = root.join("project");
        fs::create_dir(&home).unwrap();
        fs::create_dir(&project).unwrap();
        fs::write(home.join("AGENTS.md"), b"global instructions").unwrap();
        let image = root.join(image_name);
        fs::copy(artifact::executor_artifact(), &image).unwrap();
        let mut fixture = Self {
            root,
            home,
            project,
            image,
            objects: Vec::new(),
            handles: Vec::new(),
        };
        for path in [fixture.home.clone(), fixture.image.clone()] {
            fixture.protect(&path);
        }
        fixture
    }

    pub fn protect(&mut self, path: &Path) {
        let file = File::open(path).unwrap();
        let metadata = file.metadata().unwrap();
        self.handles
            .push(rustix::io::fcntl_dupfd_cloexec(&file, 256).unwrap());
        self.objects.push(proto::ExecutorProtectedObjectV0 {
            descriptor: proto::EXECUTOR_PROTECTED_DESCRIPTOR_BASE_V0 + self.objects.len() as u32,
            path: path.to_str().unwrap().to_owned(),
            identity: proto::UnixObjectIdentityV0 {
                device: metadata.dev(),
                inode: metadata.ino(),
                kind: if metadata.is_dir() {
                    proto::ExecutorObjectKindV0::Directory
                } else {
                    proto::ExecutorObjectKindV0::File
                },
            },
        });
    }

    pub fn operations(&self, rows: serde_json::Value, wait: bool) -> ExecutorRequestV0 {
        fs::write(
            self.project.join("operations.py"),
            include_str!("operations.py"),
        )
        .unwrap();
        fs::write(
            self.project.join("operations.json"),
            serde_json::to_vec(&serde_json::json!({"operations": rows, "wait": wait})).unwrap(),
        )
        .unwrap();
        self.request("exec python3 operations.py", limits(4096, 4096, 15_000))
    }

    pub fn operation_results(&self) -> Vec<serde_json::Value> {
        serde_json::from_slice(&fs::read(self.project.join("operations-result.json")).unwrap())
            .unwrap()
    }

    pub fn request(&self, script: &str, limits: ExecutorLimitsV0) -> ExecutorRequestV0 {
        let resolved_policy = proto::ExecutorResolvedPolicyV0 {
            executable: "/bin/sh".to_owned(),
            argv: vec![
                "-c".to_owned(),
                script.to_owned(),
                "contract-tool".to_owned(),
                self.home.to_str().unwrap().to_owned(),
            ],
            environment: BTreeMap::from([
                ("PATH".to_owned(), std::env::var("PATH").unwrap()),
                ("HOME".to_owned(), self.project.to_str().unwrap().to_owned()),
            ]),
            limits,
            protected_objects: self.objects.clone(),
            tool_id: "native-contract".to_owned(),
            tool_kind: "own-script".to_owned(),
            working_directory: self.project.to_str().unwrap().to_owned(),
        };
        ExecutorRequestV0 {
            policy_digest: proto::resolved_policy_digest_v0(&resolved_policy).unwrap(),
            request_id: "native-contract".to_owned(),
            resolved_policy,
            schema: proto::EXECUTOR_REQUEST_SCHEMA_V0.to_owned(),
        }
    }

    pub fn spawn(&self, request: &ExecutorRequestV0) -> Running {
        self.spawn_with_handles(request, &[])
    }

    pub fn spawn_with_handles(&self, request: &ExecutorRequestV0, extra: &[(i32, i32)]) -> Running {
        let mut command = Command::new(&self.image);
        command
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(coverage)]
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let mut remaps = self
            .handles
            .iter()
            .zip(&self.objects)
            .map(|(handle, object)| (handle.as_raw_fd(), object.descriptor as i32))
            .collect::<Vec<_>>();
        remaps.extend_from_slice(extra);
        inherit_handles(&mut command, remaps);
        let mut child = command.spawn().unwrap();
        let mut input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (sender, records) = mpsc::channel();
        thread::spawn(move || {
            for line in
                BufReader::new(stdout.take(proto::MAX_EXECUTOR_RESPONSE_BYTES_V0 as u64 + 4096))
                    .lines()
            {
                if sender.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        let (diagnostics_sender, diagnostics) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stderr.take(16 * 1024).read_to_end(&mut bytes);
            let _ = diagnostics_sender.send(bytes);
        });
        input
            .write_all(&proto::canonical_executor_request_v0(request).unwrap())
            .unwrap();
        input.flush().unwrap();
        Running {
            child,
            input: Some(input),
            records,
            diagnostics,
        }
    }

    pub fn run(&self, script: &str, limits: ExecutorLimitsV0) -> proto::ExecutorToolResultV0 {
        let request = self.request(script, limits);
        let mut running = self.spawn(&request);
        running.ready(&request);
        running.start(&request);
        running.completed(&request)
    }

    pub fn baseline(&self, request: &ExecutorRequestV0, handles: &[(i32, i32)]) -> Output {
        let policy = &request.resolved_policy;
        let mut command = Command::new(&policy.executable);
        command
            .args(&policy.argv)
            .env_clear()
            .envs(&policy.environment)
            .current_dir(&policy.working_directory)
            .stdin(Stdio::null());
        inherit_handles(&mut command, handles.to_vec());
        // File capture avoids a full pipe blocking the controlled baseline.
        let stdout = self.project.join("baseline.stdout");
        let stderr = self.project.join("baseline.stderr");
        command
            .stdout(File::create(&stdout).unwrap())
            .stderr(File::create(&stderr).unwrap());
        let mut child = command.spawn().expect("unprotected control must launch");
        let deadline = Instant::now() + Duration::from_millis(policy.limits.timeout_ms + 2000);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("unprotected control exceeded its deadline");
            }
            thread::sleep(Duration::from_millis(5));
        };
        let read = |path: &Path| {
            let mut bytes = Vec::new();
            File::open(path)
                .unwrap()
                .take(64 * 1024 + 1)
                .read_to_end(&mut bytes)
                .unwrap();
            assert!(
                bytes.len() <= 64 * 1024,
                "baseline output must stay bounded"
            );
            bytes
        };
        let output = Output {
            status,
            stdout: read(&stdout),
            stderr: read(&stderr),
        };
        assert!(
            output.status.success(),
            "unprotected control failed: {output:?}"
        );
        output
    }
}

fn inherit_handles(command: &mut Command, remaps: Vec<(i32, i32)>) {
    // Pin every source above the fixed target slots before any dup2. Parallel
    // fixtures can otherwise place a source in another remap's destination.
    let handles = remaps
        .into_iter()
        .map(|(source, target)| {
            // Each source is a live parent-owned fixture handle at this call site.
            let source = unsafe { std::os::fd::BorrowedFd::borrow_raw(source) };
            (
                rustix::io::fcntl_dupfd_cloexec(source, 256).unwrap(),
                target,
            )
        })
        .collect::<Vec<_>>();
    unsafe {
        command.pre_exec(move || {
            for (source, target) in &handles {
                if dup2(source.as_raw_fd(), *target) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
}

pub fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.is_file() {
        assert!(
            Instant::now() < deadline,
            "helper did not publish {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(5));
    }
}

pub fn assert_success(result: &proto::ExecutorToolResultV0) {
    assert_eq!(
        result.status,
        proto::ExecutorToolStatusV0::Completed,
        "{result:?}"
    );
    assert_eq!(result.exit_code, Some(0), "{result:?}");
}

impl Running {
    pub fn completed(&mut self, request: &ExecutorRequestV0) -> proto::ExecutorToolResultV0 {
        let response = self.finish(request);
        let ExecutorResponseV0::Completed {
            enforcement,
            tool_result,
            ..
        } = response
        else {
            panic!("native execution must provide a definitive result: {response:?}");
        };
        proto::validate_enforcement_receipt_v0(&enforcement, &request.policy_digest).unwrap();
        assert!(enforcement.self_protection_active);
        tool_result
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub struct Running {
    pub child: Child,
    input: Option<ChildStdin>,
    records: mpsc::Receiver<String>,
    diagnostics: mpsc::Receiver<Vec<u8>>,
}

impl Running {
    pub fn record(&self) -> String {
        self.records
            .recv_timeout(Duration::from_secs(20))
            .unwrap_or_else(|error| {
                let diagnostics = self.diagnostics.try_recv().unwrap_or_default();
                panic!(
                    "Executor record failed: {error}; {}",
                    String::from_utf8_lossy(&diagnostics)
                )
            })
    }

    pub fn ready(&self, request: &ExecutorRequestV0) {
        assert!(matches!(
            proto::parse_executor_preflight_v0(
                format!("{}\n", self.record()).as_bytes(),
                &request.request_id
            )
            .unwrap(),
            proto::ExecutorPreflightV0::Ready { .. }
        ));
    }

    pub fn start(&mut self, request: &ExecutorRequestV0) {
        let mut input = self.input.take().unwrap();
        input
            .write_all(
                &proto::canonical_executor_start_v0(&proto::ExecutorStartV0 {
                    schema: proto::EXECUTOR_START_SCHEMA_V0.to_owned(),
                    request_id: request.request_id.clone(),
                })
                .unwrap(),
            )
            .unwrap();
        input.flush().unwrap();
    }

    pub fn finish(&mut self, request: &ExecutorRequestV0) -> ExecutorResponseV0 {
        proto::parse_executor_response_v0(
            format!("{}\n", self.record()).as_bytes(),
            &request.request_id,
            &request.policy_digest,
        )
        .unwrap()
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.child.try_wait().unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "test Executor could not be reaped"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
}

pub fn limits(stdout: u64, stderr: u64, timeout_ms: u64) -> ExecutorLimitsV0 {
    ExecutorLimitsV0 {
        max_stdout_bytes: stdout,
        max_stderr_bytes: stderr,
        timeout_ms,
    }
}

unsafe extern "C" {
    fn dup2(source: i32, target: i32) -> i32;
}
