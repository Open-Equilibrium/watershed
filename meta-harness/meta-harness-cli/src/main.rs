mod agents;
mod hosts;
mod install;
mod login;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use meta_harness_core::{
    AcpAgent, Change, Service, ServiceConfig, api, default_home, remote, t3_version,
};

const USAGE: &str = "\
usage: meta <command>

  help                      show this usage
  serve                     run Meta-Harness in the foreground (the login service runs this)
  status                    show whether Meta-Harness runs, its T3 Code release and revision
  config show               print the current configuration revision as JSON
  config propose FILE [SUMMARY]
                            propose the changes in FILE, a JSON array of changes (format:
                            docs/concept/V-Spec_MetaHarness.html#config)
  config revert REVISION    propose going back to an earlier revision
  config pending            list the proposals that wait for a decision
  config approve NUMBER     approve a proposal; new harness sessions use the new revision
  config reject NUMBER      reject a proposal
  config log                print the append-only configuration log
  remote on [--port N]      listen for other hosts and devices on the Tailscale addresses
                            (port 3774 by default); it stays on across restarts
  remote off                stop listening there
  remote status             show whether remote access is on, and where
  invite --device | --host [--access LEVEL]
                            print a pairing code for another device or host, valid 5 minutes; a
                            host's LEVEL is the most its agents get here, auto by default
  host add ADDRESS CODE [--access LEVEL]
                            link the host at ADDRESS with its pairing code; LEVEL is read-only,
                            approval-required, auto-accept-edits, auto or full-access, and asks
                            for less than the host invited, which is the default
  host list                 print the linked hosts and the clients paired here as JSON
  host remove NAME          unlink a host, and end its T3 link
  host revoke NAME          refuse a paired client's token from its next connection on, and
                            end the T3 sessions of that host's link
  host agents on|off        accept agents from other hosts here (off until turned on)
  agent create NAME --project PATH --model INSTANCE/MODEL [--permissions MODE]
        [--instructions TEXT]
                            create a persistent agent: its own thread in PATH and a private
                            memory folder here; MODE is approval-required (the default),
                            auto-accept-edits, auto or full-access
  agent list                print the persistent agents as JSON
  agent remove NAME         remove an agent and its routines; its memory folder stays
  agent routines NAME       print the agent's routines as JSON
  agent routine add NAME (--every 30m|2h|1d | --at HH:MM [--days mon,fri] | --once ISO
        | --webhook) --prompt TEXT [--title TEXT]
                            wake the agent on a schedule or webhook; --once takes a date and
                            time with an offset, such as 2026-12-01T07:30:00+01:00
  agent routine remove NAME ID
                            remove one of the agent's routines
  install [--prefix PATH] [--no-service]
                            on Windows: install this download and start the login service
  service install           start the login service, also after each login and crash
  service uninstall         stop and remove the login service
  uninstall                 remove the login service and this installation; data stays

Meta-Harness keeps its data in META_HARNESS_HOME, else ~/.meta-harness.";

const CLIENT: &str = "meta";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (environment, args) = login::split_environment(&args);
    for (name, value) in environment {
        unsafe { std::env::set_var(name, value) }
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("the runtime starts");
    match runtime.block_on(run(&args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("meta: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: &[&str]) -> Result<(), String> {
    if matches!(args, [] | ["help" | "--help" | "-h"]) {
        println!("{USAGE}");
        return Ok(());
    }
    if let ["install", rest @ ..] = args {
        return install_download(rest);
    }
    if let Some(message) = missing(args) {
        return Err(message.to_owned());
    }
    let home = default_home()?;
    match args {
        ["serve"] => serve(home).await,
        ["serve", "--home", folder, "--log", log] => {
            log_to(Path::new(log))?;
            serve(PathBuf::from(folder)).await
        }
        ["status"] => {
            let status = connect(&home).await?.status().await?;
            println!("{}", status_text(&status));
            Ok(())
        }
        ["config", rest @ ..] => config(&connect(&home).await?, rest).await,
        ["remote", rest @ ..] => hosts::remote(&connect(&home).await?, rest).await,
        ["invite", rest @ ..] => hosts::invite(&connect(&home).await?, rest).await,
        ["host", rest @ ..] => hosts::host(&connect(&home).await?, rest).await,
        ["agent", rest @ ..] => agents::agent(&connect(&home).await?, rest).await,
        ["service", "install"] => install(home),
        ["service", "uninstall"] => remove_login_service(),
        ["uninstall"] => uninstall(),
        _ => Err(format!("unknown command \"{}\"\n{USAGE}", args.join(" "))),
    }
}

fn missing(args: &[&str]) -> Option<&'static str> {
    Some(match args {
        ["config", "propose"] => "meta config propose needs a change file",
        ["config", "revert"] => "meta config revert needs a revision number",
        ["config", "approve"] => "meta config approve needs a proposal number",
        ["config", "reject"] => "meta config reject needs a proposal number",
        ["host", "add"] => "meta host add needs an address and a pairing code",
        ["host", "add", _] => "meta host add needs a pairing code",
        ["host", "remove"] => "meta host remove needs a host name",
        ["host", "revoke"] => "meta host revoke needs a host name",
        ["agent", "create"] => "meta agent create needs an agent name",
        ["agent", "remove"] => "meta agent remove needs an agent name",
        ["agent", "routines"] => "meta agent routines needs an agent name",
        ["agent", "routine", "add"] => "meta agent routine add needs an agent name",
        ["agent", "routine", "remove"] => {
            "meta agent routine remove needs an agent name and a routine ID"
        }
        ["agent", "routine", "remove", _] => "meta agent routine remove needs a routine ID",
        ["service"] => "meta service needs install or uninstall",
        _ => return None,
    })
}

fn status_text(status: &api::Status) -> String {
    let mut text = format!(
        "Meta-Harness runs as process {} with T3 Code {}, configuration revision {}.",
        status.pid, status.t3_version, status.revision
    );
    if !status.reconciled {
        text.push_str(&format!(
            "\nThe harness homes are not reconciled with revision {}: approving a change writes \
             the new revision into them, and is refused while that fails.",
            status.revision
        ));
    }
    text
}

async fn connect(home: &Path) -> Result<remote::Host, String> {
    remote::Host::connect(home, CLIENT).await
}

fn print(value: &impl serde::Serialize) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    println!("{text}");
    Ok(())
}

fn number(text: &str) -> Result<u64, String> {
    text.parse()
        .map_err(|_| format!("{text:?} is not a number."))
}

async fn config(host: &remote::Host, args: &[&str]) -> Result<(), String> {
    match args {
        ["show"] => print(&host.config().await?),
        ["propose", file, summary @ ..] => {
            let text =
                std::fs::read_to_string(file).map_err(|e| format!("Could not read {file}: {e}"))?;
            let changes: Vec<Change> = serde_json::from_str(&text)
                .map_err(|e| format!("{file} holds no list of changes: {e}"))?;
            let summary = summary.join(" ");
            print(&host.propose(&summary, changes).await?)
        }
        ["revert", revision] => print(&host.propose_revert(number(revision)?).await?),
        ["pending"] => print(&host.pending().await?),
        ["approve", proposal] => print(&host.approve(number(proposal)?).await?),
        ["reject", proposal] => {
            let number = number(proposal)?;
            host.reject(number).await?;
            println!("{}", rejected(number));
            Ok(())
        }
        ["log"] => print(&host.log().await?),
        _ => Err(format!("unknown config command\n{USAGE}")),
    }
}

fn rejected(number: u64) -> String {
    format!("Rejected proposal {number}.")
}

async fn serve(home: PathBuf) -> Result<(), String> {
    let service = Service::start(ServiceConfig {
        home,
        t3: pinned_t3()?,
        acp_agents: tokio::task::spawn_blocking(|| flow_agent(std::env::var_os("PATH")))
            .await
            .ok()
            .flatten()
            .into_iter()
            .collect(),
    })
    .await?;
    eprintln!(
        "Meta-Harness serves on {} with T3 Code {}.",
        service.url,
        t3_version()
    );
    stopped().await;
    service.stop().await;
    Ok(())
}

#[cfg(unix)]
fn log_to(_log: &Path) -> Result<(), String> {
    Err("--log is for the Windows login task; launchd and systemd keep the log.".to_owned())
}

#[cfg(windows)]
fn log_to(log: &Path) -> Result<(), String> {
    use std::os::windows::io::IntoRawHandle;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .map_err(|e| format!("Could not open the log {}: {e}", log.display()))?;
    let handle = file.into_raw_handle();
    unsafe {
        windows_sys::Win32::System::Console::SetStdHandle(
            windows_sys::Win32::System::Console::STD_ERROR_HANDLE,
            handle,
        );
    }
    Ok(())
}

#[cfg(unix)]
async fn stopped() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate()).expect("signals can be followed");
    tokio::select! {
        _ = terminate.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}

#[cfg(not(unix))]
async fn stopped() {
    let _ = tokio::signal::ctrl_c().await;
}

fn prefix() -> Result<PathBuf, String> {
    let program = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map_err(|e| format!("Could not find this program: {e}"))?;
    program
        .parent()
        .filter(|bin| bin.ends_with("bin"))
        .and_then(Path::parent)
        .filter(|prefix| prefix.join("lib/meta-harness").is_dir())
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("{} is not an installed meta.", program.display()))
}

