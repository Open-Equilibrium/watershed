use super::super::helpers::{empty_workspace, reserve_session_log};
use crate::runtime::{
    conversations::read_anchored_jsonl,
    digest::sha256_hex,
    fs_guards::{
        for_each_segmented_jsonl_line, open_runtime_dir, read_anchored_file_with_limit,
        segmented_jsonl_files, segmented_jsonl_path,
        with_segmented_jsonl_discovery_metrics_for_test,
    },
    segmented_appender::SessionLogAppender,
    types::{
        CONTEXT_MANIFEST_STREAM_LIMITS, EVENT_STREAM_LIMITS, MAX_SESSION_OBJECT_BYTES,
        RuntimeError, SessionStreamLimits,
    },
};
use std::fs;

#[test]
fn conversation_segment_discovery_has_constant_inventory_memory_and_one_pass() {
    let workspace = empty_workspace("conversation-segment-discovery-budget");
    let reservation =
        reserve_session_log(&workspace, "segmentbudget001").expect("session reserved");
    let line = b"{}\n";
    fs::write(
        reservation.session_path.diagnostic_path(),
        line.repeat(crate::runtime::conversations::MAX_CONVERSATION_SCAN_RECORDS + 1),
    )
    .expect("multi-quantum base segment writes");
    let second = segmented_jsonl_path(&reservation.session_path, 2).expect("second segment path");
    fs::write(second.diagnostic_path(), line).expect("second segment writes");

    let (result, metrics) = with_segmented_jsonl_discovery_metrics_for_test(|| {
        read_anchored_jsonl::<serde_json::Value>(&reservation.session_path)
    });

    assert_eq!(
        result.expect("segmented conversation stream reads").len(),
        crate::runtime::conversations::MAX_CONVERSATION_SCAN_RECORDS + 2
    );
    assert_eq!(metrics.passes, 1, "inventory is discovered once per read");
    assert!(
        metrics.retained_path_peak <= 1,
        "inventory retains at most one discovered path, observed {}",
        metrics.retained_path_peak
    );
    reservation.rollback().expect("reservation rolls back");
}
#[test]
fn segmented_stream_consumers_reject_high_ordinals_and_rollback_preserves_foreign_siblings() {
    for (label, context) in [("event", false), ("context", true)] {
        let workspace = empty_workspace(&format!("high-ordinal-{label}"));
        let reservation =
            reserve_session_log(&workspace, "segmenthigh001").expect("session reserved");
        let base = if context {
            &reservation.context_path
        } else {
            &reservation.session_path
        };
        let limits = if context {
            CONTEXT_MANIFEST_STREAM_LIMITS
        } else {
            EVENT_STREAM_LIMITS
        };
        let high = segmented_jsonl_path(base, limits.max_segments + 1)
            .expect("high segment path resolves");
        fs::write(high.diagnostic_path(), b"\n").expect("high segment fixture writes");

        let results = if context {
            vec![
                for_each_segmented_jsonl_line(base, CONTEXT_MANIFEST_STREAM_LIMITS, |_| Ok(()))
                    .map(|_| ()),
            ]
        } else {
            vec![SessionLogAppender::open(base).map(|_| ())]
        };
        for result in results {
            let err = result.expect_err("high ordinal must not be omitted");
            assert!(err.to_string().contains("segment count"), "{label}: {err}");
        }

        let alias = segmented_jsonl_path(base, 3)
            .expect("alias path resolves")
            .diagnostic_path()
            .with_extension("JSONL");
        fs::write(&alias, b"\n").expect("case-aliased segment fixture writes");
        reservation.rollback().expect("reservation rolls back");
        assert_eq!(
            fs::read(high.diagnostic_path()).expect("foreign high segment remains"),
            b"\n",
            "{label} foreign high segment bytes must be preserved"
        );
        assert_eq!(
            fs::read(&alias).expect("foreign alias remains"),
            b"\n",
            "{label} foreign alias bytes must be preserved"
        );
        assert_eq!(
            fs::read(base.diagnostic_path()).expect("owned base orphan remains readable"),
            b"",
            "{label} owned base must remain an inventory-visible empty orphan"
        );
    }
}

