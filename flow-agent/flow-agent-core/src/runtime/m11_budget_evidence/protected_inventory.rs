use super::M11BudgetOutcome;
use std::path::Path;

const CHILD_DIRECTORIES: usize = 32;
pub(super) const SMALL_FILES_PER_CHILD: usize = 16;
pub(super) const LARGE_FILES_PER_CHILD: usize = 512;

pub(super) fn inputs(files_per_child: usize) -> serde_json::Value {
    serde_json::json!({
        "home_child_directories": CHILD_DIRECTORIES,
        "single_link_files_per_child": files_per_child,
        "single_link_files": CHILD_DIRECTORIES * files_per_child,
        "file_content_bytes": 0,
        "cross_root_hardlinks_per_child": 1,
        "cross_root_hardlinked_inodes": CHILD_DIRECTORIES,
        "links_per_hardlinked_inode": 2,
        "roots": ["home", "platform", "home/child-00", "home/child-00"],
        "unique_directories": CHILD_DIRECTORIES + 2,
        "fixture_file_entries": CHILD_DIRECTORIES * (files_per_child + 2),
        "operations": 1,
        "operation": "verify_protected_directory_aliases",
        "input_bytes": 0,
        "output_bytes": 0,
        "checksum": "successfully created fixture file entries, including both hardlink names",
        "cache_preparation": "warm-cache fixture creation; no cache eviction or preliminary scan",
    })
}

#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
pub(super) fn run(temp_root: &Path, files_per_child: usize) -> Result<M11BudgetOutcome, String> {
    use super::outcome;
    use crate::runtime::{
        fs_guards::{DirectoryErrorMode, verify_protected_directory_aliases},
        session_store::open_flow_agent_home_at,
    };
    use std::{fs, time::Instant};

    let home = open_flow_agent_home_at(&temp_root.join("home"), true)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "protected inventory fixture home was not created".to_owned())?;
    let platform = open_flow_agent_home_at(&temp_root.join("platform"), true)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "protected inventory fixture platform root was not created".to_owned())?;
    let mut roots = vec![home.clone(), platform.clone()];
    let mut fixture_file_entries = 0_u64;
    for index in 0..CHILD_DIRECTORIES {
        let child = home
            .private_child(
                format!("child-{index:02}"),
                true,
                DirectoryErrorMode::Protocol,
            )
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "protected inventory fixture child was not created".to_owned())?;
        for file in 0..files_per_child {
            fs::File::create_new(child.path.join(format!("file-{file:03}")))
                .map_err(|error| error.to_string())?;
            fixture_file_entries += 1;
        }
        let source = child.path.join("protected-link");
        fs::File::create_new(&source).map_err(|error| error.to_string())?;
        fs::hard_link(
            &source,
            platform.path.join(format!("protected-link-{index:02}")),
        )
        .map_err(|error| error.to_string())?;
        fixture_file_entries += 2;
        if index == 0 {
            roots.push(child.clone());
            roots.push(child);
        }
    }

    let started = Instant::now();
    let verified = verify_protected_directory_aliases(&roots);
    let elapsed = started.elapsed();
    verified.map_err(|error| error.to_string())?;
    Ok(outcome(elapsed, 1, 0, 0, fixture_file_entries))
}

#[cfg(not(any(test, all(target_os = "linux", target_arch = "x86_64"))))]
pub(super) fn run(_temp_root: &Path, _files_per_child: usize) -> Result<M11BudgetOutcome, String> {
    Err("protected inventory evidence requires Linux x86_64 or a test build".to_owned())
}
