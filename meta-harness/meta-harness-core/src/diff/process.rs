//! Running the host's `git`: bounded output, kept error text and a process group that ends with
//! it.

use super::*;
use std::future::{Future, poll_fn};
use std::os::fd::AsFd;
use std::pin::pin;

/// The one path a `git rev-parse` query in `dir` prints. A path may hold line breaks, so only
/// the single trailing one is taken off; splitting the output into lines could name another
/// repository.
pub(super) async fn git_path(dir: &Path, args: &[&str]) -> Result<String, DiffError> {
    let output = git_path_output(dir, &[], args)
        .await
        .map_err(|error| match error {
            DiffError::Git(reason) if reason.contains("not a git repository") => {
                DiffError::NotARepository
            }
            error => error,
        })?;
    match output.strip_suffix('\n') {
        Some(path) if !path.is_empty() => Ok(path.to_owned()),
        _ => Err(DiffError::NotARepository),
    }
}

/// Runs the host's `git` in `dir` and returns at most [`MAX_OUTPUT_BYTES`] of its output.
pub(super) async fn git(
    dir: &Path,
    env: &[(&str, OsString)],
    args: &[&str],
) -> Result<String, DiffError> {
    Ok(String::from_utf8_lossy(&git_bytes(dir, env, args).await?).into_owned())
}

/// Bounded machine output whose individual records may need exact decoding.
pub(super) async fn git_bytes(
    dir: &Path,
    env: &[(&str, OsString)],
    args: &[&str],
) -> Result<Vec<u8>, DiffError> {
    output(git_command(dir, env, args), false, None).await
}

/// Decodes ref identities without aliasing byte-distinct branches.
pub(super) fn ref_text(bytes: &[u8]) -> Result<&str, DiffError> {
    std::str::from_utf8(bytes).map_err(|_| {
        DiffError::Git("Git returned an unsupported non-UTF-8 ref identity".to_owned())
    })
}

/// Queries ref identities that must be represented exactly by the String API.
pub(super) async fn git_ref_output(dir: &Path, args: &[&str]) -> Result<String, DiffError> {
    ref_text(&git_bytes(dir, &[], args).await?).map(str::to_owned)
}

/// Reads Git configuration exactly, treating only its silent missing-key exit as empty output.
pub(super) async fn git_config(dir: &Path, args: &[&str]) -> Result<String, DiffError> {
    let mut command = git_command(dir, &[], &["config"]);
    command.args(args);
    String::from_utf8(output(command, true, None).await?).map_err(|_| {
        DiffError::Git("Git returned configuration that is not valid UTF-8".to_owned())
    })
}

/// Reads pathname output without replacing bytes that the String-based path API cannot name.
pub(super) async fn git_path_output(
    dir: &Path,
    env: &[(&str, OsString)],
    args: &[&str],
) -> Result<String, DiffError> {
    String::from_utf8(output(git_command(dir, env, args), false, None).await?)
        .map_err(|_| DiffError::Git("Git returned a pathname that is not valid UTF-8".to_owned()))
}

/// A Git timestamp represented without wrapping Liquid's Unix milliseconds.
pub(super) fn timestamp_ms(seconds: &str) -> Result<i64, DiffError> {
    seconds
        .parse::<i64>()
        .ok()
        .and_then(|seconds| seconds.checked_mul(1000))
        .ok_or_else(|| {
            DiffError::Git("Git returned a timestamp outside the supported range".to_owned())
        })
}

/// Runs the host's `git` in `dir` as the user's own would run, with the project's hooks and
/// settings, for a Git write the user asked for; it never asks for credentials.
pub(super) async fn user_git(dir: &Path, args: &[&str]) -> Result<String, DiffError> {
    let mut command = Command::new("git");
    command.args(args).current_dir(dir);
    user_output(Git::spawn(&mut piped(command, &NO_PROMPTS))?).await
}