#[cfg(any(unix, windows))]
#[test]
fn rotated_stream_segments_and_objects_reject_hardlinks() {
    for kind in ["event", "context", "object"] {
        let workspace = empty_workspace(&format!("hardlinked-{kind}-read"));
        let reservation =
            reserve_session_log(&workspace, "hardlinkedread001").expect("session reserved");
        let target = workspace.join("hardlink-target");
        fs::write(&target, b"linked bytes\n").expect("hardlink target written");

        let result = match kind {
            "event" => {
                fs::write(reservation.session_path.diagnostic_path(), b"{}\n")
                    .expect("base event segment completed");
                let segment = segmented_jsonl_path(&reservation.session_path, 2)
                    .expect("segment path resolves");
                fs::hard_link(&target, segment.diagnostic_path()).expect("event segment linked");
                for_each_segmented_jsonl_line(
                    &reservation.session_path,
                    EVENT_STREAM_LIMITS,
                    |_| Ok(()),
                )
                .map(|_| ())
            }
            "context" => {
                fs::write(reservation.context_path.diagnostic_path(), b"{}\n")
                    .expect("base context segment completed");
                let segment = segmented_jsonl_path(&reservation.context_path, 2)
                    .expect("segment path resolves");
                fs::hard_link(&target, segment.diagnostic_path()).expect("context segment linked");
                for_each_segmented_jsonl_line(
                    &reservation.context_path,
                    CONTEXT_MANIFEST_STREAM_LIMITS,
                    |_| Ok(()),
                )
                .map(|_| ())
            }
            "object" => {
                let sessions = open_runtime_dir(&workspace, "sessions")
                    .expect("session object directory opens")
                    .expect("session object directory exists");
                let digest = sha256_hex(b"linked bytes\n");
                let object =
                    sessions.file(format!("{}.object.sha256-{digest}", reservation.session_id));
                fs::hard_link(&target, object.diagnostic_path()).expect("object linked");
                read_anchored_file_with_limit(&object, MAX_SESSION_OBJECT_BYTES).map(|_| ())
            }
            _ => unreachable!(),
        };
        let err = result.expect_err("hard-linked read must fail");
        assert!(
            matches!(err, RuntimeError::Protocol(message) if message.contains("hard-linked")),
            "{kind} hardlink was not rejected"
        );
        reservation.rollback().expect("reservation rolls back");
        drop(reservation);
        fs::remove_dir_all(workspace).expect("workspace removed");
    }
}

#[test]
fn segmented_stream_rejects_invalid_ordinal_layouts() {
    for (label, ordinals, expected) in [
        (
            "event-segment-invalid-ordinal",
            vec![0],
            "invalid segmented JSONL ordinal",
        ),
        (
            "event-segment-base-ordinal",
            vec![1],
            "invalid segmented JSONL ordinal",
        ),
        ("event-segment-gap", vec![3], "non-contiguous"),
        (
            "event-segment-count",
            (2..=EVENT_STREAM_LIMITS.max_segments + 1).collect::<Vec<_>>(),
            "segment count",
        ),
    ] {
        let workspace = empty_workspace(label);
        let reservation =
            reserve_session_log(&workspace, "segmentinvalid001").expect("session reserved");
        for ordinal in ordinals {
            let segment = reservation
                .session_path
                .parent
                .file(format!("segmentinvalid001.{ordinal:06}.jsonl"));
            fs::write(segment.diagnostic_path(), b"\n").expect("invalid segment fixture writes");
        }

        let err = segmented_jsonl_files(&reservation.session_path, EVENT_STREAM_LIMITS)
            .expect_err("invalid segment layout is rejected");
        assert!(err.to_string().contains(expected), "{err}");
        reservation.rollback().expect("reservation rolls back");
        drop(reservation);
        fs::remove_dir_all(workspace).expect("invalid segment workspace removed");
    }
}

#[test]
fn segmented_stream_rejects_malformed_reserved_names() {
    for suffix in [".00002.jsonl", ".abcdef.jsonl", ".1000000.jsonl"] {
        let workspace = empty_workspace(&format!(
            "segment-malformed-name-{}",
            suffix.replace('.', "-")
        ));
        let reservation =
            reserve_session_log(&workspace, "segmentmalformed001").expect("session reserved");
        let malformed = reservation
            .session_path
            .parent
            .file(format!("segmentmalformed001{suffix}"));
        fs::write(malformed.diagnostic_path(), b"omitted\n")
            .expect("malformed reserved-name fixture writes");

        let err = segmented_jsonl_files(&reservation.session_path, EVENT_STREAM_LIMITS)
            .expect_err("malformed reserved segment name is rejected");
        assert!(err.to_string().contains("malformed"), "{suffix}: {err}");
        reservation.rollback().expect("reservation rolls back");
    }
}

#[test]
fn segmented_stream_rejects_case_aliased_names() {
    for base_alias in [true, false] {
        let workspace = empty_workspace(&format!("segment-case-alias-{base_alias}"));
        let reservation =
            reserve_session_log(&workspace, "segmentcase001").expect("session reserved");
        let canonical = if base_alias {
            reservation.session_path.clone()
        } else {
            segmented_jsonl_path(&reservation.session_path, 2).expect("segment path")
        };
        let alias = canonical.diagnostic_path().with_file_name(
            canonical
                .diagnostic_path()
                .file_name()
                .expect("stream name")
                .to_string_lossy()
                .to_ascii_uppercase(),
        );
        if base_alias && cfg!(any(windows, target_os = "macos")) {
            fs::rename(canonical.diagnostic_path(), &alias).expect("case-aliased base renamed");
        } else {
            fs::write(&alias, b"\n").expect("case-aliased stream file written");
        }

        let err = segmented_jsonl_files(&reservation.session_path, EVENT_STREAM_LIMITS)
            .expect_err("case-aliased stream file must be rejected");
        assert!(
            err.to_string().contains("non-canonical"),
            "base_alias={base_alias}: {err}"
        );
        reservation.rollback().expect("reservation rolls back");
    }
}

