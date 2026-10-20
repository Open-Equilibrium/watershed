use super::{Fixture, assert_success, pty::Terminal, wait_for_file};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, PermissionsExt},
    },
};

fn results(fixture: &Fixture, rows: &[Value], allowed: impl Fn(&Value) -> bool) {
    let actual = fixture.operation_results();
    assert_eq!(actual.len(), rows.len(), "{actual:?}");
    for (result, row) in actual.iter().zip(rows) {
        assert_eq!(result["case"], row["case"]);
        assert!(allowed(result), "{result:?}");
    }
}

fn denied(result: &Value) -> bool {
    matches!(result["errno"].as_i64(), Some(1 | 13 | 30))
}

#[test]
fn selected_home_external_store_and_programs_cover_later_trusted_publications() {
    // Admission/discovery and publication-lease tests belong to the controller.
    // Here its explicit complete inventory is enforced by the actual Executor.
    for guarded in [false, true] {
        let mut fixture = Fixture::with_image_name("custom-executor");
        let store = fixture.root.join("platform-store");
        fs::create_dir(&store).unwrap();
        fixture.protect(&store);
        let mut paths = vec![fixture.image.clone()];
        for name in ["flow", "flow-executor"] {
            let path = fixture.root.join(name);
            fs::write(&path, b"installed program fixture").unwrap();
            fixture.protect(&path);
            paths.push(path);
        }
        for path in [
            fixture.home.join("AGENTS.md"),
            fixture.home.join("config.json"),
            fixture.home.join("registry.json"),
            fixture.home.join("context"),
            fixture.home.join("runtime"),
            fixture.home.join("replaced"),
            store.join("selection"),
        ] {
            fs::write(&path, b"original").unwrap();
            paths.push(path);
        }
        fs::hard_link(
            fixture.home.join("context"),
            fixture.home.join("internal-alias"),
        )
        .unwrap();
        paths.push(fixture.home.join("internal-alias"));
        paths.extend([
            fixture.home.join("created"),
            fixture.home.join("published-alias"),
            fixture.home.join("published-directory/record"),
            store.join("new-record"),
        ]);
        let rows = paths
            .iter()
            .map(|path| json!({"case":path, "action":"write", "target":path}))
            .collect::<Vec<_>>();
        let request = fixture.operations(json!(rows), guarded);
        let mut running = guarded.then(|| {
            let mut running = fixture.spawn(&request);
            running.ready(&request);
            running.start(&request);
            wait_for_file(&fixture.project.join("helper-ready"));
            running
        });
        // Trusted publication is deliberately after the guarded Tool has started.
        fs::write(fixture.home.join("created"), b"created by owner").unwrap();
        OpenOptions::new()
            .append(true)
            .open(fixture.home.join("runtime"))
            .unwrap()
            .write_all(b"+appended by owner")
            .unwrap();
        fs::hard_link(
            fixture.home.join("context"),
            fixture.home.join("published-alias"),
        )
        .unwrap();
        fs::write(fixture.home.join("staged-file"), b"replacement by owner").unwrap();
        fs::rename(
            fixture.home.join("staged-file"),
            fixture.home.join("replaced"),
        )
        .unwrap();
        fs::create_dir(fixture.home.join("staged-directory")).unwrap();
        fs::write(
            fixture.home.join("staged-directory/record"),
            b"staged by owner",
        )
        .unwrap();
        fs::rename(
            fixture.home.join("staged-directory"),
            fixture.home.join("published-directory"),
        )
        .unwrap();
        fs::write(store.join("new-record"), b"external publication by owner").unwrap();
        let digest = |path: &std::path::Path| {
            let mut hash = Sha256::new();
            let mut file = File::open(path).unwrap();
            let mut buffer = [0; 8192];
            loop {
                let count = file.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                hash.update(&buffer[..count]);
            }
            hash.finalize()
        };
        let expected = paths.iter().map(|path| digest(path)).collect::<Vec<_>>();
        if let Some(running) = &mut running {
            fs::write(fixture.project.join("release"), b"go").unwrap();
            assert_success(&running.completed(&request));
        } else {
            fixture.baseline(&request, &[]);
        }
        results(&fixture, &rows, |result| {
            if guarded {
                denied(result)
            } else {
                result["errno"] == 0
            }
        });
        for (path, expected) in paths.iter().zip(expected) {
            if guarded {
                assert_eq!(digest(path), expected, "{} changed", path.display());
            } else {
                assert_eq!(fs::read(path).unwrap(), b"changed", "{}", path.display());
            }
        }
    }
}