#[cfg(unix)]
fn answers_as_flow_agent(flow: &Path) -> bool {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    const PROBE_OUTPUT_BYTES: u64 = 64 * 1024;
    let Ok(mut child) = Command::new(flow)
        .args(["acp", "--help"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    let stdout = child.stdout.take();
    let (sender, usage) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut text = Vec::new();
        let read = stdout.is_some_and(|stdout| {
            stdout
                .take(PROBE_OUTPUT_BYTES)
                .read_to_end(&mut text)
                .is_ok()
        });
        let _ = sender.send(read.then_some(text));
    });
    let usage = usage
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .ok()
        .flatten();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return status.success()
                    && usage
                        .is_some_and(|usage| String::from_utf8_lossy(&usage).contains("flow acp"));
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

#[cfg(not(unix))]
fn flow_agent(_path: Option<std::ffi::OsString>) -> Option<AcpAgent> {
    None
}

#[cfg(unix)]
fn flow_agent(path: Option<std::ffi::OsString>) -> Option<AcpAgent> {
    use std::os::unix::fs::PermissionsExt;
    let command = std::env::split_paths(&path?)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("flow"))
        .find(|flow| {
            flow.metadata()
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
                && answers_as_flow_agent(flow)
        })?;
    Some(AcpAgent {
        id: "flow-agent".to_owned(),
        command,
        args: vec!["acp".to_owned()],
        models: Vec::new(),
    })
}

