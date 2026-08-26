use super::super::super::error::RegistryError;
use super::super::super::load::{
    RegistryTraversalLimits, load_flow_registry_from_root, read_registry_file_to_string,
};
use super::super::super::model::{MAX_REGISTRY_FILE_BYTES, ResolvedRegistry};
#[cfg(windows)]
use super::super::create_windows_junction;
use super::super::{collect_registry_files, load_registry, registry_location, temp_registry_dir};
use std::path::Path;
#[cfg(unix)]
use std::{
    process::Command,
    time::{Duration, Instant},
};

fn load_registry_with_traversal_limits(
    root: &Path,
    limits: RegistryTraversalLimits,
) -> Result<ResolvedRegistry, RegistryError> {
    let (workspace, registry_root) = registry_location(root);
    ResolvedRegistry::load_for_flow_with_all_limits(workspace, registry_root, "root", 1024, limits)
}

#[test]
fn registry_loader_enforces_selected_root_boundary_and_reports_missing_root() {
    let root = temp_registry_dir("registry-root-boundary");
    let escaping_root = Path::new("../outside");
    let err = load_flow_registry_from_root(&root, escaping_root, "root")
        .expect_err("registry root must stay within the selected root");
    assert!(
        matches!(&err, RegistryError::UnsafePath { path, .. } if path == escaping_root),
        "unexpected error: {err:?}"
    );
    assert!(err.to_string().contains("stay within the selected root"));

    let missing_root = root.join("missing-root");
    let err = load_flow_registry_from_root(&missing_root, Path::new("registry"), "root")
        .expect_err("missing selected root must remain an I/O failure");
    assert!(
        matches!(&err, RegistryError::Io { path, source }
            if path.as_path() == missing_root && source.kind() == std::io::ErrorKind::NotFound),
        "unexpected error: {err:?}"
    );
}

#[test]
fn registry_loader_accepts_nested_yaml_files_and_ignores_non_registry_files() {
    let root = temp_registry_dir("nested-registry");
    std::fs::write(root.join("README.txt"), "ignored").expect("ignored file written");
    std::fs::create_dir_all(root.join("nested")).expect("nested dir created");
    std::fs::write(
        root.join("nested").join("instruction.yml"),
        "instruction:\n  id: inspect\n  name: Inspect\n  prompt: Inspect\n",
    )
    .expect("registry file written");
    std::fs::write(
        root.join("phase.yaml"),
        "phase:\n  id: phase\n  name: Phase\n  instruction_refs: [inspect]\n  tool_refs: []\n  output:\n    type: string\n",
    )
    .expect("phase written");
    std::fs::write(
        root.join("flow.yaml"),
        "flow:\n  id: root\n  name: Root\n  phase_refs: [phase]\n  subflow_refs: []\n",
    )
    .expect("flow written");

    let registry = load_registry(root).expect("nested yml registry loads");

    assert!(registry.instruction_block("Inspect").is_some());
}

#[test]
fn registry_loader_rejects_files_above_read_limit() {
    let root = temp_registry_dir("registry-file-read-limit");
    std::fs::write(
        root.join("instruction.yaml"),
        "instruction:\n  id: inspect\n  name: Inspect\n  prompt: Inspect\n",
    )
    .expect("registry file written");

    let (workspace, registry_root) = registry_location(&root);
    let err = ResolvedRegistry::load_for_flow_with_limits(
        workspace,
        registry_root,
        "root",
        16,
        1024,
        1024,
    )
    .expect_err("oversized registry file is rejected before parsing");

    assert!(err.to_string().contains("registry read size"));
    assert!(matches!(
        err,
        RegistryError::ReadLimitExceeded {
            path,
            bytes,
            max: 16,
        } if path.ends_with("instruction.yaml") && bytes > 16
    ));
}

