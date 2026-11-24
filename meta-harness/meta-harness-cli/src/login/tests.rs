use super::*;
use crate::testing::{Dir, put};

fn plan(dir: &Path) -> Plan {
    Plan::new(dir.join("A & B/bin/meta"), dir.join("data 100%"), |name| {
        (name == "PATH").then(|| "/usr/bin:/Users/p/.local/bin".to_owned())
    })
    .unwrap()
}

fn elsewhere(dir: &Path) -> Plan {
    Plan::new(dir.join("bin/meta"), dir.join("outside"), |_| None).unwrap()
}

fn attempt(
    manager: &Manager,
    plan: &Plan,
    mut run: impl FnMut(&Step) -> Result<(), String>,
) -> (Result<(), String>, Vec<String>) {
    let mut ran = Vec::new();
    let result = manager.install(plan, |step| {
        ran.push(format!("{} {}", step.program, step.args.join(" ")));
        run(step)
    });
    (result, ran)
}

fn installed(manager: &Manager, plan: &Plan) -> (String, Vec<String>) {
    let (result, ran) = attempt(manager, plan, |_| Ok(()));
    result.unwrap();
    (std::fs::read_to_string(&manager.file).unwrap(), ran)
}

#[test]
fn the_service_gets_absolute_folders_and_the_person_s_harness_logins() {
    let dir = Dir::new();
    let codex = dir.join("codex-work");
    let variables = |name: &str| match name {
        "PATH" => Some("/usr/bin:.::bin:/opt/homebrew/bin".to_owned()),
        "CODEX_HOME" => Some(codex.to_string_lossy().into_owned()),
        "CLAUDE_CONFIG_DIR" => Some("relative".to_owned()),
        _ => None,
    };
    let plan = Plan::new(dir.join("meta"), dir.join("data"), variables).unwrap();
    let names: Vec<_> = plan
        .environment
        .iter()
        .map(|(n, v)| format!("{n}={v}"))
        .collect();
    assert_eq!(
        names,
        [
            "PATH=/usr/bin:/opt/homebrew/bin".to_owned(),
            format!("META_HARNESS_HOME={}", dir.join("data").display()),
            format!("CODEX_HOME={}", codex.display()),
        ]
    );
    let broken = Plan::new(dir.join("meta"), dir.join("da\nta"), |_| None);
    assert!(broken.is_err_and(|e| e.contains("control character")));
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let not_utf8 = PathBuf::from(std::ffi::OsStr::from_bytes(b"/data/\xff"));
        assert!(Plan::new(dir.join("meta"), not_utf8, |_| None).is_err());
    }
    let relative = Plan::new(dir.join("meta"), dir.join("data"), |name| {
        (name == "PATH").then(|| ".:bin".to_owned())
    })
    .unwrap();
    assert_eq!(
        relative.environment[0],
        ("PATH", "/usr/bin:/bin".to_owned())
    );
}

