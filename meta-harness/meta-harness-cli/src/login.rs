use std::path::{Path, PathBuf};

pub const LABEL: &str = "io.github.open-equilibrium.meta-harness";
const UNIT: &str = "meta-harness.service";
const TASK: &str = "Meta-Harness";

pub struct Plan {
    program: PathBuf,
    home: PathBuf,
    environment: Vec<(&'static str, String)>,
}

const PASSED: [&str; 2] = ["CODEX_HOME", "CLAUDE_CONFIG_DIR"];

impl Plan {
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
        if path.is_empty() {
            path = vec!["/usr/bin".to_owned(), "/bin".to_owned()];
        }
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

pub struct Manager {
    pub file: PathBuf,
    render: fn(&Plan) -> String,
    wide: bool,
    home_of: fn(&str) -> Option<String>,
    start: Vec<Step>,
    stop: Vec<Step>,
    restore: Vec<Step>,
    restart: Vec<Step>,
}

impl Manager {
    pub fn launchd(user_home: &Path, uid: u32) -> Self {
        let file = user_home
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist"));
        let domain = format!("gui/{uid}");
        let target = format!("{domain}/{LABEL}");
        let plist = file.to_string_lossy().into_owned();
        Self {
            render: plist_of,
            wide: false,
            home_of: plist_home,
            start: vec![
                step("launchctl", &["bootout", &target], false),
                step("launchctl", &["enable", &target], false),
                step("launchctl", &["bootstrap", &domain, &plist], true),
            ],
            stop: vec![step("launchctl", &["bootout", &target], false)],
            restore: Vec::new(),
            restart: vec![step("launchctl", &["bootstrap", &domain, &plist], true)],
            file,
        }
    }

    pub fn systemd(config: &Path) -> Self {
        Self {
            file: config.join("systemd/user").join(UNIT),
            render: unit_of,
            wide: false,
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
            restore: vec![step("systemctl", &["--user", "daemon-reload"], false)],
            restart: vec![step("systemctl", &["--user", "restart", UNIT], true)],
        }
    }

    pub fn task_scheduler(local: &Path) -> Self {
        let file = local.join("Meta-Harness").join("meta-harness-task.xml");
        let xml = file.to_string_lossy().into_owned();
        let create = step(
            "schtasks",
            &["/Create", "/TN", TASK, "/XML", &xml, "/F"],
            true,
        );
        let run = step("schtasks", &["/Run", "/TN", TASK], true);
        Self {
            render: task_of,
            wide: true,
            home_of: task_home,
            start: vec![step("schtasks", &["/End", "/TN", TASK], false), create, run],
            stop: vec![
                step("schtasks", &["/End", "/TN", TASK], false),
                step("schtasks", &["/Delete", "/TN", TASK, "/F"], false),
            ],
            restore: Vec::new(),
            restart: vec![
                step(
                    "schtasks",
                    &["/Create", "/TN", TASK, "/XML", &xml, "/F"],
                    true,
                ),
                step("schtasks", &["/Run", "/TN", TASK], true),
            ],
            file,
        }
    }

    fn encode(&self, text: &str) -> Vec<u8> {
        if self.wide {
            [0xFF, 0xFE]
                .into_iter()
                .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
                .collect()
        } else {
            text.as_bytes().to_vec()
        }
    }

    fn decode(&self, bytes: &[u8]) -> Option<String> {
        if self.wide {
            let units: Vec<u16> = bytes
                .strip_prefix(&[0xFF, 0xFE])?
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            String::from_utf16(&units).ok()
        } else {
            String::from_utf8(bytes.to_vec()).ok()
        }
    }

    pub fn current() -> Result<Self, String> {
        let home = std::env::home_dir().ok_or("The home folder is unknown.")?;
        if cfg!(target_os = "macos") {
            #[cfg(unix)]
            return Ok(Self::launchd(&home, rustix::process::getuid().as_raw()));
            #[cfg(not(unix))]
            return Err("Meta-Harness has no login service on this platform yet.".to_owned());
        } else if cfg!(windows) {
            let local = std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .ok_or("LOCALAPPDATA is not set, so the login task has no place.")?;
            Ok(Self::task_scheduler(&local))
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
        let had_previous = previous.is_some();
        replace_file(&self.file, &self.encode(&(self.render)(plan)))
            .map_err(|e| format!("Could not write {}: {e}", self.file.display()))?;
        let started = run_all(&self.start, &mut run).map(|_| ());
        if let Err(error) = started {
            let restored = match previous {
                Some(bytes) => replace_file(&self.file, &bytes),
                None => match std::fs::remove_file(&self.file) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                    _ => Ok(()),
                },
            };
            return Err(match restored {
                Ok(()) => {
                    let mut message = error;
                    for failure in run_all(&self.restore, &mut run).unwrap_or_else(|e| vec![e]) {
                        message = format!("{message}. {failure}");
                    }
                    if had_previous && let Err(failure) = run_all(&self.restart, &mut run) {
                        message = format!(
                            "{message}. The previous service did not start again: {failure}"
                        );
                    }
                    message
                }
                Err(e) => format!(
                    "{error}. The previous service file could not be restored: {e} ({})",
                    self.file.display()
                ),
            });
        }
        Ok(())
    }

