#[cfg(windows)]
use crate::tests::helpers::create_windows_junction;
use crate::{
    runtime::{
        apply::{FlowApplication, apply_flow_with_sink},
        context::ContextManifestCheckpoint,
        event_writer::RuntimeEventSink,
        execution_plan::{FlowExecutionOptions, ToolSideEffectMode},
        planning::plan_flow,
        session::{run_flow, set_run_post_config_observer, set_run_pre_plan_observer},
        types::{EmitMode, EventClock, RuntimeError},
    },
    tests::{
        helpers::{empty_workspace, fixture_runtime_policy},
        test_support::workspace_copy,
    },
};
use proto::{EventEnvelope, EventType};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn replace_ambient_registry_text(workspace: &Path, path: &str, before: &str, after: &str) {
    let path = workspace.join("registry").join(path);
    let text = fs::read_to_string(&path).expect("ambient registry fixture reads");
    assert_eq!(
        text.matches(before).count(),
        1,
        "ambient registry fixture contains one target fragment"
    );
    fs::write(path, text.replacen(before, after, 1)).expect("ambient registry fixture updates");
}

#[test]
fn tool_dispatch_rejects_a_workspace_root_rebound_after_run_or_tool_start() {
    struct RebindWorkspace<'a> {
        moved: PathBuf,
        outside: &'a Path,
        rebound: bool,
        rebind_blocked: bool,
        trigger: EventType,
        workspace: &'a Path,
    }

    impl RuntimeEventSink for RebindWorkspace<'_> {
        fn commit(
            &mut self,
            event: &EventEnvelope,
            _canonical_jsonl: &str,
            _context_manifest: Option<ContextManifestCheckpoint>,
        ) -> Result<(), RuntimeError> {
            if !self.rebound
                && !self.rebind_blocked
                && event.event_type == self.trigger
                && (self.trigger != EventType::ToolStarted
                    || event
                        .payload
                        .get("tool_id")
                        .and_then(serde_json::Value::as_str)
                        == Some("write-summary"))
            {
                if let Err(source) = fs::rename(self.workspace, &self.moved) {
                    if cfg!(windows) && source.raw_os_error() == Some(32) {
                        self.rebind_blocked = true;
                        return Ok(());
                    }
                    panic!("workspace root must move or be retained open: {source}");
                }
                fs::rename(self.outside, self.workspace).expect("workspace root is rebound");
                self.rebound = true;
            }
            Ok(())
        }
    }

    for (label, trigger) in [
        ("run", EventType::SessionStarted),
        ("tool", EventType::ToolStarted),
    ] {
        let workspace = workspace_copy("hello-flow");
        let outside = empty_workspace(&format!("outside-{label}-rebound-workspace"));
        let moved = workspace.with_extension(format!("{label}-original"));
        let (registry, policy) = fixture_runtime_policy("hello-flow", "hello-flow");
        let flow_block = registry
            .flow_block("hello-flow")
            .expect("hello flow exists");
        let mut sink = RebindWorkspace {
            moved: moved.clone(),
            outside: &outside,
            rebound: false,
            rebind_blocked: false,
            trigger,
            workspace: &workspace,
        };

        let plan = plan_flow(
            &workspace,
            &registry,
            &policy,
            flow_block,
            &format!("rebound{label}001"),
            FlowExecutionOptions::new(EventClock::fixed_fixture(), ToolSideEffectMode::Plan),
        )
        .expect("flow planning succeeds");
        let result = apply_flow_with_sink(
            FlowApplication {
                workspace: &workspace,
                session_id: &format!("rebound{label}001"),
                options: FlowExecutionOptions::new(
                    EventClock::fixed_fixture(),
                    ToolSideEffectMode::Apply,
                ),
                plan: &plan,
            },
            Some(&mut sink),
        );

        if sink.rebind_blocked {
            assert!(
                result.is_ok(),
                "an OS-retained workspace root must remain dispatchable"
            );
            assert!(
                !outside.join("out/summary.txt").exists(),
                "outside workspace must remain untouched"
            );
            continue;
        }
        assert!(
            sink.rebound,
            "{label} fixture must rebind the workspace root"
        );
        fs::rename(&*workspace, &*outside).expect("outside workspace restores");
        fs::rename(&moved, &*workspace).expect("original workspace root restores");
        let execution = result.expect("workspace rebind is recorded as a terminal execution");
        assert!(execution.failed, "{label} workspace rebind must fail");
        let err = execution
            .terminal_error
            .expect("workspace rebind preserves its terminal error");
        assert!(
            err.to_string().contains("workspace root identity changed"),
            "{err}"
        );
        assert!(
            !outside.join("out/summary.txt").exists(),
            "rebound outside workspace must remain untouched"
        );
    }
}

