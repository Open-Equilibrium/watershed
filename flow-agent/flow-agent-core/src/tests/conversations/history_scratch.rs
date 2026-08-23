use super::super::helpers::empty_workspace;
use super::{
    FLOW_HASH, REGISTRY_HASH, create_review_run, create_terminal_review_run, entry,
    file_tree_bytes,
    history_support::{
        assert_history_validation_scratch_is_empty, history_validation_root,
        stale_history_validation_scratch, write_history_records,
    },
    write_terminal_run,
};
use crate::runtime::{
    conversations::{
        HistoryScratchFault, HistoryScratchMemberStage, HistoryScratchStage,
        MAX_CONVERSATION_SCAN_RECORDS, abandon_history_index_scratch_for_test,
        abandon_history_index_scratches_for_test, complete_history_index_scratch_for_test,
        create_conversation_run, history_index_limits_for_test, read_conversation_history,
        reserve_conversation_continuation, set_history_index_available_space_for_test,
        set_history_index_sort_record_limit_for_test, set_history_scratch_fault_for_test,
        take_history_index_metrics_for_test, with_history_scratch_member_observer_for_test,
        with_history_scratch_stage_observer_for_test,
    },
    session_authority::SessionOwnershipLease,
};
use std::{
    fs::{self},
    path::Path,
    sync::mpsc,
    thread,
};

#[test]
fn conversation_history_scratch_budget() {
    let workspace = empty_workspace("conversation-history-active-read-bound");
    create_terminal_review_run(&workspace);
    let records = (0..=MAX_CONVERSATION_SCAN_RECORDS)
        .map(|index| {
            let parent = (index > 0).then(|| format!("entry-{:04}", index - 1));
            entry(
                &format!("entry-{index:04}"),
                parent.as_deref(),
                "review-1",
                1,
            )
        })
        .collect::<Vec<_>>();
    write_history_records(&workspace, "review", &records);

    let history = read_conversation_history(&workspace, "review")
        .expect("valid history continues across scan quanta");
    assert_eq!(history.len(), MAX_CONVERSATION_SCAN_RECORDS + 1);
    assert_eq!(
        history.last().map(|entry| entry.entry_id.as_str()),
        Some("entry-4096")
    );
    let metrics = take_history_index_metrics_for_test().expect("index metrics are recorded");
    let (memory_limit, scratch_per_entry, work_reserve, scratch_limit) =
        history_index_limits_for_test(metrics.entries).expect("scratch limit is representable");
    assert_eq!(metrics.entries, (MAX_CONVERSATION_SCAN_RECORDS + 1) as u64);
    assert_eq!(metrics.scratch_limit, scratch_limit);
    assert_eq!(
        scratch_limit,
        metrics
            .entries
            .checked_mul(scratch_per_entry)
            .and_then(|bytes| bytes.checked_add(work_reserve))
            .expect("boundary scratch limit is representable")
    );
    assert!(metrics.scratch_peak <= metrics.scratch_limit);
    assert!(metrics.memory_bound <= memory_limit);
    assert!(metrics.work <= metrics.work_limit);
    assert_history_validation_scratch_is_empty(&workspace);
}

#[test]
fn conversation_history_rejects_graph_corruption_across_a_scan_quantum() {
    let workspace = empty_workspace("conversation-history-cross-quantum-corruption");
    create_terminal_review_run(&workspace);
    let valid_prefix = (0..=MAX_CONVERSATION_SCAN_RECORDS)
        .map(|index| {
            let parent = (index > 0).then(|| format!("entry-{:04}", index - 1));
            entry(
                &format!("entry-{index:04}"),
                parent.as_deref(),
                "review-1",
                1,
            )
        })
        .collect::<Vec<_>>();
    for (name, invalid) in [
        (
            "duplicate",
            entry("entry-0000", Some("entry-4096"), "review-1", 1),
        ),
        (
            "missing-parent",
            entry("entry-4097", Some("missing"), "review-1", 1),
        ),
        (
            "later-parent",
            entry("entry-4097", Some("entry-4098"), "review-1", 1),
        ),
    ] {
        let mut records = valid_prefix.clone();
        records.push(invalid);
        if name == "later-parent" {
            records.push(entry("entry-4098", Some("entry-4096"), "review-1", 1));
        }
        write_history_records(&workspace, "review", &records);
        let error = read_conversation_history(&workspace, "review")
            .expect_err("cross-quantum graph corruption must fail");
        assert!(
            error.to_string().contains(if name == "duplicate" {
                "duplicated"
            } else {
                "does not precede"
            }),
            "{name}: {error}"
        );
        assert_history_validation_scratch_is_empty(&workspace);
    }
}