fn pinned_t3() -> Result<PathBuf, String> {
    let t3 = match std::env::var_os("META_HARNESS_T3") {
        Some(t3) => PathBuf::from(t3),
        None => installed_t3()?,
    };
    existing(t3)
}

fn existing(t3: PathBuf) -> Result<PathBuf, String> {
    if t3.is_file() {
        Ok(t3)
    } else {
        Err(format!(
            "Could not start T3: no program at {}.",
            t3.display()
        ))
    }
}

fn installed_t3() -> Result<PathBuf, String> {
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("linux", "x86_64") => "linux-x64",
        ("windows", "x86_64") => "win32-x64",
        (os, arch) => return Err(format!("Meta-Harness has no T3 Code for {os} {arch}.")),
    };
    let prefix = prefix().map_err(|e| {
        format!(
            "{e} For a build that is not installed, set META_HARNESS_T3 to its T3 Code program."
        )
    })?;
    Ok(prefix
        .join("lib/meta-harness")
        .join(format!("t3-{}-{platform}", t3_version()))
        .join(format!("t3{}", std::env::consts::EXE_SUFFIX)))
}

fn install_download(args: &[&str]) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("On macOS and Linux, run install.sh from the download.".to_owned());
    }
    let (mut prefix, mut service) = (None, true);
    let mut rest = args;
    while let Some((flag, more)) = rest.split_first() {
        rest = match (*flag, more) {
            ("--prefix", [path, after @ ..]) => {
                prefix = Some(PathBuf::from(path));
                after
            }
            ("--no-service", after) => {
                service = false;
                after
            }
            _ => return Err(format!("meta install does not take {flag}\n{USAGE}")),
        };
    }
    let prefix = match prefix {
        Some(prefix) => prefix,
        None => std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or("LOCALAPPDATA is not set; pass --prefix.")?
            .join("Programs")
            .join("Meta-Harness"),
    };
    let bundle = std::env::current_exe()
        .map_err(|e| format!("Could not find this program: {e}"))?
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or("meta install runs from the bin folder of an unpacked download.")?;
    let task = |step: &str| {
        std::process::Command::new("schtasks")
            .args([step, "/TN", "Meta-Harness"])
            .output()
            .is_ok_and(|output| output.status.success())
    };
    let ended = task("/End");
    let installed = install::install_files(&bundle, &prefix, "win32-x64");
    if ended && (installed.is_err() || !service) {
        task("/Run");
    }
    let program = installed?;
    if service {
        let status = std::process::Command::new(&program)
            .args(["service", "install"])
            .status()
            .map_err(|e| format!("Could not run {}: {e}", program.display()))?;
        if !status.success() {
            return Err(format!(
                "the files are installed in {}, but the login service did not start; \
                 --no-service installs without it",
                prefix.display()
            ));
        }
    }
    println!(
        "Installed Meta-Harness {} in {}",
        install::bundle_info(&bundle)?.0,
        prefix.display()
    );
    println!(
        "Add {} to your PATH to run meta from any folder.",
        prefix.join("bin").display()
    );
    Ok(())
}

