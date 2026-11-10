//! Observation workload for checkpoints in a large project (PERFORMANCE.md, Meta-Harness): a Git
//! project of 20,000 committed files of 1 KiB in 200 folders. Each sample is one turn: its start
//! checkpoint, ten changed files and one new file, its end checkpoint, the status and stash
//! Liquid reads after each fetch and turn, the list of what it changed, the uncommitted scope,
//! commit attribution over all turns so far, the plan of a revert to its start and that revert.
//! The first sample's start also
//! opens the project. A last turn replaces a file with a folder of 20,000 folders, which the
//! plan searches for repositories before it would write the file back. Last, the sessions with
//! checkpoints are listed and all checkpoints removed. The project is written by
//! a second run of this example, so building it adds nothing to the measured process. It prints
//! one JSON line per sample and a summary and never compares with a threshold. Run from the
//! repository root:
//!
//! ```sh
//! cargo build --release -p meta-harness-core --example checkpoint_observe
//! /usr/bin/time -l target/release/examples/checkpoint_observe
//! ```

use std::fs::DirBuilder;
use std::io::ErrorKind;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use meta_harness_core::{Checkpoint, Checkpoints, Phase, TurnCheckpoints, Whitespace};
use serde_json::json;

const FOLDERS: usize = 200;
const FILES_PER_FOLDER: usize = 100;
const FILE_BYTES: usize = 1024;
const CHANGED_PER_TURN: usize = 10;
const SAMPLES: usize = 20;
const FOLDERS_IN_PLACE: usize = 20_000;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [flag, dir] = args.as_slice()
        && flag == "--write"
    {
        write_project(Path::new(dir));
        return;
    }
    let dir = private_folder();
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

/// A new folder in the temporary directory, created exclusively with owner-only permissions, so
/// a link or folder another user put at its name makes the attempt fail rather than receive writes.
fn private_folder() -> PathBuf {
    for attempt in 0..8 {
        let dir = std::env::temp_dir().join(format!(
            "checkpoint-observe-{}-{attempt}",
            std::process::id()
        ));
        match DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => return dir,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => panic!("the observation folder: {error}"),
        }
    }
    panic!("no free observation folder")
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
            // No inherited variable, such as GIT_INDEX_FILE from a Git hook, may point these
            // commands at another repository.
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
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
    let mut polls = Vec::new();
    let mut uncommitted = Vec::new();
    let mut commits = Vec::new();
    let mut turns = Vec::new();
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
        // What Liquid reads after each fetch and turn: the branch's counts and its stash.
        let started = Instant::now();
        let status = checkpoints.status().await.expect("status");
        checkpoints.stash().await.expect("stash");
        let poll_ms = ms(started);
        assert_eq!(status.changed, CHANGED_PER_TURN as u64 + 1);
        let started = Instant::now();
        let changes = checkpoints
            .changes(&at(Phase::Start), &at(Phase::End), Whitespace::Shown)
            .await
            .expect("changes");
        let list_ms = ms(started);
        turns.push(TurnCheckpoints {
            session: "observe".to_owned(),
            turn: format!("t{sample}"),
            number: sample as u64 + 1,
            start_entry: String::new(),
            end_entry: Some(String::new()),
            prompt_entry: None,
            prompt: String::new(),
            started_ms: None,
            ended_ms: None,
            ended: true,
            failure: None,
            changed: None,
        });
        let started = Instant::now();
        let files = checkpoints
            .uncommitted_changes(Whitespace::Shown)
            .await
            .expect("uncommitted")
            .1;
        let uncommitted_ms = ms(started);
        assert_eq!(files.len(), changes.len());
        let started = Instant::now();
        let commit = checkpoints
            .commit_plan(&turns)
            .await
            .expect("commit attribution");
        let commit_ms = ms(started);
        assert_eq!(commit.files.len(), changes.len());
        let started = Instant::now();
        let plan = checkpoints
            .revert_plan(&at(Phase::Start), Some(&at(Phase::End)))
            .await
            .expect("plan");
        let plan_ms = ms(started);
        let started = Instant::now();
        checkpoints
            .revert(&at(Phase::Start), &plan.confirmation)
            .await
            .expect("reverted");
        let revert_ms = ms(started);
        assert_eq!(
            plan.files.len(),
            changes.len(),
            "the revert undoes the turn"
        );
        println!(
            "{}",
            json!({
                "sample": sample,
                "start_ms": round(start_ms),
                "end_ms": round(end_ms),
                "list_ms": round(list_ms),
                "plan_ms": round(plan_ms),
                "revert_ms": round(revert_ms),
                "poll_ms": round(poll_ms),
                "uncommitted_ms": round(uncommitted_ms),
                "commit_plan_ms": round(commit_ms),
                "attributed_turns": turns.len(),
                "files": changes.len(),
            })
        );
        starts.push(start_ms);
        ends.push(end_ms);
        lists.push(list_ms);
        plans.push(plan_ms);
        reverts.push(revert_ms);
        polls.push(poll_ms);
        uncommitted.push(uncommitted_ms);
        commits.push(commit_ms);
    }
    let in_place = |phase| Checkpoint {
        session: "observe".to_owned(),
        turn: "in-place".to_owned(),
        phase,
    };
    checkpoints
        .capture(&in_place(Phase::Start))
        .await
        .expect("start captured");
    let replaced = file(dir, 0);
    std::fs::remove_file(&replaced).expect("file removed");
    for index in 0..FOLDERS_IN_PLACE {
        std::fs::create_dir_all(replaced.join(format!("d{index:05}"))).expect("folder");
    }
    checkpoints
        .capture(&in_place(Phase::End))
        .await
        .expect("end captured");
    let started = Instant::now();
    checkpoints
        .revert_plan(&in_place(Phase::Start), None)
        .await
        .expect("plan");
    let in_place_ms = ms(started);
    let started = Instant::now();
    let sessions = checkpoints.sessions().await.expect("sessions");
    let sessions_ms = ms(started);
    assert_eq!(sessions, [("observe".to_owned(), SAMPLES + 1)]);
    let started = Instant::now();
    checkpoints.remove_all().await.expect("removed");
    let remove_all_ms = ms(started);
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
            "plan_with_folders_in_place_ms": round(in_place_ms),
            "list_sessions_ms": round(sessions_ms),
            "remove_all_ms": round(remove_all_ms),
            "start": summary(&mut starts[1..].to_vec()),
            "end": summary(&mut ends),
            "list": summary(&mut lists),
            "plan": summary(&mut plans),
            "revert": summary(&mut reverts),
            "poll": summary(&mut polls),
            "uncommitted": summary(&mut uncommitted),
            "commit_plan": summary(&mut commits),
        })
    );
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}
