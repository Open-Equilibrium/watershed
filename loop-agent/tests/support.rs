use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn fixture_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join(name)
}

pub(crate) fn workspace_copy(fixture: &str) -> PathBuf {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let target =
        std::env::temp_dir().join(format!("watershed-loop-agent-{}-{id}", std::process::id()));
    if target.exists() {
        fs::remove_dir_all(&target).expect("stale temp workspace removed");
    }
    copy_fixture_workspace(&fixture_dir(fixture), &target);
    target
}

#[allow(dead_code)]
pub(crate) fn expected_stream(fixture: &str, stream: &str) -> String {
    fs::read_to_string(fixture_dir(fixture).join("expected").join(stream))
        .expect("expected stream is readable")
}

pub(crate) fn copy_fixture_workspace(source: &Path, target: &Path) {
    copy_dir(&source.join("registry"), &target.join("registry"));
    let source_config = source.join(".loop/config.yaml");
    if source_config.exists() {
        let target_config = target.join(".loop/config.yaml");
        fs::create_dir_all(target_config.parent().expect("config path has parent"))
            .expect("workspace config directory created");
        fs::copy(source_config, target_config).expect("workspace config copied");
    }
    if source.join("out").is_dir() {
        fs::create_dir_all(target.join("out")).expect("output directory shape copied");
    }
}

pub(crate) fn copy_dir(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("target directory created");
    for entry in fs::read_dir(source).expect("source directory readable") {
        let entry = entry.expect("source entry readable");
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir(&source_path, &target_path);
        } else {
            fs::copy(source_path, target_path).expect("fixture file copied");
        }
    }
}
