//! Observation workload for checkpoints in a large project (PERFORMANCE.md, Meta-Harness): a Git
//! project of 20,000 committed files of 1 KiB in 200 folders. Each sample is one turn: its start
//! checkpoint, ten changed files and one new file, its end checkpoint, the list of what it
//! changed, the plan of a revert to its start and that revert. The first sample's start also
//! opens the project. The project is written by
//! a second run of this example, so building it adds nothing to the measured process. It prints
//! one JSON line per sample and a summary and never compares with a threshold. Run from the
//! repository root:
//!
//! ```sh
//! cargo build --release -p meta-harness-core --example checkpoint_observe
//! /usr/bin/time -l target/release/examples/checkpoint_observe
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use meta_harness_core::{Checkpoint, Checkpoints, Phase, Whitespace};
use serde_json::json;

const FOLDERS: usize = 200;
const FILES_PER_FOLDER: usize = 100;
const FILE_BYTES: usize = 1024;
const CHANGED_PER_TURN: usize = 10;
const SAMPLES: usize = 20;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [flag, dir] = args.as_slice()
        && flag == "--write"
    {
        write_project(Path::new(dir));
        return;
    }
    let dir = std::env::temp_dir().join(format!("checkpoint-observe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let status = Command::new(std::env::current_exe().expect("this example"))
        .arg("--write")
        .arg(&dir)
        .status()
        .expect("the writer runs");
    assert!(status.success(), "the project is written");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime starts")
        .block_on(observe(&dir));
    let _ = std::fs::remove_dir_all(&dir);
}

fn file(dir: &Path, index: usize) -> PathBuf {
    dir.join(format!("f{:03}", index / FILES_PER_FOLDER))
        .join(format!("{:03}.txt", index % FILES_PER_FOLDER))
}

/// Writes and commits the project.
fn write_project(dir: &Path) {
    for index in 0..FOLDERS * FILES_PER_FOLDER {
        let path = file(dir, index);
        std::fs::create_dir_all(path.parent().expect("folder")).expect("folder");
        let line = format!("file {index} line\n");
        std::fs::write(&path, line.repeat(FILE_BYTES / line.len() + 1)).expect("file");
    }
    for args in [
        &["init", "-q"][..],
        &["add", "."],
        &["commit", "-q", "-m", "base"],
    ] {
        let status = Command::new("git")
            .args([
                "-c",
                "user.name=Observe",
                "-c",
                "user.email=observe@example.invalid",
            ])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?}");
    }
}

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

async fn observe(dir: &Path) {
    let checkpoints = Checkpoints::new(dir);
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    let mut lists = Vec::new();
    let mut plans = Vec::new();
    let mut reverts = Vec::new();
    for sample in 0..SAMPLES {
        let at = |phase| Checkpoint {
            session: "observe".to_owned(),
            turn: format!("t{sample}"),
            phase,
        };
        let started = Instant::now();
        checkpoints
            .capture(&at(Phase::Start))
            .await
            .expect("start captured");
        let start_ms = ms(started);
        for change in 0..CHANGED_PER_TURN {
            let path = file(
                dir,
                (sample * CHANGED_PER_TURN + change) * 97 % (FOLDERS * FILES_PER_FOLDER),
            );
            std::fs::write(&path, format!("changed in turn {sample}\n")).expect("change");
        }
        std::fs::write(dir.join(format!("new-{sample}.txt")), "new\n").expect("new file");
        let started = Instant::now();
        checkpoints
            .capture(&at(Phase::End))
            .await
            .expect("end captured");
        let end_ms = ms(started);
        let started = Instant::now();
        let changes = checkpoints
            .changes(&at(Phase::Start), &at(Phase::End), Whitespace::Shown)
            .await
            .expect("changes");
        let list_ms = ms(started);
        let started = Instant::now();
        let plan = checkpoints
            .revert_plan(&at(Phase::Start), Some(&at(Phase::End)))
            .await
            .expect("plan");
        let plan_ms = ms(started);
        let started = Instant::now();
        checkpoints
            .revert(&at(Phase::Start))
            .await
            .expect("reverted");
        let revert_ms = ms(started);
        assert_eq!(plan.files.len(), changes.len(), "the revert undoes the turn");
        println!(
            "{}",
            json!({
                "sample": sample,
                "start_ms": round(start_ms),
                "end_ms": round(end_ms),
                "list_ms": round(list_ms),
                "plan_ms": round(plan_ms),
                "revert_ms": round(revert_ms),
                "files": changes.len(),
            })
        );
        starts.push(start_ms);
        ends.push(end_ms);
        lists.push(list_ms);
        plans.push(plan_ms);
        reverts.push(revert_ms);
    }
    checkpoints.close().await;
    // The first start also opens the project, so the summary leaves it out.
    let summary = |values: &mut Vec<f64>| {
        values.sort_by(f64::total_cmp);
        let at = |share: f64| values[((values.len() - 1) as f64 * share).round() as usize];
        json!({"p50_ms": round(at(0.5)), "p95_ms": round(at(0.95)), "max_ms": round(at(1.0))})
    };
    println!(
        "{}",
        json!({
            "summary": "checkpoints",
            "files": FOLDERS * FILES_PER_FOLDER,
            "samples": SAMPLES,
            "first_start_ms": round(starts[0]),
            "start": summary(&mut starts[1..].to_vec()),
            "end": summary(&mut ends),
            "list": summary(&mut lists),
            "plan": summary(&mut plans),
            "revert": summary(&mut reverts),
        })
    );
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}
