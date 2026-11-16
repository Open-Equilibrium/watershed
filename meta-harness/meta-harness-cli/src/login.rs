//! The login service that keeps `meta serve` running for the user, without administrator rights
//! (ADR-0286): a launch agent on macOS and a systemd user unit on Linux, written and started as
//! T3 Code's `t3 service install` does.

use std::path::{Path, PathBuf};

/// The launch agent's label and the systemd unit's name.
pub const LABEL: &str = "io.github.open-equilibrium.meta-harness";
const UNIT: &str = "meta-harness.service";

/// What the login service runs.
pub struct Plan {
    /// The installed `meta`.
    pub program: PathBuf,
    /// Meta-Harness's data folder, where the service also logs.
    pub home: PathBuf,
    /// The `PATH` the service gets, so T3 finds the harnesses the user installed; a login
    /// service does not read the user's shell profile.
    pub path: String,
}

impl Plan {
    fn log(&self) -> PathBuf {
        self.home.join("service.log")
    }
}

/// One command the installer runs, and whether its failure stops the installation.
#[derive(Debug, PartialEq, Eq)]
pub struct Step {
    pub program: &'static str,
    pub args: Vec<String>,
    pub required: bool,
}

fn step(program: &'static str, args: &[&str], required: bool) -> Step {
    Step {
        program,
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        required,
    }
}

/// A platform's service manager: where the service file goes, what it holds and the commands
/// that start and stop it.
pub struct Manager {
    /// The service file.
    pub file: PathBuf,
    render: fn(&Plan) -> String,
    start: Vec<Step>,
    stop: Vec<Step>,
}

impl Manager {
    /// The launch agent of the user `uid` whose home folder is `user_home`.
    pub fn launchd(user_home: &Path, uid: u32) -> Self {
        let file = user_home
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist"));
        let domain = format!("gui/{uid}");
        let target = format!("{domain}/{LABEL}");
        let plist = file.to_string_lossy().into_owned();
        Self {
            render: plist_of,
            // A job that is still loaded refuses bootstrap, and a persisted disable blocks it.
            start: vec![
                step("launchctl", &["bootout", &target], false),
                step("launchctl", &["enable", &target], false),
                step("launchctl", &["bootstrap", &domain, &plist], true),
            ],
            stop: vec![step("launchctl", &["bootout", &target], false)],
            file,
        }
    }

    /// The systemd user unit, under `config` (`XDG_CONFIG_HOME`, else `~/.config`).
    pub fn systemd(config: &Path) -> Self {
        Self {
            file: config.join("systemd/user").join(UNIT),
            render: unit_of,
            start: vec![
                step("systemctl", &["--user", "daemon-reload"], true),
                step("systemctl", &["--user", "enable", UNIT], true),
                step("systemctl", &["--user", "restart", UNIT], true),
            ],
            stop: vec![step(
                "systemctl",
                &["--user", "disable", "--now", UNIT],
                false,
            )],
        }
    }

    /// The manager of this platform for the current user.
    pub fn current() -> Result<Self, String> {
        let home = std::env::home_dir().ok_or("The home folder is unknown.")?;
        if cfg!(target_os = "macos") {
            Ok(Self::launchd(&home, rustix::process::getuid().as_raw()))
        } else if cfg!(target_os = "linux") {
            let config = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(".config"));
            Ok(Self::systemd(&config))
        } else {
            Err("Meta-Harness has no login service on this platform yet.".to_owned())
        }
    }

    /// Writes the service file for `plan` and starts the service, replacing one already
    /// installed. `run` runs each step and says whether it succeeded.
    pub fn install(
        &self,
        plan: &Plan,
        mut run: impl FnMut(&Step) -> Result<(), String>,
    ) -> Result<(), String> {
        std::fs::create_dir_all(&plan.home)
            .map_err(|e| format!("Could not create {}: {e}", plan.home.display()))?;
        let folder = self.file.parent().expect("a service file has a folder");
        std::fs::create_dir_all(folder)
            .map_err(|e| format!("Could not create {}: {e}", folder.display()))?;
        std::fs::write(&self.file, (self.render)(plan))
            .map_err(|e| format!("Could not write {}: {e}", self.file.display()))?;
        run_all(&self.start, &mut run)
    }

    /// Stops the service and removes its file; the data folder stays.
    pub fn uninstall(
        &self,
        mut run: impl FnMut(&Step) -> Result<(), String>,
    ) -> Result<(), String> {
        run_all(&self.stop, &mut run)?;
        match std::fs::remove_file(&self.file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("Could not remove {}: {e}", self.file.display()))
            }
            _ => Ok(()),
        }
    }
}

fn run_all(
    steps: &[Step],
    run: &mut impl FnMut(&Step) -> Result<(), String>,
) -> Result<(), String> {
    for step in steps {
        if let Err(error) = run(step)
            && step.required
        {
            return Err(error);
        }
    }
    Ok(())
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The launch agent: it starts at login and again 5 seconds after `meta serve` ends.
fn plist_of(plan: &Plan) -> String {
    let program = xml(&plan.program.to_string_lossy());
    let home = xml(&plan.home.to_string_lossy());
    let log = xml(&plan.log().to_string_lossy());
    let path = xml(&plan.path);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{program}</string>
    <string>serve</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key>
    <string>{path}</string>
    <key>META_HARNESS_HOME</key>
    <string>{home}</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>ThrottleInterval</key>
  <integer>5</integer>
  <key>ExitTimeOut</key>
  <integer>30</integer>
  <key>ProcessType</key>
  <string>Interactive</string>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
</dict>
</plist>
"#
    )
}

