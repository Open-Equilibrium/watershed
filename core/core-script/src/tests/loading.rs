use std::path::Path;

#[path = "loading/catalog.rs"]
mod catalog;
#[path = "loading/storage.rs"]
mod storage;

const INSPECT: &str = "instruction:\n  id: inspect\n  name: Inspect\n  prompt: Inspect\n";

fn write_registry_file(dir: &Path, name: &str, source: &str) {
    std::fs::write(dir.join(name), source).expect("registry file written");
}