/// Feeds literal NUL-delimited paths to an explicit write while draining its output to completion.
pub(super) async fn user_git_input(
    dir: &Path,
    args: &[&str],
    input: &[u8],
) -> Result<String, DiffError> {
    let mut command = Command::new("git");
    command.args(args).current_dir(dir);
    let mut command = piped(command, &NO_PROMPTS);
    let mut child = Git::spawn(command.stdin(Stdio::piped()))?;
    let mut stdin = child.child.stdin.take().expect("stdin is piped");
    let input = input.to_owned();
    let writing = tokio::spawn(async move { stdin.write_all(&input).await });
    let output = user_output(child).await;
    let written = writing.await;
    let output = output?;
    written
        .map_err(|error| DiffError::Io(std::io::Error::other(error)))?
        .map_err(unreadable)?;
    Ok(output)
}

async fn user_output(mut child: Git) -> Result<String, DiffError> {
    let mut stdout = child.child.stdout.take().expect("stdout is piped");
    let mut stderr = child.child.stderr.take().expect("stderr is piped");
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let status = {
        // Retain bounded diagnostics but drain everything while Git and its synchronous hooks
        // run. Unlike a query, excessive output must never interrupt an explicit write.
        let mut stdout = pin!(keep_into(
            &mut stdout,
            MAX_OUTPUT_BYTES as usize,
            &mut output
        ));
        let mut stderr = pin!(keep_into(&mut stderr, MAX_ERROR_BYTES, &mut errors));
        let mut ended = [false; 2];
        let mut drain = |context: &mut std::task::Context<'_>| {
            if !ended[0] {
                ended[0] = stdout.as_mut().poll(context).is_ready();
            }
            if !ended[1] {
                ended[1] = stderr.as_mut().poll(context).is_ready();
            }
        };
        let mut wait = pin!(child.wait());
        poll_fn(|context| {
            drain(context);
            wait.as_mut().poll(context)
        })
        .await
    }
    .map_err(unreadable)?;
    // Read bytes already in the nonblocking pipes even if the reactor has not delivered
    // their readiness yet. Never wait for a background hook descendant to close them.
    keep_ready(&stdout, MAX_OUTPUT_BYTES as usize, &mut output).map_err(unreadable)?;
    keep_ready(&stderr, MAX_ERROR_BYTES, &mut errors).map_err(unreadable)?;
    if !status.success() {
        return Err(DiffError::Git(
            format!(
                "{}\n{}",
                String::from_utf8_lossy(&output),
                String::from_utf8_lossy(&errors)
            )
            .trim()
            .to_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&output).into_owned())
}

/// Runs the host's `gh` (`program`) in `dir` with bounded output and optional input. It never
/// prompts, and its login stays its own: Liquid passes no credential.
pub(super) async fn gh(
    dir: &Path,
    program: &std::ffi::OsStr,
    args: &[&str],
    input: Option<&[u8]>,
) -> Result<Vec<u8>, DiffError> {
    let mut command = Command::new(program);
    command.args(args).current_dir(dir);
    let mut command = piped(command, &NO_PROMPTS);
    command.envs([
        ("GH_PROMPT_DISABLED", "1"),
        ("GH_NO_UPDATE_NOTIFIER", "1"),
        ("GH_NO_EXTENSION_UPDATE_NOTIFIER", "1"),
        ("GH_SPINNER_DISABLED", "1"),
        ("GH_PAGER", "cat"),
        ("NO_COLOR", "1"),
    ]);
    output(command, false, input).await
}

/// Keeps Git and its credential helpers from asking on a terminal or in a window, as T3 Code
/// does.
pub(super) const NO_PROMPTS: [(&str, &str); 5] = [
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GIT_ASKPASS", ""),
    ("SSH_ASKPASS", ""),
    // Force the empty helper even with a controlling terminal, so SSH cannot prompt there.
    ("SSH_ASKPASS_REQUIRE", "force"),
    ("GCM_INTERACTIVE", "never"),
];

/// The non-interactive environment for the checked Git reader.
pub(super) fn no_prompts() -> Vec<(&'static str, OsString)> {
    NO_PROMPTS
        .iter()
        .map(|(name, value)| (*name, OsString::from(value)))
        .collect()
}

