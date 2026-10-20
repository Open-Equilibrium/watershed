use flow_agent_core::RuntimeError;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const HARD_INTERRUPT_EXIT_CODE: i32 = 130;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InterruptDisposition {
    Continue,
    Exit,
}

fn interrupt_disposition(
    action: flow_agent_core::ProductiveInterruptAction,
) -> InterruptDisposition {
    match action {
        flow_agent_core::ProductiveInterruptAction::Cancel
        | flow_agent_core::ProductiveInterruptAction::Defer => InterruptDisposition::Continue,
        flow_agent_core::ProductiveInterruptAction::Exit => InterruptDisposition::Exit,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct InterruptCoordinator;

impl InterruptCoordinator {
    pub(crate) fn install() -> Result<Self, RuntimeError> {
        use signal_hook::{consts::SIGINT, iterator::Signals};

        let mut signals = Signals::new([SIGINT]).map_err(|_| {
            RuntimeError::Protocol("CLI interrupt handler installation failed".to_owned())
        })?;
        let coordinator = Self::new();
        std::thread::Builder::new()
            .name("flow-interrupt".to_owned())
            .spawn(move || {
                for _ in signals.forever() {
                    match interrupt_disposition(flow_agent_core::request_productive_interrupt()) {
                        InterruptDisposition::Continue => {}
                        InterruptDisposition::Exit => std::process::exit(HARD_INTERRUPT_EXIT_CODE),
                    }
                }
            })
            .map_err(|_| {
                RuntimeError::Protocol("CLI interrupt worker creation failed".to_owned())
            })?;
        Ok(coordinator)
    }

    pub(crate) fn new() -> Self {
        Self
    }

    #[cfg(test)]
    pub(crate) fn activate(&self) -> Result<ActiveOperation, RuntimeError> {
        let operation = self.operation();
        operation.activate()?;
        Ok(operation)
    }

    pub(crate) fn operation(&self) -> ActiveOperation {
        ActiveOperation {
            state: Arc::new(ActiveOperationState {
                active: AtomicBool::new(false),
            }),
        }
    }
}

#[derive(Clone)]
pub(crate) struct ActiveOperation {
    state: Arc<ActiveOperationState>,
}

impl ActiveOperation {
    pub(crate) fn activate(&self) -> Result<(), RuntimeError> {
        flow_agent_core::begin_productive_operation()?;
        self.state.active.store(true, Ordering::Release);
        Ok(())
    }
}

struct ActiveOperationState {
    active: AtomicBool,
}

impl Drop for ActiveOperationState {
    fn drop(&mut self) {
        if self.active.load(Ordering::Acquire) && flow_agent_core::settle_productive_operation() {
            std::process::exit(HARD_INTERRUPT_EXIT_CODE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HARD_INTERRUPT_EXIT_CODE, InterruptCoordinator, InterruptDisposition, interrupt_disposition,
    };
    use std::{
        path::Path,
        process::{Child, Command, ExitStatus},
        time::{Duration, Instant},
    };

    #[test]
    fn first_active_interrupt_cancels_and_repeated_interrupt_exits() {
        assert_eq!(
            interrupt_disposition(flow_agent_core::ProductiveInterruptAction::Cancel),
            InterruptDisposition::Continue
        );
        assert_eq!(
            interrupt_disposition(flow_agent_core::ProductiveInterruptAction::Exit),
            InterruptDisposition::Exit
        );
    }

    #[test]
    fn interrupt_after_completion_wins_is_deferred_until_disarm() {
        assert_eq!(
            interrupt_disposition(flow_agent_core::ProductiveInterruptAction::Defer),
            InterruptDisposition::Continue
        );
    }

    #[test]
    fn active_operation_drop_disarms_the_shared_coordinator() {
        const CHILD_ENV: &str = "WATERSHED_INTERRUPT_GUARD_CHILD";
        if std::env::var_os(CHILD_ENV).is_none() && std::env::var_os("NEXTEST").is_none() {
            let test_name = crate::test_support::current_test_name();
            let status = std::process::Command::new(
                std::env::current_exe().expect("CLI test executable resolves"),
            )
            .args(["--exact", &test_name, "--nocapture"])
            .env(CHILD_ENV, "1")
            .status()
            .expect("isolated interrupt-guard test starts");
            assert!(status.success(), "isolated interrupt-guard test failed");
            return;
        }

        let coordinator = InterruptCoordinator::new();
        let operation = coordinator.activate().expect("operation activates");
        assert_eq!(
            flow_agent_core::request_productive_interrupt(),
            flow_agent_core::ProductiveInterruptAction::Cancel
        );
        drop(operation);
        assert_eq!(
            flow_agent_core::request_productive_interrupt(),
            flow_agent_core::ProductiveInterruptAction::Exit
        );
    }

    #[test]
    fn installed_handler_controls_first_active_sigint_and_exits_on_repeat() {
        const CHILD_MODE_ENV: &str = "WATERSHED_INSTALLED_SIGINT_CHILD_MODE";
        const MARKER_ENV: &str = "WATERSHED_INSTALLED_SIGINT_MARKER";
        if let Some(mode) = std::env::var_os(CHILD_MODE_ENV) {
            let marker = std::env::var_os(MARKER_ENV).expect("signal marker path is configured");
            let coordinator = InterruptCoordinator::install().expect("signal handler installs");
            let operation = coordinator
                .activate()
                .expect("productive operation activates");
            std::fs::write(marker, b"ready").expect("signal marker is written");
            if mode == "controlled" {
                std::thread::sleep(Duration::from_millis(500));
                assert_eq!(
                    flow_agent_core::request_productive_interrupt(),
                    flow_agent_core::ProductiveInterruptAction::Exit,
                    "the installed handler processed the first active SIGINT"
                );
                drop(operation);
                std::process::exit(65);
            }
            std::thread::sleep(Duration::from_secs(5));
            drop(operation);
            panic!("the repeated SIGINT did not terminate the process");
        }

        let controlled = run_signal_child("controlled", |child| {
            send_sigint(child);
        });
        assert_eq!(controlled.code(), Some(65));

        let repeated = run_signal_child("repeated", |child| {
            send_sigint(child);
            std::thread::sleep(Duration::from_millis(250));
            assert!(
                child
                    .try_wait()
                    .expect("signal child status is readable")
                    .is_none(),
                "the first active SIGINT remains controlled"
            );
            send_sigint(child);
        });
        assert_eq!(repeated.code(), Some(HARD_INTERRUPT_EXIT_CODE));
    }

    fn run_signal_child(mode: &str, signal: impl FnOnce(&mut Child)) -> ExitStatus {
        let marker = std::env::temp_dir().join(format!(
            "watershed-installed-sigint-{}-{mode}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&marker);
        let test_name = crate::test_support::current_test_name();
        let mut child =
            Command::new(std::env::current_exe().expect("CLI test executable resolves"))
                .args(["--exact", &test_name, "--nocapture"])
                .env("WATERSHED_INSTALLED_SIGINT_CHILD_MODE", mode)
                .env("WATERSHED_INSTALLED_SIGINT_MARKER", &marker)
                .spawn()
                .expect("signal child starts");
        let watchdog = Duration::from_secs(5);
        wait_for_marker(&marker, &mut child, watchdog);
        signal(&mut child);
        let status = wait_for_child(&mut child, watchdog);
        let _ = std::fs::remove_file(marker);
        status
    }

    fn wait_for_marker(marker: &Path, child: &mut Child, watchdog: Duration) {
        let deadline = Instant::now() + watchdog;
        while !marker.exists() {
            assert!(
                child
                    .try_wait()
                    .expect("signal child status is readable")
                    .is_none(),
                "signal child exited before activating"
            );
            if Instant::now() >= deadline {
                if marker.exists() {
                    return;
                }
                child.kill().expect("timed-out signal child is terminated");
                child.wait().expect("timed-out signal child is reaped");
                let _ = std::fs::remove_file(marker);
                panic!("signal child activation timed out");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn send_sigint(child: &Child) {
        let status = Command::new("kill")
            .args(["-s", "INT", &child.id().to_string()])
            .status()
            .expect("kill command starts");
        assert!(status.success(), "SIGINT delivery failed");
    }

    fn wait_for_child(child: &mut Child, watchdog: Duration) -> ExitStatus {
        let deadline = Instant::now() + watchdog;
        loop {
            if let Some(status) = child.try_wait().expect("signal child status is readable") {
                return status;
            }
            if Instant::now() >= deadline {
                child.kill().expect("timed-out signal child is terminated");
                child.wait().expect("timed-out signal child is reaped");
                panic!("signal child did not exit in time");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