#[test]
fn registry_file_reader_enforces_limit_before_utf8_decoding() {
    let root = temp_registry_dir("registry-bounded-file-read");
    let path = root.join("instruction.yaml");
    let mut source = vec![b'a'; 17];
    source.push(0xff);
    std::fs::write(&path, source).expect("registry file written");
    let (opened_root, files) = collect_registry_files(&root).expect("registry file collected");
    assert_eq!(files.len(), 1);

    let err = read_registry_file_to_string(&opened_root, &files[0], 16)
        .expect_err("oversized registry file is rejected before decoding trailing bytes");

    assert!(matches!(
        err,
        RegistryError::ReadLimitExceeded {
            path: error_path,
            bytes: 17,
            max: 16,
        } if error_path == path
    ));
}

#[test]
fn registry_file_reader_rejects_invalid_utf8() {
    let root = temp_registry_dir("registry-invalid-utf8");
    let invalid_utf8 = root.join("invalid.yaml");
    std::fs::write(&invalid_utf8, [0xff]).expect("invalid UTF-8 registry file written");
    let (opened_root, files) = collect_registry_files(&root).expect("registry file collected");
    assert_eq!(files.len(), 1);
    let error = read_registry_file_to_string(&opened_root, &files[0], MAX_REGISTRY_FILE_BYTES)
        .expect_err("invalid UTF-8 is rejected");
    assert!(std::error::Error::source(&error).is_some());
    assert!(error.to_string().contains("invalid.yaml"));
    assert!(matches!(
        error,
        RegistryError::Io { source, .. } if source.kind() == std::io::ErrorKind::InvalidData
    ));
}

#[test]
fn registry_file_reader_reports_leaf_removed_after_collection() {
    let root = temp_registry_dir("registry-leaf-removed");
    let path = root.join("instruction.yaml");
    std::fs::write(
        &path,
        "instruction:\n  id: inspect\n  name: Inspect\n  prompt: Inspect\n",
    )
    .expect("registry file written");
    let (opened_root, files) = collect_registry_files(&root).expect("registry file collected");
    std::fs::remove_file(&path).expect("registry file removed");

    let err = read_registry_file_to_string(&opened_root, &files[0], MAX_REGISTRY_FILE_BYTES)
        .expect_err("disappearing registry file must remain an I/O failure");
    assert!(
        matches!(&err, RegistryError::Io { path: error_path, source }
            if error_path.as_path() == path && source.kind() == std::io::ErrorKind::NotFound),
        "unexpected error: {err:?}"
    );
}

