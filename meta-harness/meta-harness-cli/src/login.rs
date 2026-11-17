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
    program: PathBuf,
    /// Meta-Harness's data folder, where the service also logs.
    home: PathBuf,
    /// The variables the service gets. A login service reads no shell profile, so they come from
    /// the installing shell.
    environment: Vec<(&'static str, String)>,
}

/// Variables the service takes from the installing shell when set: where the person keeps the
/// harnesses' own logins, which the harness homes link to (SECURITY.md).
const PASSED: [&str; 2] = ["CODEX_HOME", "CLAUDE_CONFIG_DIR"];

impl Plan {
    /// The plan for `program` and `home`, with variables from `var`. `PATH` keeps only absolute
    /// folders, so the service never runs a program from whatever folder it works in.
    pub fn new(
        program: PathBuf,
        home: PathBuf,
        var: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        let mut path: Vec<String> = var("PATH")
            .unwrap_or_default()
            .split(':')
            .filter(|folder| folder.starts_with('/'))
            .map(str::to_owned)
            .collect();
        // An empty PATH makes some programs search the working folder, so the system folders stand in.
        if path.is_empty() {
            path = vec!["/usr/bin".to_owned(), "/bin".to_owned()];
        }
        // Service files hold text, so a path that is not UTF-8 would be written as another path.
        let utf8 = |p: &Path| {
            p.to_str().map(str::to_owned).ok_or_else(|| {
                format!(
                    "{} is not UTF-8, which a login service cannot carry.",
                    p.display()
                )
            })
        };
        let mut environment = vec![
            ("PATH", path.join(":")),
            ("META_HARNESS_HOME", utf8(&home)?),
        ];
        for name in PASSED {
            if let Some(value) = var(name).filter(|v| v.starts_with('/')) {
                environment.push((name, value));
            }
        }
        // A service file is line-based; a control character would end or break an entry.
        let program_text = utf8(&program)?;
        for value in std::iter::once(program_text.as_str())
            .chain(environment.iter().map(|(_, v)| v.as_str()))
        {
            if value.chars().any(char::is_control) {
                return Err(format!(
                    "{value:?} holds a control character, which a login service cannot carry."
                ));
            }
        }
        Ok(Self {
            program,
            home,
            environment,
        })
    }

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
    /// The data folder a service file's text names.
    home_of: fn(&str) -> Option<String>,
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
            home_of: plist_home,
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
            home_of: unit_home,
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
        let previous = match std::fs::read(&self.file) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("Could not read {}: {e}", self.file.display())),
        };
        std::fs::write(&self.file, (self.render)(plan))
            .map_err(|e| format!("Could not write {}: {e}", self.file.display()))?;
        let started = run_all(&self.start, &mut run).map(|_| ());
        if let Err(error) = started {
            // The file keeps naming the data folder the service ran with, which uninstall checks.
            let restored = match previous {
                Some(bytes) => std::fs::write(&self.file, bytes),
                None => match std::fs::remove_file(&self.file) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                    _ => Ok(()),
                },
            };
            return Err(match restored {
                Ok(()) => error,
                Err(e) => format!(
                    "{error}. The previous service file could not be restored: {e} ({})",
                    self.file.display()
                ),
            });
        }
        Ok(())
    }

    /// The data folder the installed service keeps, from its file; `None` without a file.
    pub fn recorded_home(&self) -> Result<Option<PathBuf>, String> {
        match std::fs::read_to_string(&self.file) {
            Ok(text) => Ok((self.home_of)(&text).map(PathBuf::from)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("Could not read {}: {e}", self.file.display())),
        }
    }

    /// Stops the service and removes its file; the data folder stays. Returns the failures of
    /// the stop steps, which do not stop the removal.
    pub fn uninstall(
        &self,
        mut run: impl FnMut(&Step) -> Result<(), String>,
    ) -> Result<Vec<String>, String> {
        let failures = run_all(&self.stop, &mut run)?;
        match std::fs::remove_file(&self.file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("Could not remove {}: {e}", self.file.display()))
            }
            _ => Ok(failures),
        }
    }
}