#[test]
fn apply_rejects_a_workspace_root_rebound_after_planning() {
    let workspace = workspace_copy("hello-flow");
    let outside = workspace_copy("hello-flow");
    let moved = workspace.with_extension("planned-original");
    let (registry, policy) = fixture_runtime_policy("hello-flow", "hello-flow");
    let flow_block = registry
        .flow_block("hello-flow")
        .expect("hello flow exists");
    let plan = plan_flow(
        &workspace,
        &registry,
        &policy,
        flow_block,
        "reboundplan001",
        FlowExecutionOptions::new(EventClock::fixed_fixture(), ToolSideEffectMode::Plan),
    )
    .expect("flow plans against the original workspace");

    fs::rename(&*workspace, &moved).expect("planned workspace root moves");
    fs::rename(&*outside, &*workspace).expect("workspace root is rebound before apply");
    let result = apply_flow_with_sink(
        FlowApplication {
            workspace: &workspace,
            session_id: "reboundplan001",
            options: FlowExecutionOptions::new(
                EventClock::fixed_fixture(),
                ToolSideEffectMode::Apply,
            ),
            plan: &plan,
        },
        None,
    );
    fs::rename(&*workspace, &*outside).expect("rebound workspace restores");
    fs::rename(&moved, &*workspace).expect("planned workspace restores");

    let err = result.expect_err("apply must retain the planned workspace identity");
    assert!(
        err.to_string().contains("workspace root identity changed"),
        "{err}"
    );
    assert!(
        !outside.join("out/summary.txt").exists(),
        "rebound outside workspace must remain untouched"
    );
}

#[test]
fn run_rejects_a_workspace_root_rebound_after_config_load() {
    let workspace = workspace_copy("hello-flow");
    let outside = workspace_copy("hello-flow");
    let moved = workspace.with_extension("configured-original");
    let workspace_for_observer = workspace.to_path_buf();
    let outside_for_observer = outside.to_path_buf();
    let moved_for_observer = moved.clone();
    let rebind_blocked = std::rc::Rc::new(std::cell::Cell::new(false));
    let observer_rebind_blocked = rebind_blocked.clone();
    set_run_post_config_observer(move || {
        if let Err(source) = fs::rename(&workspace_for_observer, &moved_for_observer) {
            if cfg!(windows) && source.raw_os_error() == Some(32) {
                observer_rebind_blocked.set(true);
                return;
            }
            panic!("configured workspace root must move or be retained open: {source}");
        }
        fs::rename(&outside_for_observer, &workspace_for_observer)
            .expect("workspace root is rebound after config load");
    });

    let result = run_flow(&workspace, "hello-flow", EmitMode::Jsonl);
    if rebind_blocked.get() {
        assert!(result.is_ok(), "OS-retained workspace remains runnable");
        assert!(!outside.join("out/summary.txt").exists());
        return;
    }
    fs::rename(&*workspace, &*outside).expect("rebound workspace restores");
    fs::rename(&moved, &*workspace).expect("configured workspace restores");

    let err = result.expect_err("configured workspace identity must remain authoritative");
    assert!(
        err.to_string().contains("workspace root identity changed"),
        "{err}"
    );
    assert!(
        !outside.join("out/summary.txt").exists(),
        "rebound outside workspace must remain untouched"
    );
}