/// Runs `command` and returns at most [`MAX_OUTPUT_BYTES`] of its output.
async fn output(
    mut command: Command,
    missing_config: bool,
    input: Option<&[u8]>,
) -> Result<Vec<u8>, DiffError> {
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = Git::spawn(&mut command)?;
    // Git's warnings drain meanwhile, so a full error pipe never blocks it; dropped, the set ends
    // the drain.
    let stderr = child.child.stderr.take().expect("stderr is piped");
    let mut errors = tokio::task::JoinSet::new();
    errors.spawn(keep_start(stderr, MAX_ERROR_BYTES));
    let mut output = Vec::new();
    // Reading one byte past the limit tells a cut-off output apart; dropping the child then
    // stops Git instead of buffering unbounded output.
    let stdout = child.child.stdout.take().expect("stdout is piped");
    let mut writes = tokio::task::JoinSet::new();
    if let (Some(mut stdin), Some(input)) = (child.child.stdin.take(), input) {
        let input = input.to_owned();
        writes.spawn(async move { stdin.write_all(&input).await.map_err(unreadable) });
    }
    stdout
        .take(MAX_OUTPUT_BYTES + 1)
        .read_to_end(&mut output)
        .await
        .map_err(unreadable)?;
    if output.len() as u64 > MAX_OUTPUT_BYTES {
        return Err(DiffError::TooLarge);
    }
    if let Some(written) = writes.join_next().await {
        written.map_err(|error| DiffError::Io(std::io::Error::other(error)))??;
    }
    // A program Git started can hold its error output after Git ended. Git is reaped only once
    // that output ended too, so until then its time limit still ends such programs with it.
    let errors = errors
        .join_next()
        .await
        .and_then(Result::ok)
        .unwrap_or_default();
    let status = child
        .wait()
        .await
        .map_err(|error| DiffError::Git(error.to_string()))?;
    if !status.success() {
        if missing_config && status.code() == Some(1) && errors.is_empty() {
            return Ok(Vec::new());
        }
        return Err(DiffError::Git(
            String::from_utf8_lossy(&errors).trim().to_owned(),
        ));
    }
    Ok(output)
}

/// The host's `git` in `dir` with its output piped. Project settings that would run something
/// or let Git skip a changed file are overridden.
pub(super) fn git_command(dir: &Path, env: &[(&str, OsString)], args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .args([
            // Reading the project runs none of its hooks or file monitors, which could change it.
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            // Every file's state is checked in full, so no setting hides a change.
            "-c",
            "core.ignoreStat=false",
            "-c",
            "core.checkStat=default",
            "-c",
            "core.trustctime=true",
            "-c",
            "core.fileMode=true",
            "-c",
            "core.sparseCheckout=false",
            // No split index lands in the project's Git directory, and a submodule's diff runs
            // none of its commands.
            "-c",
            "core.splitIndex=false",
            "-c",
            "diff.submodule=short",
            // Blank context lines keep their leading space, which Liquid reads as context.
            "-c",
            "diff.suppressBlankEmpty=false",
        ])
        .args(args)
        .current_dir(dir);
    piped(command, env)
}

