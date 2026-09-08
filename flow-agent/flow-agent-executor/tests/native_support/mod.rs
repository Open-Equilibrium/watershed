use proto::{ExecutorLimitsV0, ExecutorRequestV0, ExecutorResponseV0};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{fs::MetadataExt, process::CommandExt},
    },
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
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
        let image = root.join("flow-executor");
        let artifact = std::env::var_os("FLOW_EXECUTOR_UNDER_TEST")
            .map(PathBuf::from)
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_flow-executor").into());
        fs::copy(artifact, &image).unwrap();
        let mut objects = Vec::new();
        let mut handles = Vec::new();
        for (index, path) in [&home, &image].into_iter().enumerate() {
            let file = File::open(path).unwrap();
            let metadata = file.metadata().unwrap();
            handles.push(rustix::io::fcntl_dupfd_cloexec(&file, 256).unwrap());
            objects.push(proto::ExecutorProtectedObjectV0 {
                descriptor: proto::EXECUTOR_PROTECTED_DESCRIPTOR_BASE_V0 + index as u32,
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
        Self {
            root,
            home,
            project,
            image,
            objects,
            handles,
        }
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
        let remaps = self
            .handles
            .iter()
            .zip(&self.objects)
            .map(|(handle, object)| (handle.as_raw_fd(), object.descriptor as i32))
            .collect::<Vec<_>>();
        unsafe {
            command.pre_exec(move || {
                for &(source, target) in &remaps {
                    if dup2(source, target) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
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
        let response = running.finish(&request);
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