#[test]
fn protected_metadata_mapped_bytes_and_hardlinks_have_real_allowed_controls() {
    for guarded in [false, true] {
        let fixture = Fixture::new();
        let rows = ["chmod", "xattr", "mmap", "link"].map(|action| {
            let path = fixture.home.join(action);
            fs::write(&path, b"original").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            json!({"case": action, "action": action, "target": path,
                   "auxiliary": fixture.project.join("hardlink")})
        });
        let request = fixture.operations(json!(rows), false);
        if guarded {
            let mut running = fixture.spawn(&request);
            running.ready(&request);
            running.start(&request);
            assert_success(&running.completed(&request));
        } else {
            fixture.baseline(&request, &[]);
        }
        results(&fixture, &rows, |result| {
            if guarded {
                denied(result) || (result["case"] == "link" && result["errno"] == 18)
            } else {
                result["errno"] == 0
            }
        });
        assert_eq!(
            fixture.operation_results()[1]["value"],
            if guarded {
                Value::Null
            } else {
                json!("changed")
            }
        );
        assert_eq!(
            fs::read(fixture.home.join("mmap")).unwrap(),
            if guarded { b"original" } else { b"Xriginal" }
        );
        assert_eq!(
            fs::metadata(fixture.home.join("chmod")).unwrap().mode() & 0o777,
            if guarded { 0o600 } else { 0o777 }
        );
        let alias = fixture.project.join("hardlink");
        assert_eq!(alias.exists(), !guarded);
        if !guarded {
            assert_eq!(
                fs::metadata(alias).unwrap().ino(),
                fs::metadata(fixture.home.join("link")).unwrap().ino()
            );
        }
    }
}

#[test]
fn writable_and_internal_handles_do_not_reach_the_tool() {
    for guarded in [false, true] {
        let fixture = Fixture::new();
        let path = fixture.home.join("handle-target");
        fs::write(&path, b"original").unwrap();
        let writable = OpenOptions::new().write(true).open(&path).unwrap();
        let directory = File::open(&fixture.home).unwrap();
        let image = File::open(&fixture.image).unwrap();
        let mut terminal = Terminal::new();
        let rows = [
            json!({"case":"file", "action":"fd-write", "target":"200"}),
            json!({"case":"directory", "action":"fd-create", "target":"201"}),
            json!({"case":"terminal", "action":"fd-write", "target":"202"}),
            json!({"case":"protected-directory", "action":"fd-stat", "target":"32"}),
            json!({"case":"protected-image", "action":"fd-stat", "target":"33"}),
        ];
        let request = fixture.operations(json!(rows), false);
        let handles = [
            (writable.as_raw_fd(), 200),
            (directory.as_raw_fd(), 201),
            (terminal.slave.as_raw_fd(), 202),
        ];
        if guarded {
            let mut running = fixture.spawn_with_handles(&request, &handles);
            running.ready(&request);
            running.start(&request);
            assert_success(&running.completed(&request));
        } else {
            let mut baseline_handles = handles.to_vec();
            baseline_handles.extend([(directory.as_raw_fd(), 32), (image.as_raw_fd(), 33)]);
            fixture.baseline(&request, &baseline_handles);
            // The writable terminal really received bytes, not just an open fd.
            rustix::fs::fcntl_setfl(&terminal.master, rustix::fs::OFlags::NONBLOCK).unwrap();
            let mut bytes = [0; 7];
            terminal.master.read_exact(&mut bytes).unwrap();
            assert_eq!(&bytes, b"changed");
        }
        results(&fixture, &rows, |result| {
            result["errno"] == if guarded { 9 } else { 0 }
        });
        assert_eq!(
            fs::read(path).unwrap(),
            if guarded { b"original" } else { b"changedl" }
        );
        assert_eq!(fixture.home.join("from-handle").exists(), !guarded);
    }
}

#[test]
fn current_and_later_host_terminals_are_inaccessible_to_the_tool() {
    let fixture = Fixture::new();
    let current = Terminal::new();
    let pointer = fixture.project.join("later-terminal");
    let rows = [
        json!({"case":"current", "action":"terminal", "target":current.path}),
        json!({"case":"later", "action":"terminal", "target":pointer, "indirect":true}),
    ];
    let request = fixture.operations(json!(rows), true);
    let mut running = fixture.spawn(&request);
    running.ready(&request);
    running.start(&request);
    wait_for_file(&fixture.project.join("helper-ready"));
    let later = Terminal::new();
    fs::write(&pointer, later.path.to_str().unwrap()).unwrap();
    let baseline = Fixture::new();
    let control = baseline.operations(json!(rows), false);
    baseline.baseline(&control, &[]);
    results(&baseline, &rows, |result| result["errno"] == 0);
    fs::write(fixture.project.join("release"), b"go").unwrap();
    assert_success(&running.completed(&request));
    // Linux's private device view may hide a live host PTY; the real baseline
    // above proves both names exist and are accessible outside the boundary.
    results(&fixture, &rows, |result| {
        denied(result) || (cfg!(target_os = "linux") && result["errno"] == 2)
    });
}
