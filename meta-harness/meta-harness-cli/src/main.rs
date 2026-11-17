//! `meta`, Meta-Harness's command line (ADR-0286, ADR-0296): it runs the service, which the
//! login service starts, and talks to it like any client. Data commands print JSON.

mod hosts;
mod login;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use meta_harness_core::{Change, Service, ServiceConfig, api, default_home, remote, t3_version};

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
            println!("{}", status_text(&status));
            Ok(())
        }
        ["config", rest @ ..] => config(&connect(&home).await?, rest).await,
        ["remote", rest @ ..] => hosts::remote(&connect(&home).await?, rest).await,
        ["invite", rest @ ..] => hosts::invite(&connect(&home).await?, rest).await,
        ["host", rest @ ..] => hosts::host(&connect(&home).await?, rest).await,
        ["service", "install"] => install(home),
        ["service", "uninstall"] => remove_login_service(),
        ["uninstall"] => uninstall(),
        _ => Err(format!("unknown command \"{}\"\n{USAGE}", args.join(" "))),
    }
}

/// What `meta status` says of the service and its harness homes.
fn status_text(status: &api::Status) -> String {
    let mut text = format!(
        "Meta-Harness runs T3 Code {} as process {}, configuration revision {}.",
        status.t3_version, status.pid, status.revision
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

/// Stops and removes the login service; a stop step that failed is printed as a warning.
fn remove_login_service() -> Result<(), String> {
    for failure in login::Manager::current()?.uninstall(run_step)? {
        eprintln!("meta: warning: {failure}");
    }
    Ok(())
}

/// Whether uninstalling `prefix` removes the data folder `home`, both resolved: `home` lies in
/// a folder uninstall removes, or is the prefix itself.
fn removes_data(prefix: &Path, home: &Path) -> bool {
    // A folder reached through a link is removed where the link points.
    let resolved = |path: PathBuf| path.canonicalize().unwrap_or(path);
    home == prefix
        || ["lib/meta-harness", "share/meta-harness"]
            .iter()
            .any(|folder| home.starts_with(resolved(prefix.join(folder))))
}

/// The first of `home` and `recorded`, the data folder the login service keeps, that uninstalling
/// `prefix` would remove.
fn data_in_installation(prefix: &Path, recorded: Option<PathBuf>, home: &Path) -> Option<PathBuf> {
    std::iter::once(home.to_path_buf())
        .chain(recorded)
        .find(|home| {
            // A link to the data folder counts as its target.
            let target = home.canonicalize().unwrap_or_else(|_| home.clone());
            removes_data(prefix, &target)
        })
}

fn uninstall() -> Result<(), String> {
    let prefix = prefix()?;
    let home = default_home()?;
    let manager = login::Manager::current()?;
    // Checked before anything is removed: the login service may keep a data folder that this
    // shell does not name.
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
        home.display()
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn status_says_when_the_harness_homes_are_not_reconciled() {
        let status = |reconciled| api::Status {
            t3_version: "0.1.9".to_owned(),
            pid: 7,
            revision: 3,
            reconciled,
        };
        assert!(!status_text(&status(true)).contains("not reconciled"));
        assert!(
            status_text(&status(false))
                .contains("The harness homes are not reconciled with revision 3")
        );
    }

    #[test]
    fn uninstall_checks_the_data_folder_the_login_service_keeps_as_well() {
        let prefix = Path::new("/opt/mh");
        let recorded = PathBuf::from("/opt/mh/lib/meta-harness/data");
        let shell = Path::new("/home/p/.meta-harness");
        // This shell names a folder outside the installation; the service keeps its data inside.
        assert_eq!(
            data_in_installation(prefix, Some(recorded.clone()), shell),
            Some(recorded)
        );
        assert_eq!(data_in_installation(prefix, None, shell), None);
    }

    #[test]
    fn the_usage_names_the_access_flags_and_what_revoke_ends() {
        assert!(
            USAGE.contains("invite --device | --host [--access LEVEL]"),
            "{USAGE}"
        );
        assert!(
            USAGE.contains("end the T3 sessions of that host's link"),
            "{USAGE}"
        );
    }
}
