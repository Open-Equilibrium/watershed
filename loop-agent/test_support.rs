use std::{fs, path::Path};

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