#[cfg(unix)]
#[test]
fn a_launch_agent_keeps_meta_serve_running_with_the_user_s_path() {
    let dir = Dir::new();
    let manager = Manager::launchd(&dir, 501);
    let plan = plan(&dir);
    let (plist, ran) = installed(&manager, &plan);
    assert_eq!(
        manager.file,
        dir.join("Library/LaunchAgents/io.github.open-equilibrium.meta-harness.plist")
    );
    for part in [
        "<string>/usr/bin:/Users/p/.local/bin</string>",
        "<string>serve</string>",
        "<key>KeepAlive</key>\n  <true/>",
        "<key>RunAtLoad</key>\n  <true/>",
    ] {
        assert!(plist.contains(part), "{part}");
    }
    assert!(plist.contains(&format!("{}/A &amp; B/bin/meta", dir.display())));
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

#[cfg(unix)]
#[test]
fn a_systemd_unit_quotes_and_escapes_what_systemd_would_expand() {
    let dir = Dir::new();
    let manager = Manager::systemd(&dir.join("config"));
    let plan = plan(&dir);
    let (unit, ran) = installed(&manager, &plan);
    let root = dir.display();
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

    let plan = Plan::new(dir.join("$bin/meta"), dir.join("data$"), |name| {
        (name == "PATH").then(|| "/usr/bin".to_owned())
    })
    .unwrap();
    let (unit, _) = installed(&manager, &plan);
    assert!(unit.contains(&format!("ExecStart=\"{root}/$$bin/meta\" serve\n")));
    assert!(unit.contains(&format!("Environment=\"META_HARNESS_HOME={root}/data$\"\n")));
}

#[test]
fn a_service_file_names_its_data_folder_in_every_layout_and_refuses_one_that_does_not() {
    let home = PathBuf::from("/data $ \"q\" \\ 100% & <x>");
    let plan = Plan::new(PathBuf::from("/bin/meta"), home.clone(), |_| None).unwrap();
    let text = home.to_string_lossy().into_owned();
    assert_eq!(plist_home(&plist_of(&plan)), Some(text.clone()));
    assert_eq!(unit_home(&unit_of(&plan)), Some(text));

    let dir = Dir::new();
    let launchd = Manager::launchd(&dir, 501);
    put(
        &launchd.file,
        "<dict><key>META_HARNESS_HOME</key><string>/data &amp; 100%</string></dict>",
    );
    assert_eq!(
        launchd.recorded_home(),
        Ok(Some(PathBuf::from("/data & 100%")))
    );
    let systemd = Manager::systemd(&dir.join("config"));
    put(
        &systemd.file,
        "[Service]\n  Environment = \"META_HARNESS_HOME=/data 100%%\"\n",
    );
    assert_eq!(
        systemd.recorded_home(),
        Ok(Some(PathBuf::from("/data 100%")))
    );

    put(&systemd.file, "[Service]\nExecStart=/bin/meta serve\n");
    assert!(systemd.recorded_home().is_err());
}

#[test]
fn a_failed_replacement_restores_the_previous_unit_and_keeps_its_data_folder_on_record() {
    let dir = Dir::new();
    let manager = Manager::systemd(&dir.join("config"));
    let old = plan(&dir);
    installed(&manager, &old);

    let (result, _) = attempt(&manager, &elsewhere(&dir), |step| {
        if step.args.iter().any(|arg| arg == "daemon-reload") {
            Err("daemon-reload failed".to_owned())
        } else {
            Ok(())
        }
    });
    assert!(result.is_err());
    assert_eq!(manager.recorded_home(), Ok(Some(old.home.clone())));

    let (result, ran) = attempt(&manager, &elsewhere(&dir), |step| {
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
    assert_eq!(manager.recorded_home(), Ok(Some(old.home)));

    let mut restarts = 0;
    let (result, _) = attempt(&manager, &elsewhere(&dir), |step| {
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
}

#[test]
fn a_failed_start_fails_the_install_and_uninstall_keeps_the_data() {
    let dir = Dir::new();
    let manager = Manager::launchd(&dir, 501);
    let plan = plan(&dir);
    let (result, _) = attempt(&manager, &plan, |step| match step.args[0].as_str() {
        "bootstrap" => Err("Bootstrap failed: 5: Input/output error".to_owned()),
        _ => Err("not loaded".to_owned()),
    });
    assert_eq!(
        result,
        Err("Bootstrap failed: 5: Input/output error".to_owned())
    );

    put(plan.home.join("meta-harness.sqlite"), "");
    let warnings = manager.uninstall(|_| Err("not loaded".to_owned()));
    assert_eq!(warnings, Ok(vec!["not loaded".to_owned()]));
    assert!(!manager.file.exists());
    assert!(plan.home.join("meta-harness.sqlite").exists());
    assert_eq!(manager.uninstall(|_| Ok(())), Ok(Vec::new()));
}

#[test]
fn a_windows_logon_task_runs_meta_serve_hidden_and_logs_to_the_data_folder() {
    let dir = Dir::new();
    let manager = Manager::task_scheduler(&dir);
    let plan = plan(&dir);
    let (result, ran) = attempt(&manager, &plan, |_| Ok(()));
    result.unwrap();
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
        "--headless \"{}\" serve --home \"{}\" --log \"{}\"",
        xml(&plan.program.to_string_lossy()),
        xml(&plan.home.to_string_lossy()),
        xml(&plan.log().to_string_lossy()),
    )));
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

    let home = format!("{}\\", dir.join("data").to_string_lossy());
    let plan = Plan::new(dir.join("bin/meta"), PathBuf::from(&home), |_| None).unwrap();
    manager.install(&plan, |_| Ok(())).unwrap();
    let task = manager
        .decode(&std::fs::read(&manager.file).unwrap())
        .unwrap();
    assert!(
        task.contains(&format!("--home \"{home}\\\" --log")),
        "{task}"
    );
    assert_eq!(manager.recorded_home(), Ok(Some(PathBuf::from(home))));
}

#[test]
fn a_windows_logon_task_passes_the_harness_logins_on_to_meta_serve() {
    let dir = Dir::new();
    let codex = dir.join("codex");
    let plan = Plan::new(dir.join("bin/meta"), dir.join("data"), |name| {
        (name == "CODEX_HOME").then(|| codex.to_string_lossy().into_owned())
    })
    .unwrap();
    assert!(task_of(&plan).contains(&format!("--env \"CODEX_HOME={}\"", codex.display())));
    assert_eq!(
        split_environment(&["serve", "--env", "CODEX_HOME=/a", "--env", "PATH=/x"]),
        (
            vec![("CODEX_HOME", "/a")],
            vec!["serve", "--env", "PATH=/x"]
        )
    );
}

#[test]
fn a_failed_first_windows_task_install_deletes_the_task_it_created() {
    let dir = Dir::new();
    let manager = Manager::task_scheduler(&dir);
    let (result, ran) = attempt(&manager, &plan(&dir), |step| {
        if step.args[0] == "/Run" {
            Err("Access is denied.".to_owned())
        } else if step.args[0] == "/End" {
            Err("ERROR: The system cannot find the file specified.".to_owned())
        } else {
            Ok(())
        }
    });
    assert_eq!(result, Err("Access is denied.".to_owned()));
    assert_eq!(ran.last().unwrap(), "schtasks /Delete /TN Meta-Harness /F");
    assert!(!manager.file.exists());
}

#[test]
fn a_service_manager_saying_the_service_is_absent_or_stopped_counts_as_not_loaded() {
    for failure in [
        "Boot-out failed: 3: No such process",
        "Boot-out failed: 113: Could not find specified service",
        "Could not find service \"x\" in domain for user gui: 502",
        "Unit x.service not loaded.",
        "ERROR: The task is not currently running.",
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
    let manager = Manager::launchd(&dir, 501);
    installed(&manager, &plan(&dir));
    let result = manager.uninstall(|_| Err("Operation not permitted".to_owned()));
    assert!(result.is_err(), "{result:?}");
    assert!(manager.file.exists());

    let task = Manager::task_scheduler(&dir);
    task.install(&plan(&dir), |_| Ok(())).unwrap();
    let mut ran = Vec::new();
    let result = task.uninstall(|step| {
        ran.push(step.args[0].clone());
        Err("Operation not permitted".to_owned())
    });
    assert!(result.is_err(), "{result:?}");
    assert_eq!(
        ran,
        ["/End"],
        "the task is deleted only after its stop succeeds"
    );
    assert!(task.file.exists());
}

#[test]
fn a_failed_replacement_bootstraps_the_previous_agent_again() {
    let dir = Dir::new();
    let manager = Manager::launchd(&dir, 501);
    installed(&manager, &plan(&dir));
    let mut bootstraps = Vec::new();
    let (result, _) = attempt(&manager, &elsewhere(&dir), |step| {
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
fn a_systemd_unit_whose_file_is_gone_is_stopped_by_name() {
    let dir = Dir::new();
    let manager = Manager::systemd(&dir.join("config"));
    let mut ran = Vec::new();
    let result = manager.uninstall(|step| {
        ran.push(step.args.join(" "));
        Err("Unit meta-harness.service not loaded.".to_owned())
    });
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(ran, ["--user stop meta-harness.service"]);
    let result = manager
        .uninstall(|_| Err("Failed to stop meta-harness.service: Access denied.".to_owned()));
    assert!(result.is_err(), "{result:?}");
}

#[cfg(unix)]
#[test]
fn a_reinstall_replaces_the_service_file_whole_for_a_reader_that_has_it_open() {
    use std::io::Read;
    let dir = Dir::new();
    let manager = Manager::systemd(&dir.join("config"));
    installed(&manager, &plan(&dir));
    let previous = std::fs::read_to_string(&manager.file).unwrap();
    let mut reader = std::fs::File::open(&manager.file).unwrap();
    let other = Plan::new(dir.join("bin/meta"), dir.join("other"), |_| None).unwrap();
    manager.install(&other, |_| Ok(())).unwrap();
    let mut seen = String::new();
    reader.read_to_string(&mut seen).unwrap();
    assert_eq!(seen, previous);
    assert_ne!(std::fs::read_to_string(&manager.file).unwrap(), previous);
}