    pub fn recorded_home(&self) -> Result<Option<PathBuf>, String> {
        match std::fs::read(&self.file) {
            Ok(bytes) => match self.decode(&bytes).and_then(|text| (self.home_of)(&text)) {
                Some(home) => Ok(Some(PathBuf::from(home))),
                None => Err(format!(
                    "The login service file {} does not name its data folder, so nothing was removed.",
                    self.file.display()
                )),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("Could not read {}: {e}", self.file.display())),
        }
    }

    pub fn uninstall(
        &self,
        mut run: impl FnMut(&Step) -> Result<(), String>,
    ) -> Result<Vec<String>, String> {
        let failures = run_all(&self.stop, &mut run)?;
        if let Some(failure) = failures.iter().find(|failure| !not_loaded(failure)) {
            return Err(format!(
                "The login service could not be stopped ({failure}), so nothing was removed. Stop it, then run uninstall again."
            ));
        }
        match std::fs::remove_file(&self.file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("Could not remove {}: {e}", self.file.display()))
            }
            _ => Ok(failures),
        }
    }
}

fn not_loaded(failure: &str) -> bool {
    [
        "not loaded",
        "No such process",
        "Could not find service",
        "Could not find specified service",
        "cannot find the file specified",
        "is not running",
    ]
    .iter()
    .any(|phrase| failure.contains(phrase))
}

fn replace_file(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let name = file.file_name().unwrap_or_default().to_string_lossy();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = file.with_file_name(format!(".{name}.{}.{nanos}.tmp", std::process::id()));
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        });
    let result = written
        .and_then(|()| std::fs::rename(&temporary, file))
        .and_then(|()| sync_directory(file));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(unix)]
fn sync_directory(file: &Path) -> std::io::Result<()> {
    let dir = match file.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    std::fs::File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_file: &Path) -> std::io::Result<()> {
    Ok(())
}

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

fn task_user() -> String {
    let user = std::env::var("USERNAME").unwrap_or_default();
    match std::env::var("USERDOMAIN") {
        Ok(domain) if !domain.is_empty() => format!("{domain}\\{user}"),
        _ => user,
    }
}

fn task_of(plan: &Plan) -> String {
    let program = xml(&plan.program.to_string_lossy());
    let home = xml(&plan.home.to_string_lossy());
    let log = xml(&plan.log().to_string_lossy());
    let user = xml(&task_user());
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Meta-Harness, started at logon</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>999</Count>
    </RestartOnFailure>
    <Hidden>true</Hidden>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>conhost.exe</Command>
      <Arguments>--headless "{program}" serve --home "{home}" --log "{log}"</Arguments>
    </Exec>
  </Actions>
</Task>
"#
    )
}

fn task_home(text: &str) -> Option<String> {
    let value = text.split_once("--home \"")?.1.split_once('"')?.0;
    Some(
        value
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&"),
    )
}

fn systemd_value(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    format!("\"{escaped}\"")
}

fn plist_home(text: &str) -> Option<String> {
    let value = text
        .split_once("<key>META_HARNESS_HOME</key>")?
        .1
        .trim_start()
        .strip_prefix("<string>")?
        .split_once("</string>")?
        .0;
    Some(
        value
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&"),
    )
}