/// `command` with its output piped, in a process group of its own.
fn piped(mut command: Command, env: &[(&str, impl AsRef<std::ffi::OsStr>)]) -> Command {
    command
        // A group of its own, so Git ends with the programs it starts (see [`Git`]).
        .process_group(0)
        // Git's messages in English, as Liquid shows them and reads one.
        .env("LC_ALL", "C")
        .envs(env.iter().map(|(name, value)| (name, value)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

/// A failed read of Git's output or write of its input.
pub(super) fn unreadable(error: std::io::Error) -> DiffError {
    DiffError::Git(error.to_string())
}

/// A running Git. Dropped before it ended, as at its time limit, it ends with the programs it
/// started that stay in its process group, such as a project's clean filter (ADR-0210).
pub(super) struct Git {
    pub(super) child: Child,
    /// Git's process group; `None` once Git was reaped, since its id may then be reused.
    group: Option<rustix::process::Pid>,
}

impl Git {
    pub(super) fn spawn(command: &mut Command) -> Result<Self, DiffError> {
        let child = crate::spawn(command).map_err(DiffError::GitStart)?;
        let group = child
            .id()
            .and_then(|id| i32::try_from(id).ok())
            .and_then(rustix::process::Pid::from_raw);
        Ok(Self { child, group })
    }

    pub(super) async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        let status = self.child.wait().await;
        if status.is_ok() {
            self.group = None;
        }
        status
    }
}

impl Drop for Git {
    fn drop(&mut self) {
        if let Some(group) = self.group {
            let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
        }
    }
}

/// Reads `reader` to its end and keeps at most `limit` bytes from its start.
async fn keep_start(reader: impl AsyncRead + Unpin, limit: usize) -> Vec<u8> {
    let mut kept = Vec::new();
    keep_into(reader, limit, &mut kept).await;
    kept
}

/// Drains a pipe, retaining only its bounded start even if its future is later cancelled.
async fn keep_into(mut reader: impl AsyncRead + Unpin, limit: usize, kept: &mut Vec<u8>) {
    let mut chunk = [0; 8192];
    while let Ok(read @ 1..) = reader.read(&mut chunk).await {
        let room = limit.saturating_sub(kept.len());
        kept.extend_from_slice(&chunk[..read.min(room)]);
    }
}

/// Keeps only immediately available bytes from a Tokio pipe, within the remaining budget.
fn keep_ready(reader: impl AsFd, limit: usize, kept: &mut Vec<u8>) -> std::io::Result<()> {
    if kept.len() >= limit {
        return Ok(());
    }
    // Duplication preserves O_NONBLOCK; converting Tokio's pipe into an owned fd clears it.
    let mut reader = std::fs::File::from(reader.as_fd().try_clone_to_owned()?);
    let mut chunk = [0; 8192];
    while kept.len() < limit {
        let room = chunk.len().min(limit - kept.len());
        match std::io::Read::read(&mut reader, &mut chunk[..room]) {
            Ok(read @ 1..) => kept.extend_from_slice(&chunk[..read]),
            Ok(_) => break,
            Err(error) if error.kind() == ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
#[test]
fn explicit_git_retains_output_when_the_child_exits_before_the_first_poll() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        for exit in [0, 1] {
            let mut command = Command::new("sh");
            command.args([
                "-c",
                &format!("printf retained; printf diagnostic >&2; exit {exit}"),
            ]);
            let mut child = Git::spawn(&mut piped(command, &NO_PROMPTS)).expect("child");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            // Keep the reactor from running until the direct child has exited. Its pipe
            // data is available even though no async readiness has been delivered yet.
            while child.child.try_wait().expect("child status").is_none() {
                assert!(std::time::Instant::now() < deadline, "child did not exit");
                std::thread::yield_now();
            }
            let result = user_output(child).await;
            match exit {
                0 => assert_eq!(result.expect("successful child"), "retained"),
                _ => assert!(
                    matches!(result, Err(DiffError::Git(ref text)) if text == "retained\ndiagnostic"),
                    "{result:?}"
                ),
            }
        }
    });
}

#[cfg(test)]
#[test]
fn explicit_git_writes_drain_stdout_past_the_retained_limit() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let dir =
            std::env::temp_dir().join(format!("watershed-user-output-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("fixture directory");
        // Git forwards hook stdout to stderr. A shell alias exercises its actual stdout pipe
        // and writes a completion marker only if every output byte was accepted.
        let result = user_git(
            &dir,
            &[
                "-c",
                "alias.noisy=!head -c 5242880 /dev/zero && touch finished",
                "noisy",
            ],
        )
        .await;
        let finished = dir.join("finished").exists();
        std::fs::remove_dir_all(&dir).expect("cleanup");
        assert!(finished, "explicit write was interrupted");
        assert_eq!(
            result.expect("write succeeded").len() as u64,
            MAX_OUTPUT_BYTES
        );
    });
}

/// Bounded pathname output while feeding a batch through stdin.
pub(super) async fn git_path_input(
    dir: &Path,
    args: &[&str],
    input: &[u8],
) -> Result<String, DiffError> {
    String::from_utf8(output(git_command(dir, &[], args), false, Some(input)).await?)
        .map_err(|_| DiffError::Git("Git returned a pathname that is not valid UTF-8".to_owned()))
}

/// The paths of NUL-separated output, each once.
pub(super) fn paths(output: &str) -> Vec<String> {
    let unique: BTreeSet<&str> = output.split('\0').filter(|path| !path.is_empty()).collect();
    unique.into_iter().map(str::to_owned).collect()
}