/// Runs `steps` in order. A failed step that is not required is returned, not acted on.
fn run_all(
    steps: &[Step],
    run: &mut impl FnMut(&Step) -> Result<(), String>,
) -> Result<Vec<String>, String> {
    let mut failures = Vec::new();
    for step in steps {
        match run(step) {
            Ok(()) => {}
            Err(error) if step.required => return Err(error),
            Err(error) => failures.push(error),
        }
    }
    Ok(failures)
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
    let log = xml(&plan.log().to_string_lossy());
    let environment: String = plan
        .environment
        .iter()
        .map(|(name, value)| {
            format!(
                "    <key>{name}</key>\n    <string>{}</string>\n",
                xml(value)
            )
        })
        .collect();
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
{environment}  </dict>
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

/// A systemd value: quoted, with `\`, `"` and `%` (specifiers) escaped.
fn systemd_value(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    format!("\"{escaped}\"")
}

/// The data folder a launch agent names, read back from its environment.
fn plist_home(text: &str) -> Option<String> {
    let rest = text
        .split_once("<key>META_HARNESS_HOME</key>\n    <string>")?
        .1;
    let value = rest.split_once("</string>")?.0;
    Some(
        value
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&"),
    )
}

/// The data folder a systemd unit names, read back from its `Environment=` line, which
/// `systemd_value` quoted and escaped.
fn unit_home(text: &str) -> Option<String> {
    let quoted = text
        .lines()
        .find_map(|line| line.strip_prefix("Environment=\"META_HARNESS_HOME="))?
        .strip_suffix('"')?;
    let mut value = String::new();
    let mut chars = quoted.chars();
    while let Some(c) = chars.next() {
        // A backslash or a percent sign escapes the character after it.
        if c == '\\' || c == '%' {
            value.push(chars.next()?);
        } else {
            value.push(c);
        }
    }
    Some(value)
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
         {}\
         KillMode=mixed\n\
         Restart=always\n\
         RestartSec=5\n\
         StandardOutput=append:{log}\n\
         StandardError=append:{log}\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        // ExecStart= expands `$` variables, which Environment= does not.
        systemd_value(&plan.program.to_string_lossy().replace('$', "$$")),
        plan.environment
            .iter()
            .map(|(name, value)| format!(
                "Environment={}\n",
                systemd_value(&format!("{name}={value}"))
            ))
            .collect::<String>(),
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
        Plan::new(dir.join("A & B/bin/meta"), dir.join("data 100%"), |name| {
            (name == "PATH").then(|| "/usr/bin:/Users/p/.local/bin".to_owned())
        })
        .unwrap()
    }

    #[test]
    fn the_service_gets_absolute_folders_and_the_person_s_harness_logins() {
        let dir = Dir::new();
        let variables = |name: &str| match name {
            "PATH" => Some("/usr/bin:.::bin:/opt/homebrew/bin".to_owned()),
            "CODEX_HOME" => Some("/Users/p/.codex-work".to_owned()),
            "CLAUDE_CONFIG_DIR" => Some("relative".to_owned()),
            _ => None,
        };
        let plan = Plan::new(dir.0.join("meta"), dir.0.join("data"), variables).unwrap();
        let names: Vec<_> = plan
            .environment
            .iter()
            .map(|(n, v)| format!("{n}={v}"))
            .collect();
        assert_eq!(
            names,
            [
                "PATH=/usr/bin:/opt/homebrew/bin".to_owned(),
                format!("META_HARNESS_HOME={}", dir.0.join("data").display()),
                "CODEX_HOME=/Users/p/.codex-work".to_owned(),
            ]
        );
        let broken = Plan::new(dir.0.join("meta"), dir.0.join("da\nta"), |_| None);
        assert!(broken.is_err_and(|e| e.contains("control character")));
        // A path that is not UTF-8 would be written as another path, so it is refused.
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let not_utf8 = PathBuf::from(std::ffi::OsStr::from_bytes(b"/data/\xff"));
            assert!(Plan::new(dir.0.join("meta"), not_utf8, |_| None).is_err());
        }
        // Without an absolute folder, PATH names the system folders, never none.
        let relative = Plan::new(dir.0.join("meta"), dir.0.join("data"), |name| {
            (name == "PATH").then(|| ".:bin".to_owned())
        })
        .unwrap();
        assert_eq!(
            relative.environment[0],
            ("PATH", "/usr/bin:/bin".to_owned())
        );
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
        let plan = plan(&dir.0);
        let (unit, ran) = installed(&manager, &plan);
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
        assert_eq!(manager.recorded_home(), Ok(Some(plan.home)));
    }

    #[test]
    fn a_systemd_unit_expands_dollar_signs_only_in_exec_start() {
        let dir = Dir::new();
        let manager = Manager::systemd(&dir.0.join("config"));
        let plan = Plan::new(dir.0.join("$bin/meta"), dir.0.join("data$"), |name| {
            (name == "PATH").then(|| "/usr/bin".to_owned())
        })
        .unwrap();
        let (unit, _) = installed(&manager, &plan);
        let root = dir.0.display();
        // systemd expands `$` in ExecStart= only; a `$` in Environment= is literal.
        assert!(unit.contains(&format!("ExecStart=\"{root}/$$bin/meta\" serve\n")));
        assert!(unit.contains(&format!("Environment=\"META_HARNESS_HOME={root}/data$\"\n")));
    }

    #[test]
    fn a_service_file_names_the_data_folder_it_was_written_with() {
        let home = PathBuf::from("/data $ \"q\" \\ 100% & <x>");
        let plan = Plan::new(PathBuf::from("/bin/meta"), home.clone(), |_| None).unwrap();
        let home = home.to_string_lossy().into_owned();
        assert_eq!(plist_home(&plist_of(&plan)), Some(home.clone()));
        assert_eq!(unit_home(&unit_of(&plan)), Some(home));
    }

    #[test]
    fn a_replacement_that_fails_keeps_the_data_folder_of_the_service_on_record() {
        let dir = Dir::new();
        let manager = Manager::systemd(&dir.0.join("config"));
        let old = Plan::new(dir.0.join("bin/meta"), dir.0.join("lib/data"), |_| None).unwrap();
        installed(&manager, &old);
        let outside = Plan::new(dir.0.join("bin/meta"), dir.0.join("outside"), |_| None).unwrap();
        let result = manager.install(&outside, |_| Err("daemon-reload failed".to_owned()));
        assert!(result.is_err());
        // Uninstall checks the folder this file names, so the old one must still be named.
        assert_eq!(manager.recorded_home(), Ok(Some(old.home)));
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
        // A failed stop is reported, and the file goes anyway.
        let warnings = manager.uninstall(|_| Err("not loaded".to_owned()));
        assert_eq!(warnings, Ok(vec!["not loaded".to_owned()]));
        assert!(!manager.file.exists());
        assert!(plan.home.join("meta-harness.sqlite").exists());
        assert_eq!(manager.uninstall(|_| Ok(())), Ok(Vec::new()));
    }
}