fn unit_home(text: &str) -> Option<String> {
    let quoted = text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("Environment")?
            .trim_start()
            .strip_prefix('=')?
            .trim_start()
            .strip_prefix('"')?
            .strip_suffix('"')?
            .strip_prefix("META_HARNESS_HOME=")
    })?;
    let mut value = String::new();
    let mut chars = quoted.chars();
    while let Some(c) = chars.next() {
        if c == '\\' || c == '%' {
            value.push(chars.next()?);
        } else {
            value.push(c);
        }
    }
    Some(value)
}

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
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let not_utf8 = PathBuf::from(std::ffi::OsStr::from_bytes(b"/data/\xff"));
            assert!(Plan::new(dir.0.join("meta"), not_utf8, |_| None).is_err());
        }
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
        assert_eq!(manager.recorded_home(), Ok(Some(old.home)));
    }

    #[test]
    fn a_failed_restart_reloads_the_restored_unit() {
        let dir = Dir::new();
        let manager = Manager::systemd(&dir.0.join("config"));
        installed(&manager, &plan(&dir.0));
        let outside = Plan::new(dir.0.join("bin/meta"), dir.0.join("outside"), |_| None).unwrap();
        let mut ran = Vec::new();
        let result = manager.install(&outside, |step| {
            ran.push(format!("{} {}", step.program, step.args.join(" ")));
            if step.args.iter().any(|arg| arg == "restart") {
                Err("restart failed".to_owned())
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        assert_eq!(
            ran.last_chunk::<2>(),
            Some(&[
                "systemctl --user daemon-reload".to_owned(),
                "systemctl --user restart meta-harness.service".to_owned(),
            ]),
            "{ran:?}"
        );
    }

    #[test]
    fn a_failed_start_fails_the_install_and_uninstall_keeps_the_data() {
        let dir = Dir::new();
        let manager = Manager::launchd(&dir.0, 501);
        let plan = plan(&dir.0);
        let result = manager.install(&plan, |step| match step.args[0].as_str() {
            "bootstrap" => Err("Bootstrap failed: 5: Input/output error".to_owned()),
            _ => Err("not loaded".to_owned()),
        });
        assert_eq!(
            result,
            Err("Bootstrap failed: 5: Input/output error".to_owned())
        );

        std::fs::write(plan.home.join("meta-harness.sqlite"), "").unwrap();
        let warnings = manager.uninstall(|_| Err("not loaded".to_owned()));
        assert_eq!(warnings, Ok(vec!["not loaded".to_owned()]));
        assert!(!manager.file.exists());
        assert!(plan.home.join("meta-harness.sqlite").exists());
        assert_eq!(manager.uninstall(|_| Ok(())), Ok(Vec::new()));
    }

    #[test]
    fn a_service_file_whose_home_cannot_be_read_refuses_to_be_uninstalled() {
        let dir = Dir::new();
        let manager = Manager::systemd(&dir.0.join("config"));
        std::fs::create_dir_all(manager.file.parent().unwrap()).unwrap();
        std::fs::write(&manager.file, "[Service]\nExecStart=/bin/meta serve\n").unwrap();
        assert!(manager.recorded_home().is_err());
    }

    #[test]
    fn a_windows_logon_task_runs_meta_serve_hidden_and_logs_to_the_data_folder() {
        let dir = Dir::new();
        let manager = Manager::task_scheduler(&dir.0);
        let plan = plan(&dir.0);
        let mut ran = Vec::new();
        manager
            .install(&plan, |step| {
                ran.push(format!("{} {}", step.program, step.args.join(" ")));
                Ok(())
            })
            .unwrap();
        let bytes = std::fs::read(&manager.file).unwrap();
        assert_eq!(&bytes[..2], &[0xFF, 0xFE], "Task Scheduler reads UTF-16");
        let task = manager.decode(&bytes).unwrap();
        for part in [
            "<LogonTrigger>",
            "<LogonType>InteractiveToken</LogonType>",
            "<RunLevel>LeastPrivilege</RunLevel>",
            "<RestartOnFailure>",
            "<Command>conhost.exe</Command>",
        ] {
            assert!(task.contains(part), "{part}");
        }
        assert!(task.contains(&format!(
            "--headless \"{}/A &amp; B/bin/meta\" serve --home \"{}/data 100%\"",
            dir.0.display(),
            dir.0.display()
        )));
        assert!(task.contains("data 100%/service.log"));
        let xml = manager.file.to_string_lossy();
        assert_eq!(
            ran,
            vec![
                "schtasks /End /TN Meta-Harness".to_owned(),
                format!("schtasks /Create /TN Meta-Harness /XML {xml} /F"),
                "schtasks /Run /TN Meta-Harness".to_owned(),
            ]
        );
        assert_eq!(manager.recorded_home(), Ok(Some(plan.home.clone())));
        assert!(not_loaded(
            "ERROR: The system cannot find the file specified."
        ));
    }

    #[test]
    fn a_service_file_names_its_home_whatever_the_layout() {
        let dir = Dir::new();
        let launchd = Manager::launchd(&dir.0, 501);
        std::fs::create_dir_all(launchd.file.parent().unwrap()).unwrap();
        std::fs::write(
            &launchd.file,
            "<dict><key>META_HARNESS_HOME</key><string>/data &amp; 100%</string></dict>",
        )
        .unwrap();
        assert_eq!(
            launchd.recorded_home(),
            Ok(Some(PathBuf::from("/data & 100%")))
        );
        let systemd = Manager::systemd(&dir.0.join("config"));
        std::fs::create_dir_all(systemd.file.parent().unwrap()).unwrap();
        std::fs::write(
            &systemd.file,
            "[Service]\n  Environment = \"META_HARNESS_HOME=/data 100%%\"\n",
        )
        .unwrap();
        assert_eq!(
            systemd.recorded_home(),
            Ok(Some(PathBuf::from("/data 100%")))
        );
    }

    #[test]
    fn launchd_and_systemd_saying_the_service_is_absent_count_as_not_loaded() {
        for failure in [
            "Boot-out failed: 3: No such process",
            "Boot-out failed: 113: Could not find specified service",
            "Could not find service \"x\" in domain for user gui: 502",
            "Unit x.service not loaded.",
        ] {
            assert!(not_loaded(failure), "{failure}");
        }
        assert!(!not_loaded("Boot-out failed: 1: Operation not permitted"));
        assert!(!not_loaded(
            "Failed to disable unit: Unit file x.service does not exist."
        ));
    }

    #[test]
    fn a_stop_that_fails_for_another_reason_keeps_the_service_file() {
        let dir = Dir::new();
        let manager = Manager::launchd(&dir.0, 501);
        installed(&manager, &plan(&dir.0));
        let result = manager.uninstall(|_| Err("Operation not permitted".to_owned()));
        assert!(result.is_err(), "{result:?}");
        assert!(manager.file.exists());
    }

    #[test]
    fn a_failed_replacement_restarts_the_previous_unit_and_reports_a_failed_restart() {
        let dir = Dir::new();
        let manager = Manager::systemd(&dir.0.join("config"));
        installed(&manager, &plan(&dir.0));
        let outside = Plan::new(dir.0.join("bin/meta"), dir.0.join("outside"), |_| None).unwrap();
        let mut ran = Vec::new();
        let mut restarts = 0;
        let result = manager.install(&outside, |step| {
            ran.push(format!("{} {}", step.program, step.args.join(" ")));
            if step.args.iter().any(|arg| arg == "restart") {
                restarts += 1;
                return Err(if restarts == 1 {
                    "start failed".to_owned()
                } else {
                    "previous restart failed".to_owned()
                });
            }
            Ok(())
        });
        let message = result.unwrap_err();
        assert!(message.contains("previous restart failed"), "{message}");
        assert_eq!(
            ran.last().map(String::as_str),
            Some("systemctl --user restart meta-harness.service"),
            "{ran:?}"
        );
    }

    #[test]
    fn a_failed_replacement_bootstraps_the_previous_agent_again() {
        let dir = Dir::new();
        let manager = Manager::launchd(&dir.0, 501);
        installed(&manager, &plan(&dir.0));
        let outside = Plan::new(dir.0.join("bin/meta"), dir.0.join("outside"), |_| None).unwrap();
        let mut bootstraps = Vec::new();
        let result = manager.install(&outside, |step| {
            if step.args[0] == "bootstrap" {
                bootstraps.push(step.args.join(" "));
                return Err(if bootstraps.len() == 1 {
                    "first bootstrap failed".to_owned()
                } else {
                    "previous bootstrap failed".to_owned()
                });
            }
            Ok(())
        });
        let message = result.unwrap_err();
        assert!(message.contains("previous bootstrap failed"), "{message}");
        assert_eq!(bootstraps.len(), 2, "{bootstraps:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_reinstall_replaces_the_service_file_whole_for_a_reader_that_has_it_open() {
        use std::io::Read;
        let dir = Dir::new();
        let manager = Manager::systemd(&dir.0.join("config"));
        installed(&manager, &plan(&dir.0));
        let previous = std::fs::read_to_string(&manager.file).unwrap();
        let mut reader = std::fs::File::open(&manager.file).unwrap();
        let other = Plan::new(dir.0.join("bin/meta"), dir.0.join("other"), |_| None).unwrap();
        manager.install(&other, |_| Ok(())).unwrap();
        let mut seen = String::new();
        reader.read_to_string(&mut seen).unwrap();
        assert_eq!(seen, previous);
        assert_ne!(std::fs::read_to_string(&manager.file).unwrap(), previous);
    }
}