#[cfg(unix)]
#[test]
fn registry_file_reader_rejects_fifo_replacement_without_blocking() {
    const CHILD_MARKER: &str = "WATERSHED_CORE_SCRIPT_FIFO_CHILD";
    if let Some(marker) = std::env::var_os(CHILD_MARKER) {
        std::fs::write(marker, "started").expect("child marker written");
        let root = temp_registry_dir("registry-fifo-replacement-child");
        let path = root.join("instruction.yaml");
        std::fs::write(
            &path,
            "instruction:\n  id: inspect\n  name: Inspect\n  prompt: Inspect\n",
        )
        .expect("registry file written");
        let (opened_root, files) = collect_registry_files(&root).expect("registry file collected");
        std::fs::remove_file(&path).expect("registry file removed");
        assert!(
            Command::new("mkfifo")
                .arg(&path)
                .status()
                .expect("mkfifo runs")
                .success(),
            "mkfifo must create the replacement"
        );

        let err = read_registry_file_to_string(&opened_root, &files[0], MAX_REGISTRY_FILE_BYTES)
            .expect_err("FIFO replacement must be rejected");
        assert!(matches!(err, RegistryError::UnsafePath { .. }));
        return;
    }

    let marker_dir = temp_registry_dir("registry-fifo-replacement-parent");
    let marker = marker_dir.join("child-started");
    let mut child = Command::new(std::env::current_exe().expect("test binary path"))
        .args([
            "--exact",
            "script::tests::loading::storage::registry_file_reader_rejects_fifo_replacement_without_blocking",
            "--nocapture",
        ])
        .env(CHILD_MARKER, &marker)
        .spawn()
        .expect("child test starts");
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("child status is readable") {
            break status;
        }
        if started.elapsed() >= Duration::from_secs(3) {
            child.kill().expect("blocked child stops");
            child.wait().expect("blocked child is reaped");
            panic!("registry reader blocked while opening a FIFO replacement");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(marker.is_file(), "child regression path must run");
    assert!(status.success(), "child regression path must pass");
}

#[cfg(any(unix, windows))]
#[test]
fn registry_file_reader_rejects_ancestor_replaced_by_link_after_collection() {
    let root = temp_registry_dir("registry-ancestor-swap");
    let nested = root.join("nested");
    let outside = temp_registry_dir("registry-ancestor-swap-outside");
    std::fs::create_dir(&nested).expect("nested registry directory created");
    std::fs::write(
        nested.join("instruction.yaml"),
        "instruction:\n  id: inside\n  name: Inside\n  prompt: Inside\n",
    )
    .expect("inside registry file written");
    std::fs::write(
        outside.join("instruction.yaml"),
        "instruction:\n  id: outside\n  name: Outside\n  prompt: Outside\n",
    )
    .expect("outside registry file written");
    let (opened_root, files) = collect_registry_files(&root).expect("registry file collected");
    assert_eq!(files.len(), 1);

    std::fs::rename(&nested, root.join("retired")).expect("nested directory retired");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &nested).expect("replacement symlink created");
    #[cfg(windows)]
    create_windows_junction(&nested, &outside);

    let err = read_registry_file_to_string(&opened_root, &files[0], MAX_REGISTRY_FILE_BYTES)
        .expect_err("replacement link must not be followed");
    assert!(matches!(err, RegistryError::UnsafePath { .. }));
}

#[test]
fn registry_loader_rejects_total_bytes_above_read_limit() {
    let root = temp_registry_dir("registry-total-read-limit");
    let first = "instruction:\n  id: inspect-a\n  name: InspectA\n  prompt: Inspect\n";
    let second = "instruction:\n  id: inspect-b\n  name: InspectB\n  prompt: Inspect\n";
    std::fs::write(root.join("a.yaml"), first).expect("first registry file written");
    std::fs::write(root.join("b.yaml"), second).expect("second registry file written");

    let (workspace, registry_root) = registry_location(&root);
    let err = ResolvedRegistry::load_for_flow_with_limits(
        workspace,
        registry_root,
        "root",
        1024,
        u64::try_from(first.len()).expect("test length fits u64"),
        1024,
    )
    .expect_err("registry total size is rejected before parsing all files");

    assert!(matches!(
        err,
        RegistryError::ReadLimitExceeded {
            path,
            bytes,
            max,
        } if path.as_path() == root.as_ref() && bytes > max
    ));
}

#[test]
fn registry_loader_bounds_definition_and_non_definition_entries_independently() {
    let root = temp_registry_dir("registry-entry-count-limit");
    std::fs::write(
        root.join("a.yaml"),
        "instruction:\n  id: inspect-a\n  name: InspectA\n  prompt: Inspect\n",
    )
    .expect("registry file written");
    std::fs::write(root.join("README.txt"), "ignored").expect("non-registry entry written");
    std::fs::write(root.join("LICENSE.txt"), "ignored").expect("second non-registry entry written");

    let err = load_registry_with_traversal_limits(
        &root,
        RegistryTraversalLimits {
            max_file_bytes: 1024,
            max_total_bytes: 1024,
            max_entries: 1,
            max_depth: 64,
        },
    )
    .expect_err("non-definition traversal entries are bounded independently");

    assert!(
        err.to_string()
            .contains("registry traversal non-definition entry count")
    );
    assert!(matches!(
        err,
        RegistryError::TraversalLimitExceeded {
            limit: "non-definition entry count",
            observed: 2,
            max: 1,
            ..
        }
    ));

    std::fs::remove_file(root.join("README.txt")).expect("first ignored entry is removed");
    std::fs::remove_file(root.join("LICENSE.txt")).expect("second ignored entry is removed");
    std::fs::write(
        root.join("b.yaml"),
        "instruction:\n  id: inspect-b\n  name: InspectB\n  prompt: Inspect\n",
    )
    .expect("second registry definition is written");
    let err = load_registry_with_traversal_limits(
        &root,
        RegistryTraversalLimits {
            max_file_bytes: 1024,
            max_total_bytes: 1024,
            max_entries: 1,
            max_depth: 64,
        },
    )
    .expect_err("definitions are bounded independently");
    assert!(matches!(
        err,
        RegistryError::TraversalLimitExceeded {
            limit: "definition entry count",
            observed: 2,
            max: 1,
            ..
        }
    ));
}