/// A systemd value: quoted, with `\`, `"`, `%` (specifiers) and `$` (variables) escaped.
fn systemd_value(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
        .replace('$', "$$");
    format!("\"{escaped}\"")
}

/// The systemd user unit: it starts with the user's session and 5 seconds after
/// `meta serve` ends; stopping it signals Meta-Harness first, which stops T3.
fn unit_of(plan: &Plan) -> String {
    let log = plan.log().to_string_lossy().replace('%', "%%");
    format!(
        "[Unit]\n\
         Description=Meta-Harness\n\
         StartLimitIntervalSec=300\n\
         StartLimitBurst=5\n\
         \n\
         [Service]\n\
         Type=simple\n\
         WorkingDirectory=%h\n\
         ExecStart={} serve\n\
         Environment={}\n\
         Environment={}\n\
         KillMode=mixed\n\
         Restart=always\n\
         RestartSec=5\n\
         StandardOutput=append:{log}\n\
         StandardError=append:{log}\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        systemd_value(&plan.program.to_string_lossy()),
        systemd_value(&format!("PATH={}", plan.path)),
        systemd_value(&format!(
            "META_HARNESS_HOME={}",
            plan.home.to_string_lossy()
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);

    impl Dir {
        fn new() -> Self {
            let mut bytes = [0u8; 8];
            getrandom::fill(&mut bytes).unwrap();
            let name: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            let dir = std::env::temp_dir().join(format!("mh-login-{name}"));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn plan(dir: &Path) -> Plan {
        Plan {
            program: dir.join("A & B/bin/meta"),
            home: dir.join("data 100%"),
            path: "/usr/bin:/Users/p/.local/bin".to_owned(),
        }
    }

    fn installed(manager: &Manager, plan: &Plan) -> (String, Vec<String>) {
        let mut ran = Vec::new();
        manager
            .install(plan, |step| {
                ran.push(format!("{} {}", step.program, step.args.join(" ")));
                Ok(())
            })
            .unwrap();
        (std::fs::read_to_string(&manager.file).unwrap(), ran)
    }

    #[test]
    fn a_launch_agent_keeps_meta_serve_running_with_the_user_s_path() {
        let dir = Dir::new();
        let manager = Manager::launchd(&dir.0, 501);
        let plan = plan(&dir.0);
        let (plist, ran) = installed(&manager, &plan);
        assert_eq!(
            manager.file,
            dir.0
                .join("Library/LaunchAgents/io.github.open-equilibrium.meta-harness.plist")
        );
        for part in [
            "<string>/usr/bin:/Users/p/.local/bin</string>",
            "<string>serve</string>",
            "<key>KeepAlive</key>\n  <true/>",
            "<key>RunAtLoad</key>\n  <true/>",
        ] {
            assert!(plist.contains(part), "{part}");
        }
        assert!(plist.contains(&format!("{}/A &amp; B/bin/meta", dir.0.display())));
        assert!(plist.contains("data 100%/service.log"));
        let target = "gui/501/io.github.open-equilibrium.meta-harness";
        assert_eq!(
            ran,
            vec![
                format!("launchctl bootout {target}"),
                format!("launchctl enable {target}"),
                format!("launchctl bootstrap gui/501 {}", manager.file.display()),
            ]
        );
        assert!(
            plan.home.is_dir(),
            "the log folder exists before launchd opens the log"
        );
    }

    #[test]
    fn a_systemd_user_unit_quotes_what_systemd_would_expand() {
        let dir = Dir::new();
        let manager = Manager::systemd(&dir.0.join("config"));
        let (unit, ran) = installed(&manager, &plan(&dir.0));
        let root = dir.0.display();
        assert!(unit.contains(&format!("ExecStart=\"{root}/A & B/bin/meta\" serve\n")));
        assert!(unit.contains("Environment=\"PATH=/usr/bin:/Users/p/.local/bin\"\n"));
        assert!(unit.contains(&format!(
            "Environment=\"META_HARNESS_HOME={root}/data 100%%\"\n"
        )));
        assert!(unit.contains(&format!(
            "StandardError=append:{root}/data 100%%/service.log\n"
        )));
        assert!(unit.contains("Restart=always\n"));
        assert_eq!(
            ran,
            [
                "systemctl --user daemon-reload",
                "systemctl --user enable meta-harness.service",
                "systemctl --user restart meta-harness.service",
            ]
        );
    }

    #[test]
    fn a_failed_start_fails_the_install_and_uninstall_keeps_the_data() {
        let dir = Dir::new();
        let manager = Manager::launchd(&dir.0, 501);
        let plan = plan(&dir.0);
        // Only bootstrap is required: a job that was not loaded fails bootout and enable.
        let result = manager.install(&plan, |step| match step.args[0].as_str() {
            "bootstrap" => Err("Bootstrap failed: 5: Input/output error".to_owned()),
            _ => Err("not loaded".to_owned()),
        });
        assert_eq!(
            result,
            Err("Bootstrap failed: 5: Input/output error".to_owned())
        );

        std::fs::write(plan.home.join("meta-harness.sqlite"), "").unwrap();
        manager.uninstall(|_| Err("not loaded".to_owned())).unwrap();
        assert!(!manager.file.exists());
        assert!(plan.home.join("meta-harness.sqlite").exists());
        manager.uninstall(|_| Ok(())).unwrap();
    }
}