fn install(home: PathBuf) -> Result<(), String> {
    let program = prefix()?
        .join("bin")
        .join(format!("meta{}", std::env::consts::EXE_SUFFIX));
    let plan = login::Plan::new(program, home, |name| std::env::var(name).ok())?;
    login::Manager::current()?.install(&plan, run_step)?;
    if cfg!(target_os = "linux") && !lingering() {
        eprintln!(
            "meta: Meta-Harness stops when you log out. To keep it running, an administrator \
             runs once: sudo loginctl enable-linger \"$(id -un)\""
        );
    }
    Ok(())
}

fn lingering() -> bool {
    let user = std::env::var("USER").unwrap_or_default();
    std::process::Command::new("loginctl")
        .args(["show-user", &user, "--property=Linger"])
        .output()
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).trim() == "Linger=yes")
}

fn remove_login_service() -> Result<(), String> {
    for failure in login::Manager::current()?.uninstall(run_step)? {
        eprintln!("meta: warning: {failure}");
    }
    Ok(())
}

fn removes_data(prefix: &Path, home: &Path) -> bool {
    let resolved = |path: PathBuf| path.canonicalize().unwrap_or(path);
    home == prefix
        || ["lib/meta-harness", "share/meta-harness"]
            .iter()
            .any(|folder| home.starts_with(resolved(prefix.join(folder))))
}

fn data_in_installation(prefix: &Path, recorded: Option<PathBuf>, home: &Path) -> Option<PathBuf> {
    std::iter::once(home.to_path_buf())
        .chain(recorded)
        .find(|home| {
            let target = home.canonicalize().unwrap_or_else(|_| home.clone());
            removes_data(prefix, &target)
        })
}

fn uninstall() -> Result<(), String> {
    let prefix = prefix()?;
    let home = default_home()?;
    let manager = login::Manager::current()?;
    if let Some(data) = data_in_installation(&prefix, manager.recorded_home()?, &home) {
        return Err(format!(
            "The data folder {} lies in the installation that uninstall removes, so nothing was \
             removed. Move it outside {}, set META_HARNESS_HOME to its new place, run `meta \
             service install` and then uninstall again.",
            data.display(),
            prefix.display()
        ));
    }
    remove_login_service()?;
    remove_installation(&prefix)?;
    println!(
        "Meta-Harness is uninstalled; its data stays in {}.",
        home.display()
    );
    Ok(())
}