#[cfg(windows)]
#[test]
fn run_uses_the_retained_workspace_when_a_root_junction_is_transiently_rebound() {
    let original = workspace_copy("hello-flow");
    let rebound = workspace_copy("hello-flow");
    replace_ambient_registry_text(
        &rebound,
        "tools/write-summary.yaml",
        "printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        "printf 'rebound\\n' > out/summary.txt",
    );
    let workspace = empty_workspace("transient-root-junction");
    fs::remove_dir(&*workspace).expect("junction path starts absent");
    create_windows_junction(&workspace, &original);

    let workspace_for_rebind = workspace.to_path_buf();
    let rebound_for_observer = rebound.to_path_buf();
    set_run_post_config_observer(move || {
        fs::remove_dir(&workspace_for_rebind).expect("original workspace junction removed");
        create_windows_junction(&workspace_for_rebind, &rebound_for_observer);
    });
    let workspace_for_restore = workspace.to_path_buf();
    let original_for_observer = original.to_path_buf();
    set_run_pre_plan_observer(move || {
        fs::remove_dir(&workspace_for_restore).expect("rebound workspace junction removed");
        create_windows_junction(&workspace_for_restore, &original_for_observer);
    });

    let output = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("transient root rebind cannot replace capability-loaded definitions");

    assert!(!output.failed);
    assert_eq!(
        fs::read_to_string(original.join("out/summary.txt")).expect("original output is readable"),
        "hello\n"
    );
    assert!(!rebound.join("out/summary.txt").exists());
    fs::remove_dir(&*workspace).expect("test junction removed");
}

#[cfg(unix)]
#[test]
fn run_uses_the_retained_workspace_when_the_root_is_transiently_rebound() {
    let workspace = workspace_copy("hello-flow");
    let rebound = workspace_copy("hello-flow");
    replace_ambient_registry_text(
        &rebound,
        "tools/write-summary.yaml",
        "printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        "printf 'rebound\\n' > out/summary.txt",
    );
    let moved = workspace.with_extension("transient-original");
    let workspace_for_rebind = workspace.to_path_buf();
    let rebound_for_observer = rebound.to_path_buf();
    let moved_for_rebind = moved.clone();
    set_run_post_config_observer(move || {
        fs::rename(&workspace_for_rebind, &moved_for_rebind).expect("original workspace moves");
        fs::rename(&rebound_for_observer, &workspace_for_rebind)
            .expect("workspace root is rebound for registry loading");
    });
    let workspace_for_restore = workspace.to_path_buf();
    let rebound_for_restore = rebound.to_path_buf();
    let moved_for_restore = moved.clone();
    set_run_pre_plan_observer(move || {
        fs::rename(&workspace_for_restore, &rebound_for_restore)
            .expect("rebound workspace moves back");
        fs::rename(&moved_for_restore, &workspace_for_restore)
            .expect("original workspace root is restored");
    });

    let output = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("transient root rebind cannot replace capability-loaded definitions");

    assert!(!output.failed);
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("original output is readable"),
        "hello\n"
    );
    assert!(!rebound.join("out/summary.txt").exists());
}

#[cfg(unix)]
#[test]
fn run_retains_one_workspace_root_across_reservation_planning_and_apply() {
    let workspace = workspace_copy("hello-flow");
    let outside = workspace_copy("hello-flow");
    let moved = workspace.with_extension("run-original");
    let workspace_for_observer = workspace.to_path_buf();
    let outside_for_observer = outside.to_path_buf();
    let moved_for_observer = moved.clone();
    set_run_pre_plan_observer(move || {
        fs::rename(&workspace_for_observer, &moved_for_observer)
            .expect("reserved workspace root moves before planning");
        fs::rename(&outside_for_observer, &workspace_for_observer)
            .expect("workspace root is rebound before planning");
    });

    let result = run_flow(&workspace, "hello-flow", EmitMode::Jsonl);
    fs::rename(&*workspace, &*outside).expect("rebound workspace restores");
    fs::rename(&moved, &*workspace).expect("reserved workspace restores");

    let err = result.expect_err("the run-bound workspace identity must remain authoritative");
    assert!(
        err.to_string().contains("workspace root identity changed"),
        "{err}"
    );
    assert!(
        !outside.join("out/summary.txt").exists(),
        "rebound outside workspace must remain untouched"
    );
}