#[test]
fn conversation_history_fails_space_admission_before_continuation_reservation() {
    let workspace = empty_workspace("conversation-history-space-admission");
    create_terminal_review_run(&workspace);
    write_history_records(&workspace, "review", [entry("root", None, "review-1", 1)]);
    set_history_index_available_space_for_test(Some(0));
    let result = reserve_conversation_continuation(&workspace, "review", None);
    set_history_index_available_space_for_test(None);

    let error = match result {
        Ok(reservation) => {
            reservation
                .release()
                .expect("unexpected reservation releases");
            panic!("insufficient scratch space must fail before reservation")
        }
        Err(error) => error,
    };
    assert!(error.to_string().contains("insufficient space"));
    assert_eq!(
        fs::read_dir(crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs"))
            .expect("run directory reads")
            .count(),
        1
    );
}

#[test]
fn conversation_history_cleans_stale_scratch_and_preserves_a_replacement() {
    let (workspace, stale) = stale_history_validation_scratch("conversation-history-stale-cleanup");
    read_conversation_history(&workspace, "review").expect("stale scratch is recovered");
    assert!(!stale.exists());
    assert_history_validation_scratch_is_empty(&workspace);

    let replaced = abandon_history_index_scratch_for_test(&workspace, "review")
        .expect("second crash-stale scratch is created");
    fs::remove_dir_all(&replaced).expect("stale identity is removed by the fixture");
    fs::write(&replaced, b"foreign replacement").expect("replacement bytes write");
    let error = read_conversation_history(&workspace, "review")
        .expect_err("a replaced stale scratch identity must fail closed");
    assert!(error.to_string().contains("scratch"));
    assert_eq!(
        fs::read(&replaced).expect("replacement remains"),
        b"foreign replacement"
    );
}

#[test]
fn history_scratch_retries_partial_initialization_and_final_cleanup() {
    for (label, fault) in [
        (
            "after-directory",
            HistoryScratchFault::InitializationAfterDirectory,
        ),
        (
            "after-marker",
            HistoryScratchFault::InitializationAfterMarker,
        ),
    ] {
        let workspace = empty_workspace(&format!("history-scratch-initialization-{label}"));
        set_history_scratch_fault_for_test(Some(fault));
        complete_history_index_scratch_for_test(&workspace, "review")
            .expect_err("partial initialization fails");
        complete_history_index_scratch_for_test(&workspace, "review")
            .expect("partial initialization is recoverable");
        assert_history_validation_scratch_is_empty(&workspace);
    }

    for (label, fault) in [
        ("after-lease", HistoryScratchFault::CleanupAfterLeaseRemoval),
        (
            "after-marker",
            HistoryScratchFault::CleanupAfterMarkerRemoval,
        ),
    ] {
        let workspace = empty_workspace(&format!("history-scratch-final-cleanup-{label}"));
        abandon_history_index_scratch_for_test(&workspace, "review")
            .expect("stale scratch is created");
        set_history_scratch_fault_for_test(Some(fault));
        complete_history_index_scratch_for_test(&workspace, "review")
            .expect_err("partial final cleanup fails");
        complete_history_index_scratch_for_test(&workspace, "review")
            .expect("partial final cleanup is recoverable");
        assert_history_validation_scratch_is_empty(&workspace);
    }
}

#[test]
fn history_scratch_cleanup_validates_inventory_before_effects() {
    for count in [1, 32] {
        let workspace = empty_workspace(&format!("history-scratch-bounded-cleanup-{count}"));
        let stale = abandon_history_index_scratches_for_test(&workspace, "review", count)
            .expect("crash-stale scratch inventory is created");

        complete_history_index_scratch_for_test(&workspace, "review")
            .expect("valid stale scratch is reclaimed");

        assert!(
            stale.iter().all(|path| !path.exists()),
            "all valid stale scratch is reclaimed"
        );
        assert_history_validation_scratch_is_empty(&workspace);
    }

    let workspace = empty_workspace("history-scratch-validate-before-effects");
    let stale = abandon_history_index_scratches_for_test(&workspace, "review", 8)
        .expect("mixed stale scratch inventory is created");
    let foreign = stale
        .last()
        .expect("unsafe control exists")
        .join("foreign.bin");
    fs::write(&foreign, b"foreign scratch bytes").expect("unsafe control writes");

    let error = complete_history_index_scratch_for_test(&workspace, "review")
        .expect_err("unsafe scratch prevents every cleanup effect");

    assert!(error.to_string().contains("scratch contains foreign bytes"));
    assert!(
        stale.iter().all(|path| path.is_dir()),
        "validation failure preserves every stale scratch directory"
    );
    assert_eq!(
        fs::read(&foreign).expect("unsafe control remains"),
        b"foreign scratch bytes"
    );
}

#[test]
fn history_scratch_cleanup_validates_members_before_effects() {
    for count in [1, 64] {
        let workspace = empty_workspace(&format!("history-scratch-bounded-members-{count}"));
        let scratch = abandon_history_index_scratch_for_test(&workspace, "review")
            .expect("crash-stale scratch is created");
        for index in 0..count {
            fs::write(
                scratch.join(format!("g000-r{index:016}.bin")),
                b"sorted run",
            )
            .expect("canonical stale run writes");
        }

        complete_history_index_scratch_for_test(&workspace, "review")
            .expect("valid stale scratch is reclaimed");

        assert!(!scratch.exists(), "all valid stale members are reclaimed");
        assert_history_validation_scratch_is_empty(&workspace);
    }

    let workspace = empty_workspace("history-scratch-member-validate-before-effects");
    let stale = abandon_history_index_scratches_for_test(&workspace, "review", 2)
        .expect("peer stale scratches are created");
    for (scratch_index, scratch) in stale.iter().enumerate() {
        for run_index in 0..4 {
            fs::write(
                scratch.join(format!("g000-r{run_index:016}.bin")),
                format!("sorted run {scratch_index} {run_index}"),
            )
            .expect("canonical stale run writes");
        }
    }
    fs::write(
        stale
            .last()
            .expect("unsafe control exists")
            .join("foreign.bin"),
        b"foreign scratch bytes",
    )
    .expect("unsafe control writes");
    let before = file_tree_bytes(&history_validation_root(&workspace));

    let error = complete_history_index_scratch_for_test(&workspace, "review")
        .expect_err("unsafe member prevents every cleanup effect");

    assert!(error.to_string().contains("scratch contains foreign bytes"));
    assert_eq!(
        file_tree_bytes(&history_validation_root(&workspace)),
        before,
        "first-pass validation failure preserves every scratch byte"
    );
}

#[test]
fn history_scratch_publication_excludes_a_peer_stale_sweep() {
    assert_history_scratch_root_serializes(
        "history-scratch-publication-serialization",
        HistoryScratchStage::DirectoryCreated,
    );
}

#[test]
fn history_scratch_removal_excludes_a_peer_stale_sweep() {
    assert_history_scratch_root_serializes(
        "history-scratch-removal-serialization",
        HistoryScratchStage::UnlockedForRemoval,
    );
}

#[test]
fn active_history_scratch_member_removal_does_not_race_a_peer_sweep() {
    #[derive(Clone, Copy)]
    enum PeerEvent {
        ActiveScratchSkipped,
        MutableMemberInspected,
    }

    fn prepare_history(workspace: &Path, conversation_id: &str) {
        let run_ids = [
            format!("{conversation_id}-1"),
            format!("{conversation_id}-2"),
        ];
        for run_id in &run_ids {
            create_conversation_run(
                workspace,
                conversation_id,
                run_id,
                "review-flow",
                REGISTRY_HASH,
                FLOW_HASH,
            )
            .expect("conversation run is created");
            write_terminal_run(workspace, conversation_id, run_id);
        }
        let records = (0..5)
            .map(|ordinal| {
                let reverse = 4 - ordinal;
                let entry_id = format!("entry-{reverse:04}");
                let parent_entry_id = (ordinal > 0).then(|| format!("entry-{:04}", reverse + 1));
                entry(
                    &entry_id,
                    parent_entry_id.as_deref(),
                    &run_ids[ordinal % 2],
                    1,
                )
            })
            .collect::<Vec<_>>();
        write_history_records(workspace, conversation_id, &records);
    }

    let workspace = empty_workspace("active-history-scratch-member-removal-race");
    prepare_history(&workspace, "review-a");
    prepare_history(&workspace, "review-b");

    let owner_workspace = workspace.to_path_buf();
    let (owner_member, owner_member_rx) = mpsc::channel();
    let (release_owner, release_owner_rx) = mpsc::channel();
    let (owner_removed, owner_removed_rx) = mpsc::channel();
    let owner = thread::spawn(move || {
        let mut selected = None;
        with_history_scratch_member_observer_for_test(
            move |stage, member| match stage {
                HistoryScratchMemberStage::BeforeRemoval if selected.is_none() => {
                    selected = Some(member.to_owned());
                    owner_member
                        .send(member.to_owned())
                        .expect("owner member reports");
                    release_owner_rx.recv().expect("owner removal releases");
                }
                HistoryScratchMemberStage::AfterRemoval if selected.as_deref() == Some(member) => {
                    owner_removed.send(()).expect("owner removal reports");
                }
                _ => {}
            },
            || {
                set_history_index_sort_record_limit_for_test(Some(2));
                let result = read_conversation_history(&owner_workspace, "review-a");
                set_history_index_sort_record_limit_for_test(None);
                result
            },
        )
    });
    let selected = owner_member_rx
        .recv()
        .expect("owner pauses before removing a mutable scratch member");

    let peer_workspace = workspace.to_path_buf();
    let (peer_event, peer_event_rx) = mpsc::channel();
    let peer_stage_event = peer_event.clone();
    let (release_peer, release_peer_rx) = mpsc::channel();
    let peer_selected = selected.clone();
    let peer = thread::spawn(move || {
        let mut inspected = false;
        with_history_scratch_stage_observer_for_test(
            move |stage| {
                if stage == HistoryScratchStage::ActiveScratchSkipped {
                    peer_stage_event
                        .send(PeerEvent::ActiveScratchSkipped)
                        .expect("active scratch skip reports");
                }
            },
            || {
                with_history_scratch_member_observer_for_test(
                    move |stage, member| {
                        if stage == HistoryScratchMemberStage::BeforeInspection
                            && member == peer_selected
                            && !inspected
                        {
                            inspected = true;
                            peer_event
                                .send(PeerEvent::MutableMemberInspected)
                                .expect("peer inspection reports");
                            release_peer_rx.recv().expect("peer inspection releases");
                        }
                    },
                    || {
                        set_history_index_sort_record_limit_for_test(Some(2));
                        let result = read_conversation_history(&peer_workspace, "review-b");
                        set_history_index_sort_record_limit_for_test(None);
                        result
                    },
                )
            },
        )
    });

    match peer_event_rx
        .recv()
        .expect("peer sweep reaches active scratch")
    {
        PeerEvent::ActiveScratchSkipped => {
            let selected_exists = fs::read_dir(history_validation_root(&workspace))
                .expect("history scratch root reads")
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .any(|path| path.join(&selected).is_file());
            assert!(
                selected_exists,
                "peer sweep leaves the active scratch untouched"
            );
            release_owner.send(()).expect("owner removal releases");
        }
        PeerEvent::MutableMemberInspected => {
            release_owner.send(()).expect("owner removal releases");
            owner_removed_rx
                .recv()
                .expect("owner removes the enumerated member normally");
            release_peer.send(()).expect("peer inspection releases");
        }
    }

    owner
        .join()
        .expect("owner thread joins")
        .expect("owner history build succeeds");
    peer.join()
        .expect("peer thread joins")
        .expect("peer history build succeeds");
    assert_history_validation_scratch_is_empty(&workspace);
}

fn assert_history_scratch_root_serializes(label: &str, owner_boundary: HistoryScratchStage) {
    let workspace = empty_workspace(label);
    let workspace_path = workspace.to_path_buf();
    let (owner_staged, owner_staged_rx) = std::sync::mpsc::channel();
    let (release_owner, release_owner_rx) = std::sync::mpsc::channel();
    let owner = thread::spawn(move || {
        with_history_scratch_stage_observer_for_test(
            move |stage| {
                if stage == owner_boundary {
                    owner_staged.send(()).expect("owner boundary reports");
                    release_owner_rx.recv().expect("owner boundary releases");
                }
            },
            || complete_history_index_scratch_for_test(&workspace_path, "review-a"),
        )
    });
    owner_staged_rx
        .recv()
        .expect("owner pauses at the selected root boundary");

    let peer_workspace = workspace.to_path_buf();
    let (peer_stage, peer_stage_rx) = std::sync::mpsc::channel();
    let peer = thread::spawn(move || {
        with_history_scratch_stage_observer_for_test(
            move |stage| {
                if matches!(
                    stage,
                    HistoryScratchStage::RootLeaseContended | HistoryScratchStage::StaleSweep
                ) {
                    peer_stage.send(stage).expect("peer stage reports");
                }
            },
            || complete_history_index_scratch_for_test(&peer_workspace, "review-b"),
        )
    });
    let observed = peer_stage_rx
        .recv()
        .expect("peer reaches the serialized root boundary");
    release_owner.send(()).expect("owner boundary releases");
    let owner_result = owner.join().expect("owner joins");
    let peer_result = peer.join().expect("peer joins");

    assert_eq!(observed, HistoryScratchStage::RootLeaseContended);
    owner_result.expect("owner scratch lifecycle completes");
    peer_result.expect("peer scratch lifecycle completes");
    assert_history_validation_scratch_is_empty(&workspace);
}

#[test]
fn conversation_history_preserves_stale_scratch_with_a_foreign_marker_identity() {
    let (workspace, stale) =
        stale_history_validation_scratch("conversation-history-foreign-scratch-marker");
    let marker_path = stale.join("marker.json");
    let mut marker = serde_json::from_slice::<serde_json::Value>(
        &fs::read(&marker_path).expect("scratch marker reads"),
    )
    .expect("scratch marker parses");
    let inode = marker["inode"]
        .as_u64()
        .expect("scratch marker inode is an unsigned integer");
    marker["inode"] = serde_json::json!(inode ^ 1);
    let foreign_marker = format!(
        "{}\n",
        serde_json::to_string(&marker).expect("foreign marker serializes")
    );
    fs::write(&marker_path, &foreign_marker).expect("foreign marker writes");
    let lease_path = stale.join("lease");
    let lease_bytes = fs::read(&lease_path).expect("scratch lease reads");

    let error = read_conversation_history(&workspace, "review")
        .expect_err("a foreign marker identity must fail closed");
    assert!(error.to_string().contains("scratch identity is invalid"));
    assert!(stale.is_dir());
    assert_eq!(
        fs::read(&marker_path).expect("foreign marker remains"),
        foreign_marker.as_bytes()
    );
    assert_eq!(
        fs::read(&lease_path).expect("scratch lease remains"),
        lease_bytes
    );
    assert_eq!(
        fs::read_dir(&stale).expect("foreign scratch reads").count(),
        2
    );
}

#[test]
fn conversation_history_preserves_stale_scratch_with_a_foreign_marker_schema() {
    let (workspace, stale) =
        stale_history_validation_scratch("conversation-history-foreign-scratch-schema");
    let marker_path = stale.join("marker.json");
    let mut marker = serde_json::from_slice::<serde_json::Value>(
        &fs::read(&marker_path).expect("scratch marker reads"),
    )
    .expect("scratch marker parses");
    marker["schema"] = serde_json::json!("foreign-history-index-schema-v0");
    let foreign_marker = format!(
        "{}\n",
        serde_json::to_string(&marker).expect("foreign marker serializes")
    );
    fs::write(&marker_path, &foreign_marker).expect("foreign marker writes");

    let error = read_conversation_history(&workspace, "review")
        .expect_err("a foreign marker schema must fail closed");
    assert!(
        error
            .to_string()
            .contains("scratch marker schema is invalid")
    );
    assert!(stale.is_dir());
    assert_eq!(
        fs::read(&marker_path).expect("foreign marker remains"),
        foreign_marker.as_bytes()
    );
    assert_eq!(
        fs::read_dir(&stale).expect("foreign scratch reads").count(),
        2
    );
}

#[test]
fn conversation_history_preserves_stale_scratch_with_a_foreign_member() {
    let (workspace, stale) =
        stale_history_validation_scratch("conversation-history-foreign-scratch-member");
    let foreign_path = stale.join("foreign.bin");
    let foreign_bytes = b"foreign scratch bytes";
    fs::write(&foreign_path, foreign_bytes).expect("foreign scratch member writes");

    let error = read_conversation_history(&workspace, "review")
        .expect_err("a foreign scratch member must fail closed");
    assert!(error.to_string().contains("scratch contains foreign bytes"));
    assert!(stale.is_dir());
    assert_eq!(
        fs::read(&foreign_path).expect("foreign scratch member remains"),
        foreign_bytes
    );
    assert_eq!(
        fs::read_dir(&stale).expect("foreign scratch reads").count(),
        3
    );
}

#[test]
fn malformed_stale_scratch_markers_fail_closed_and_remain_unchanged() {
    type MarkerCase = (&'static str, fn(Vec<u8>) -> Vec<u8>, &'static str);
    let cases: [MarkerCase; 3] = [
        (
            "invalid-utf8",
            |_| vec![0xff, b'\n'],
            "scratch marker is not UTF-8",
        ),
        (
            "missing-lf",
            |mut marker| {
                assert_eq!(marker.pop(), Some(b'\n'));
                marker
            },
            "scratch marker framing is invalid",
        ),
        (
            "invalid-json",
            |_| b"{}\n".to_vec(),
            "scratch marker is invalid",
        ),
    ];

    for (name, corrupt, expected) in cases {
        let (workspace, stale) = stale_history_validation_scratch(&format!(
            "conversation-history-malformed-scratch-marker-{name}"
        ));
        let marker_path = stale.join("marker.json");
        let marker = corrupt(fs::read(&marker_path).expect("scratch marker reads"));
        fs::write(&marker_path, &marker).expect("malformed marker writes");
        let lease_path = stale.join("lease");
        let lease = fs::read(&lease_path).expect("scratch lease reads");

        let error = read_conversation_history(&workspace, "review")
            .expect_err("a malformed scratch marker must fail closed");
        assert!(error.to_string().contains(expected), "{name}: {error}");
        assert!(stale.is_dir(), "{name}");
        assert_eq!(
            fs::read(&marker_path).expect("malformed marker remains"),
            marker,
            "{name}"
        );
        assert_eq!(
            fs::read(&lease_path).expect("scratch lease remains"),
            lease,
            "{name}"
        );
        assert_eq!(
            fs::read_dir(&stale)
                .expect("malformed scratch reads")
                .count(),
            2,
            "{name}"
        );
    }
}

#[test]
fn conversation_history_rejects_unsafe_private_validation_scratch() {
    let workspace = empty_workspace("conversation-history-unsafe-scratch");
    create_review_run(&workspace);
    write_history_records(&workspace, "review", [entry("root", None, "review-1", 1)]);
    SessionOwnershipLease::ensure_store_available(&workspace)
        .expect("private coordinator is created");
    let unsafe_scratch = history_validation_root(&workspace);
    fs::remove_dir(&unsafe_scratch).expect("empty validation directory is removed by the fixture");
    fs::write(&unsafe_scratch, b"foreign scratch bytes").expect("unsafe scratch fixture writes");

    let error = read_conversation_history(&workspace, "review")
        .expect_err("unsafe validation scratch must fail closed");

    assert!(
        error
            .to_string()
            .contains("conversation-history-validation-v1")
    );
    assert_eq!(
        fs::read(&unsafe_scratch).expect("foreign scratch remains"),
        b"foreign scratch bytes"
    );
}
