#![cfg(any(
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64")
))]

use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{BufRead as _, BufReader, Write as _},
    os::{
        fd::AsRawFd as _,
        unix::fs::{MetadataExt as _, PermissionsExt as _},
        unix::process::CommandExt as _,
    },
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

// Only fixed test descriptor slots are duplicated, before the Executor starts.
unsafe extern "C" {
    fn dup2(old: i32, new: i32) -> i32;
}

#[test]
fn native_executor_protects_own_files_and_leaves_project_work_available() {
    let temporary =
        std::env::temp_dir().join(format!("flow-native-protection-{}", std::process::id()));
    fs::create_dir(&temporary).unwrap();
    let root = temporary.canonicalize().unwrap();
    let home = root.join("flow-home");
    let project = root.join("project");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&project).unwrap();
    fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
    let protected = home.join("AGENTS.md");
    fs::write(&protected, b"global instructions").unwrap();
    let image = root.join("flow-executor");
    fs::copy(env!("CARGO_BIN_EXE_flow-executor"), &image).unwrap();
    let installed_flow = root.join("flow");
    fs::write(&installed_flow, b"installed program").unwrap();
    let handles = [
        File::open(&home).unwrap(),
        File::open(&image).unwrap(),
        File::open(&installed_flow).unwrap(),
    ];
    let objects = [&home, &image, &installed_flow]
        .into_iter()
        .zip(&handles)
        .enumerate()
        .map(|(index, (path, handle))| {
            let metadata = handle.metadata().unwrap();
            serde_json::json!({
                "descriptor": 32 + index,
                "identity": {"device": metadata.dev(), "inode": metadata.ino(),
                    "kind": if metadata.is_dir() { "directory" } else { "file" }},
                "path": path.to_str().unwrap()
            })
        })
        .collect::<Vec<_>>();
    let policy = serde_json::json!({
        "argv": ["-c", concat!(
            "set -eu\n",
            "printf local > AGENTS.md\n",
            "printf build > result.txt\n",
            "if (printf changed > \"$1/AGENTS.md\") 2>/dev/null; then exit 21; fi\n",
            "if /bin/rm \"$1/AGENTS.md\" 2>/dev/null; then exit 22; fi\n",
            "if /bin/sh -c 'printf child > \"$1/AGENTS.md\"' child \"$1\" 2>/dev/null; then exit 23; fi\n",
            "if (printf replaced > \"$2\") 2>/dev/null; then exit 24; fi\n",
            "if /bin/mv \"$1\" \"$1-moved\" 2>/dev/null; then exit 25; fi\n",
            "printf complete\n"
        ), "guard-test", home.to_str().unwrap(), installed_flow.to_str().unwrap()],
        "environment": {"PATH": "/usr/bin:/bin"},
        "executable": "/bin/sh",
        "limits": {"max_stderr_bytes": 4096, "max_stdout_bytes": 4096, "timeout_ms": 10000},
        "protected_objects": objects,
        "tool_id": "native-control",
        "tool_kind": "own-script",
        "working_directory": project.to_str().unwrap()
    });
    // Use canonical JSON rather than the Rust request type so this behavior test
    // also runs against the unreplaced Executor during the red-first migration.
    let policy_bytes = wire(&policy);
    let digest = Sha256::digest(&policy_bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let request_id = "native-protection-1";
    let request = serde_json::json!({
        "policy_digest": digest,
        "request_id": request_id,
        "resolved_policy": policy,
        "schema": "flow-executor-request-v0"
    });
    let mut command = Command::new(&image);
    command
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(coverage)]
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    let descriptors = handles.each_ref().map(|handle| handle.as_raw_fd());
    unsafe {
        command.pre_exec(move || {
            for (index, descriptor) in descriptors.iter().enumerate() {
                if dup2(*descriptor, 32 + index as i32) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let mut child = command.spawn().unwrap();
    let mut input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    input.write_all(&wire(&request)).unwrap();
    input.flush().unwrap();
    let result = (|| {
        let read_record = || -> Result<serde_json::Value, String> {
            let line = receiver
                .recv_timeout(Duration::from_secs(20))
                .map_err(|error| format!("Executor did not publish its record: {error}"))?
                .map_err(|error| error.to_string())?;
            serde_json::from_str(&line).map_err(|error| error.to_string())
        };
        let ready = read_record()?;
        if ready["outcome"] != "ready" {
            return Err(format!("native self-protection was not ready: {ready}"));
        }
        input
            .write_all(&wire(&serde_json::json!({
                "request_id": request_id, "schema": "flow-executor-start-v0"
            })))
            .map_err(|error| error.to_string())?;
        input.flush().map_err(|error| error.to_string())?;
        let terminal = read_record()?;
        if terminal["outcome"] != "completed"
            || terminal["tool_result"]["status"] != "completed"
            || terminal["tool_result"]["exit_code"] != 0
            || terminal["tool_result"]["stdout_base64"] != "Y29tcGxldGU="
            || terminal["enforcement"]["self_protection_active"] != true
            || terminal["enforcement"]["applied_policy_digest"] != digest
        {
            return Err(format!(
                "native protected execution did not complete: {terminal}"
            ));
        }
        Ok(())
    })();
    drop(input);
    let _ = child.kill();
    let _ = child.wait();
    drop(receiver);
    // A failed Executor may leave an inherited pipe open; the test's response
    // deadline, not joining that pipe reader, must bound the failure report.
    if reader.is_finished() {
        reader.join().unwrap();
    }
    let observed = [
        fs::read(&protected),
        fs::read(&installed_flow),
        fs::read(project.join("AGENTS.md")),
        fs::read(project.join("result.txt")),
    ];
    fs::remove_dir_all(&root).unwrap();
    result.expect("native Executor must enforce the approved own-file boundary");
    for (actual, expected) in observed.into_iter().zip([
        b"global instructions".as_slice(),
        b"installed program",
        b"local",
        b"build",
    ]) {
        assert_eq!(actual.unwrap(), expected);
    }
}

fn wire(value: &serde_json::Value) -> Vec<u8> {
    let mut bytes = proto::canonical_json(value).unwrap().into_bytes();
    bytes.push(b'\n');
    bytes
}
