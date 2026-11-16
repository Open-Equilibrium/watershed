//! `meta`, Meta-Harness's command line (ADR-0286, ADR-0296): it runs the service, which the
//! login service starts, and talks to it like any client. Data commands print JSON.

mod login;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use meta_harness_core::{Change, Service, ServiceConfig, default_home, remote, t3_version};

const USAGE: &str = "\
usage: meta <command>

  help                      show this usage
  serve                     run Meta-Harness in the foreground (the login service runs this)
  status                    show whether Meta-Harness runs, its T3 Code release and revision
  config show               print the current configuration revision as JSON
  config propose FILE [SUMMARY]
                            propose the changes in FILE, a JSON array of changes
  config revert REVISION    propose going back to an earlier revision
  config pending            list the proposals that wait for a decision
  config approve NUMBER     approve a proposal; new harness sessions use the new revision
  config reject NUMBER      reject a proposal
  config log                print the append-only configuration log
  service install           start the login service, also after each login and crash
  service uninstall         stop and remove the login service
  uninstall                 remove the login service and this installation; data stays

Meta-Harness keeps its data in META_HARNESS_HOME, else ~/.meta-harness.";

/// How the configuration log names this client.
const CLIENT: &str = "meta";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
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
    let home = default_home()?;
    match args {
        ["serve"] => serve(home).await,
        ["status"] => {
            let status = connect(&home).await?.status().await?;
            println!(
                "Meta-Harness runs T3 Code {} as process {}, configuration revision {}.",
                status.t3_version, status.pid, status.revision
            );
            Ok(())
        }
        ["config", rest @ ..] => config(&connect(&home).await?, rest).await,
        ["service", "install"] => install(home),
        ["service", "uninstall"] => login::Manager::current()?.uninstall(run_step),
        ["uninstall"] => uninstall(),
        _ => Err(format!("unknown command \"{}\"\n{USAGE}", args.join(" "))),
    }
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
        ["reject", proposal] => host.reject(number(proposal)?).await,
        ["log"] => print(&host.log().await?),
        _ => Err(format!("unknown config command\n{USAGE}")),
    }
}

/// Runs the service until the login service or the person stops it.
async fn serve(home: PathBuf) -> Result<(), String> {
    let service = Service::start(ServiceConfig {
        home,
        t3: pinned_t3()?,
        acp_agents: Vec::new(),
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

/// The installation's folders: `meta` in `<prefix>/bin`, T3 in `<prefix>/lib/meta-harness`.
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

/// The pinned T3 Code the installer put beside `meta`; `META_HARNESS_T3` names another, for
/// development.
fn pinned_t3() -> Result<PathBuf, String> {
    if let Some(t3) = std::env::var_os("META_HARNESS_T3") {
        return Ok(PathBuf::from(t3));
    }
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("linux", "x86_64") => "linux-x64",
        (os, arch) => return Err(format!("Meta-Harness has no T3 Code for {os} {arch}.")),
    };
    Ok(prefix()?
        .join("lib/meta-harness")
        .join(format!("t3-{}-{platform}", t3_version()))
        .join("t3"))
}

fn install(home: PathBuf) -> Result<(), String> {
    let program = prefix()?.join("bin/meta");
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

/// Whether systemd keeps this user's services running after logout.
fn lingering() -> bool {
    let user = std::env::var("USER").unwrap_or_default();
    std::process::Command::new("loginctl")
        .args(["show-user", &user, "--property=Linger"])
        .output()
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).trim() == "Linger=yes")
}

fn uninstall() -> Result<(), String> {
    let prefix = prefix()?;
    login::Manager::current()?.uninstall(run_step)?;
    for path in [
        prefix.join("lib/meta-harness"),
        prefix.join("share/meta-harness"),
        prefix.join("bin/meta"),
    ] {
        let removed = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match removed {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!("Could not remove {}: {e}", path.display()));
            }
            _ => {}
        }
    }
    // Only an empty folder goes; one with other content, or already gone, stays.
    for folder in [
        prefix.join("bin"),
        prefix.join("lib"),
        prefix.join("share"),
        prefix,
    ] {
        let _ = std::fs::remove_dir(folder);
    }
    println!(
        "Meta-Harness is uninstalled; its data stays in {}.",
        default_home().unwrap_or_default().display()
    );
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
