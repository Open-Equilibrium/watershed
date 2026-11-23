use std::path::{Path, PathBuf};

use crate::ignore_missing;

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
            restore: vec![step("schtasks", &["/Delete", "/TN", TASK, "/F"], false)],
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
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair))
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
        let previous = self.read_service()?;
        replace_file(&self.file, &self.encode(&(self.render)(plan)))
            .map_err(|e| format!("Could not write {}: {e}", self.file.display()))?;
        let Err(error) = run_all(&self.start, &mut run) else {
            return Ok(());
        };
        let restored = match &previous {
            Some(bytes) => replace_file(&self.file, bytes),
            None => ignore_missing(std::fs::remove_file(&self.file)),
        };
        Err(match restored {
            Ok(()) => {
                let mut message = error;
                for failure in run_all(&self.restore, &mut run)
                    .unwrap_or_else(|e| vec![e])
                    .into_iter()
                    .filter(|failure| !not_loaded(failure))
                {
                    message = format!("{message}. {failure}");
                }
                if previous.is_some()
                    && let Err(failure) = run_all(&self.restart, &mut run)
                {
                    message =
                        format!("{message}. The previous service did not start again: {failure}");
                }
                message
            }
            Err(e) => format!(
                "{error}. The previous service file could not be restored: {e} ({})",
                self.file.display()
            ),
        })
    }

    fn read_service(&self) -> Result<Option<Vec<u8>>, String> {
        match std::fs::read(&self.file) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("Could not read {}: {e}", self.file.display())),
        }
    }

    pub fn recorded_home(&self) -> Result<Option<PathBuf>, String> {
        let Some(bytes) = self.read_service()? else {
            return Ok(None);
        };
        match self.decode(&bytes).and_then(|text| (self.home_of)(&text)) {
            Some(home) => Ok(Some(PathBuf::from(home))),
            None => Err(format!(
                "The login service file {} does not name its data folder, so nothing was removed.",
                self.file.display()
            )),
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
        ignore_missing(std::fs::remove_file(&self.file))
            .map_err(|e| format!("Could not remove {}: {e}", self.file.display()))?;
        Ok(failures)
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
    let dir = file
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
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
    let program = xml(&argument(&plan.program.to_string_lossy()));
    let home = xml(&argument(&plan.home.to_string_lossy()));
    let log = xml(&argument(&plan.log().to_string_lossy()));
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
      <Arguments>--headless {program} serve --home {home} --log {log}</Arguments>
    </Exec>
  </Actions>
</Task>
"#
    )
}

fn argument(value: &str) -> String {
    let trailing = value.len() - value.trim_end_matches('\\').len();
    format!("\"{value}{}\"", "\\".repeat(trailing))
}

fn unxml(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn task_home(text: &str) -> Option<String> {
    let value = text.split_once("--home \"")?.1.split_once('"')?.0;
    let trailing = value.len() - value.trim_end_matches('\\').len();
    Some(unxml(&value[..value.len() - trailing / 2]))
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
    Some(unxml(value))
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
mod tests;