#[test]
fn segmented_stream_discovers_mixed_case_canonical_segments() {
    let workspace = empty_workspace("segment-mixed-case-canonical");
    let reservation =
        reserve_session_log(&workspace, "segmentmixedcase001").expect("session reserved");
    let base = reservation.session_path.parent.file("Events.jsonl");
    fs::write(base.diagnostic_path(), b"one\n").expect("mixed-case base writes");
    let second = segmented_jsonl_path(&base, 2).expect("mixed-case segment path resolves");
    fs::write(second.diagnostic_path(), b"two\n").expect("mixed-case segment writes");

    let files = segmented_jsonl_files(&base, EVENT_STREAM_LIMITS)
        .expect("mixed-case canonical segments are discovered");
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].diagnostic_path(), base.diagnostic_path());
    assert_eq!(files[1].diagnostic_path(), second.diagnostic_path());
    let mut lines = Vec::new();
    for_each_segmented_jsonl_line(&base, EVENT_STREAM_LIMITS, |line| {
        lines.push(line.to_owned());
        Ok(())
    })
    .expect("mixed-case canonical segments are traversed");
    assert_eq!(lines, ["one\n", "two\n"]);
}

#[test]
fn segmented_stream_rejects_non_ascii_stems() {
    let workspace = empty_workspace("segment-non-ascii-stem");
    let reservation =
        reserve_session_log(&workspace, "segmentunicode001").expect("session reserved");
    let base = reservation.session_path.parent.file("Ävents.jsonl");
    fs::write(base.diagnostic_path(), b"one\n").expect("non-ASCII base writes");

    let err = segmented_jsonl_files(&base, EVENT_STREAM_LIMITS)
        .expect_err("non-ASCII segmented stream stem is rejected");
    assert!(err.to_string().contains("ASCII"), "{err}");

    fs::remove_file(base.diagnostic_path()).expect("non-ASCII base removes");
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn segmented_stream_callbacks_do_not_observe_bytes_beyond_total_limit() {
    let workspace = empty_workspace("segment-callback-total-limit");
    let reservation =
        reserve_session_log(&workspace, "segmentcallback001").expect("session reserved");
    fs::write(reservation.session_path.diagnostic_path(), b"one\n").expect("base segment written");
    let second = segmented_jsonl_path(&reservation.session_path, 2).expect("segment path resolves");
    fs::write(second.diagnostic_path(), b"two\n").expect("second segment written");
    let limits = SessionStreamLimits {
        max_segments: 2,
        max_total_bytes: 7,
    };
    let mut visited = Vec::new();

    let err = for_each_segmented_jsonl_line(&reservation.session_path, limits, |line| {
        visited.push(line.to_owned());
        Ok(())
    })
    .expect_err("cumulative stream limit is enforced before callbacks");

    assert!(err.to_string().contains("exceeds max"), "{err}");
    assert_eq!(visited, vec!["one\n"]);
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn segmented_stream_callbacks_reject_unterminated_non_final_segment() {
    let workspace = empty_workspace("segment-callback-non-final-lf");
    let reservation =
        reserve_session_log(&workspace, "segmentboundary001").expect("session reserved");
    fs::write(reservation.session_path.diagnostic_path(), b"{}")
        .expect("unterminated base segment written");
    let second = segmented_jsonl_path(&reservation.session_path, 2).expect("segment path resolves");
    fs::write(second.diagnostic_path(), b"{}\n").expect("second segment written");

    let mut visited = Vec::new();
    let stream_err =
        for_each_segmented_jsonl_line(&reservation.session_path, EVENT_STREAM_LIMITS, |line| {
            visited.push(line.to_owned());
            Ok(())
        })
        .expect_err("streaming reader rejects the non-final boundary");

    assert!(
        stream_err.to_string().contains("must end with LF"),
        "{stream_err}"
    );
    assert!(visited.is_empty());
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn segmented_stream_callbacks_reject_empty_non_final_segment() {
    let workspace = empty_workspace("segment-callback-empty-non-final");
    let reservation = reserve_session_log(&workspace, "segmentempty001").expect("session reserved");
    fs::write(reservation.session_path.diagnostic_path(), b"").expect("empty base segment written");
    let second = segmented_jsonl_path(&reservation.session_path, 2).expect("segment path resolves");
    fs::write(second.diagnostic_path(), b"{}\n").expect("second segment written");

    let mut visited = Vec::new();
    let stream_err =
        for_each_segmented_jsonl_line(&reservation.session_path, EVENT_STREAM_LIMITS, |line| {
            visited.push(line.to_owned());
            Ok(())
        })
        .expect_err("streaming reader rejects the empty non-final segment");

    assert!(
        stream_err.to_string().contains("must end with LF"),
        "{stream_err}"
    );
    assert!(visited.is_empty());
    reservation.rollback().expect("reservation rolls back");
}