#[test]
fn registry_loader_rejects_directories_above_traversal_depth_limit() {
    let root = temp_registry_dir("registry-depth-limit");
    std::fs::create_dir_all(root.join("nested")).expect("nested dir created");
    std::fs::write(
        root.join("nested").join("instruction.yaml"),
        "instruction:\n  id: inspect\n  name: Inspect\n  prompt: Inspect\n",
    )
    .expect("registry file written");

    let err = load_registry_with_traversal_limits(
        &root,
        RegistryTraversalLimits {
            max_file_bytes: 1024,
            max_total_bytes: 1024,
            max_entries: 1024,
            max_depth: 0,
        },
    )
    .expect_err("registry traversal depth is rejected before recursion");

    assert!(matches!(
        err,
        RegistryError::TraversalLimitExceeded {
            limit: "depth",
            observed: 1,
            max: 0,
            ..
        }
    ));
}

#[test]
fn temp_registry_dir_removes_its_tree_on_drop() {
    let path = {
        let root = temp_registry_dir("temp-registry-cleanup");
        std::fs::write(root.join("marker"), "cleanup").expect("marker written");
        root.to_path_buf()
    };

    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn registry_loader_rejects_symlinked_registry_entries() {
    use std::os::unix::fs::symlink;

    let root = temp_registry_dir("symlink-root");
    let outside = temp_registry_dir("symlink-outside");
    symlink(&outside, root.join("linked")).expect("registry symlink created");

    let err = load_registry(&root).expect_err("registry symlink must be rejected");

    assert!(
        matches!(err, RegistryError::UnsafePath { message, .. } if message.contains("symlink"))
    );
}

#[cfg(windows)]
#[test]
fn registry_loader_rejects_junction_registry_entries() {
    let root = temp_registry_dir("junction-root");
    let outside = temp_registry_dir("junction-outside");
    std::fs::write(
        outside.join("outside-tool.yaml"),
        r#"
tool:
  id: outside-tool
  name: Outside Tool
  tool_kind: own-script
  command: script:outside-tool
  script_runtime: posix-sh
  script_body: |
    echo outside
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
    )
    .expect("outside registry file written");
    create_windows_junction(&root.join("linked"), &outside);

    let err = load_registry(&root).expect_err("registry junction must be rejected");

    assert!(
        matches!(err, RegistryError::UnsafePath { ref message, .. } if message.contains("reparse")),
        "unexpected error: {err:?}"
    );
}

#[cfg(any(unix, windows))]
#[test]
fn registry_loader_rejects_linked_registry_root() {
    let parent = temp_registry_dir("linked-root-parent");
    let outside = temp_registry_dir("linked-root-target");
    let linked_root = parent.join("linked-root");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &linked_root).expect("registry root symlink created");
    #[cfg(windows)]
    create_windows_junction(&linked_root, &outside);

    let err = load_flow_registry_from_root(&parent, Path::new("linked-root"), "root")
        .expect_err("linked registry root must be rejected");

    assert!(
        matches!(err, RegistryError::UnsafePath { ref path, ref message }
            if path == &linked_root && (message.contains("symlink") || message.contains("reparse"))),
        "unexpected error: {err:?}"
    );
}