fn ignore_missing(result: std::io::Result<()>) -> std::io::Result<()> {
    match result {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn remove_installation(prefix: &Path) -> Result<(), String> {
    for folder in [
        prefix.join("lib/meta-harness"),
        prefix.join("share/meta-harness"),
    ] {
        ignore_missing(std::fs::remove_dir_all(&folder))
            .map_err(|e| format!("Could not remove {}: {e}", folder.display()))?;
    }
    let program = format!("meta{}", std::env::consts::EXE_SUFFIX);
    for name in [program.clone(), format!("{program}.old")] {
        let path = prefix.join("bin").join(&name);
        let removed = std::fs::remove_file(&path).or_else(|error| {
            let aside = std::env::temp_dir()
                .join(format!("meta-uninstalled-{}-{name}", std::process::id()));
            if cfg!(windows) && path.is_file() && std::fs::rename(&path, &aside).is_ok() {
                Ok(())
            } else {
                Err(error)
            }
        });
        ignore_missing(removed).map_err(|e| format!("Could not remove {}: {e}", path.display()))?;
    }
    for folder in [
        prefix.join("bin"),
        prefix.join("lib"),
        prefix.join("share"),
        prefix.to_owned(),
    ] {
        let _ = std::fs::remove_dir(folder);
    }
    Ok(())
}

fn run_step(step: &login::Step) -> Result<(), String> {
    let output = std::process::Command::new(step.program)
        .args(&step.args)
        .output()
        .map_err(|e| format!("Could not run {}: {e}", step.program))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} {} failed: {}",
            step.program,
            step.args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn an_installed_flow_joins_t3_as_the_flow_agent() {
        let dir = std::env::temp_dir().join(format!("meta-flow-agent-{}", std::process::id()));
        let executable = |dir: &Path, script: &str| {
            std::fs::create_dir_all(dir).unwrap();
            let flow = dir.join("flow");
            std::fs::write(&flow, script).unwrap();
            std::fs::set_permissions(&flow, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                .unwrap();
            flow
        };
        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::write(empty.join("flow"), "not runnable").unwrap();
        let checker = dir.join("checker");
        executable(&checker, "#!/bin/sh\necho 'Flow, a static type checker'\n");
        let hanging = dir.join("hanging");
        executable(&hanging, "#!/bin/sh\nexec sleep 60\n");
        let holding = dir.join("holding");
        executable(&holding, "#!/bin/sh\nsleep 60 &\necho '  flow acp'\n");
        let bin = dir.join("bin");
        let flow = executable(&bin, "#!/bin/sh\necho '  flow acp'\n");

        assert!(flow_agent(None).is_none());
        assert!(flow_agent(Some(empty.clone().into_os_string())).is_none());
        assert!(flow_agent(Some("bin".into())).is_none());
        let path = std::env::join_paths([&empty, &checker, &hanging, &holding, &bin]).unwrap();
        let started = std::time::Instant::now();
        let agent = flow_agent(Some(path)).expect("flow on the PATH is found");
        assert!(started.elapsed() < std::time::Duration::from_secs(15));
        assert_eq!(agent.id, "flow-agent");
        assert_eq!(agent.command, flow);
        assert_eq!(agent.args, ["acp"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn uninstall_removes_the_program_and_its_backup_but_never_a_folder_in_their_place() {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).unwrap();
        let prefix = std::env::temp_dir().join(format!(
            "mh-uninstall-{}",
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ));
        let program = format!("meta{}", std::env::consts::EXE_SUFFIX);
        std::fs::create_dir_all(prefix.join("lib/meta-harness/t3")).unwrap();
        std::fs::create_dir_all(prefix.join("share/meta-harness")).unwrap();
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        std::fs::write(prefix.join("bin").join(&program), "meta").unwrap();
        std::fs::write(prefix.join("bin").join(format!("{program}.old")), "old").unwrap();
        remove_installation(&prefix).unwrap();
        assert!(!prefix.exists());

        let kept = prefix
            .join("bin")
            .join(format!("{program}.old"))
            .join("notes.txt");
        std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
        std::fs::write(&kept, "the person's notes").unwrap();
        assert!(remove_installation(&prefix).is_err());
        assert_eq!(
            std::fs::read_to_string(&kept).unwrap(),
            "the person's notes"
        );
        let _ = std::fs::remove_dir_all(&prefix);
    }

    #[test]
    fn uninstall_refuses_a_data_folder_inside_what_it_removes() {
        let prefix = Path::new("/opt/mh");
        for (home, refused) in [
            ("/opt/mh/lib/meta-harness/data", true),
            ("/opt/mh/share/meta-harness", true),
            ("/opt/mh", true),
            ("/opt/mh/lib/meta-harness-data", false),
            ("/opt/mh/bin/meta-data", false),
            ("/home/p/.meta-harness", false),
        ] {
            assert_eq!(removes_data(prefix, Path::new(home)), refused, "{home}");
        }
    }

    #[test]
    fn uninstall_checks_the_data_folder_the_login_service_keeps_as_well() {
        let prefix = Path::new("/opt/mh");
        let recorded = PathBuf::from("/opt/mh/lib/meta-harness/data");
        let shell = Path::new("/home/p/.meta-harness");
        assert_eq!(
            data_in_installation(prefix, Some(recorded.clone()), shell),
            Some(recorded)
        );
        assert_eq!(data_in_installation(prefix, None, shell), None);
    }

    #[test]
    fn the_not_running_message_holds_for_a_terminal_serve_and_the_login_service() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let home = std::env::temp_dir().join(format!("meta-not-running-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let no_file = runtime
            .block_on(remote::Host::connect(&home, CLIENT))
            .err()
            .unwrap();
        let refused = runtime
            .block_on(remote::Host::connect_to(
                "ws://127.0.0.1:1",
                "token",
                CLIENT,
            ))
            .err()
            .unwrap();
        let _ = std::fs::remove_dir_all(&home);
        for message in [no_file, refused] {
            assert!(
                message.starts_with("Meta-Harness is not running"),
                "{message}"
            );
            assert!(!message.contains("Install"), "{message}");
            assert!(
                message.contains("\"meta serve\"")
                    && message.contains("for the login service, whose log is service.log"),
                "{message}"
            );
        }
    }

    #[test]
    fn a_missing_t3_program_is_named_and_an_uninstalled_build_is_told_to_set_the_variable() {
        let missing = existing(PathBuf::from("/no/such/t3")).unwrap_err();
        assert!(missing.contains("/no/such/t3"), "{missing}");
        let uninstalled = installed_t3().unwrap_err();
        assert!(uninstalled.contains("META_HARNESS_T3"), "{uninstalled}");
    }

    #[test]
    fn status_names_the_serve_process_and_the_t3_release_and_unreconciled_homes() {
        let status = |reconciled| api::Status {
            t3_version: "0.1.11".to_owned(),
            pid: 7,
            revision: 3,
            reconciled,
        };
        assert_eq!(
            status_text(&status(true)),
            "Meta-Harness runs as process 7 with T3 Code 0.1.11, configuration revision 3."
        );
        assert!(
            status_text(&status(false))
                .contains("The harness homes are not reconciled with revision 3")
        );
    }

    #[test]
    fn a_missing_argument_is_named_without_the_usage() {
        for (args, message) in [
            (
                &["config", "propose"][..],
                "meta config propose needs a change file",
            ),
            (
                &["config", "revert"],
                "meta config revert needs a revision number",
            ),
            (
                &["config", "approve"],
                "meta config approve needs a proposal number",
            ),
            (
                &["host", "add"],
                "meta host add needs an address and a pairing code",
            ),
            (
                &["host", "add", "ws://h:1"],
                "meta host add needs a pairing code",
            ),
            (&["host", "remove"], "meta host remove needs a host name"),
            (
                &["agent", "create"],
                "meta agent create needs an agent name",
            ),
            (
                &["agent", "routine", "add"],
                "meta agent routine add needs an agent name",
            ),
            (
                &["agent", "routine", "remove"],
                "meta agent routine remove needs an agent name and a routine ID",
            ),
            (
                &["agent", "routine", "remove", "bot"],
                "meta agent routine remove needs a routine ID",
            ),
            (&["service"], "meta service needs install or uninstall"),
        ] {
            assert_eq!(missing(args), Some(message), "{args:?}");
        }
        assert_eq!(missing(&["config", "approve", "1"]), None);
    }
}
