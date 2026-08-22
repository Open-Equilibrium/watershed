use super::super::super::helpers::{copy_workspace_runtime, empty_workspace};
use super::super::recovery_fixtures::{
    replace_terminal_recovery_snapshot, write_terminal_recovery_fixture,
    write_terminal_recovery_snapshot,
};
use super::super::{
    append_uncertain_provider_intent, create_review_run, create_terminal_review_run,
    write_terminal_run,
};
use crate::runtime::{
    conversations::{
        MAX_CONVERSATION_RECORD_BYTES, append_productive_run_checkpoint, canonical_json,
        reserve_conversation_continuation, reserve_conversation_run_recovery,
    },
    digest::sha256_hex,
    types::MAX_SESSION_METADATA_BYTES,
};
use std::fs::{self, OpenOptions};

#[test]
fn productive_recovery_header_reader_rejects_every_uncommitted_or_foreign_header() {
    let base = empty_workspace("conversation-recovery-header-base");
    create_review_run(&base);
    write_terminal_recovery_snapshot(&base, "review", "review-1");
    write_terminal_run(&base, "review", "review-1");
    let base_run = crate::tests::helpers::workspace_session_dir(&base).join("review/runs/review-1");
    let base_bytes = fs::read(base_run.join("recovery.jsonl")).expect("recovery reads");
    let first_lf = base_bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .expect("header is committed");
    let header = base_bytes[..first_lf].to_vec();
    let mut header_value: serde_json::Value =
        serde_json::from_slice(&header).expect("header parses");
    let terminal = base_bytes[first_lf + 1..]
        .split(|byte| *byte == b'\n')
        .next()
        .expect("terminal record exists")
        .to_vec();

    let mut foreign_address = header_value.clone();
    foreign_address["conversation_id"] = serde_json::json!("other");
    let mut foreign_schema = header_value.clone();
    foreign_schema["schema"] = serde_json::json!("flow-productive-recovery-v9");
    header_value["prior_history_object"] = serde_json::json!("not-an-object-uri");
    let oversized_header = vec![b' '; MAX_CONVERSATION_RECORD_BYTES + 1];
    let cases = [
        ("no-newline", header.clone(), "no committed header"),
        ("empty", b"\n".to_vec(), "invalid framing"),
        ("carriage-return", b"{}\r\n".to_vec(), "invalid framing"),
        (
            "oversized-header",
            [oversized_header.as_slice(), b"\n"].concat(),
            "invalid framing",
        ),
        (
            "noncanonical",
            [b" ".as_slice(), header.as_slice(), b"\n"].concat(),
            "canonical JSON",
        ),
        (
            "terminal-first",
            [terminal.as_slice(), b"\n"].concat(),
            "must begin with a header",
        ),
        (
            "foreign-address",
            format!(
                "{}\n",
                canonical_json(&foreign_address).expect("foreign header canonicalizes")
            )
            .into_bytes(),
            "does not match its addressed run",
        ),
        (
            "foreign-schema",
            format!(
                "{}\n",
                canonical_json(&foreign_schema).expect("foreign header canonicalizes")
            )
            .into_bytes(),
            "unsupported schema",
        ),
        (
            "invalid-history-object",
            format!(
                "{}\n",
                canonical_json(&header_value).expect("invalid header canonicalizes")
            )
            .into_bytes(),
            "object URI is invalid",
        ),
    ];

    for (name, bytes, expected) in cases {
        let workspace = empty_workspace(&format!("conversation-recovery-header-{name}"));
        copy_workspace_runtime(&base, &workspace);
        let run =
            crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review-1");
        fs::write(run.join("recovery.jsonl"), bytes).expect("invalid header fixture writes");
        let error = match reserve_conversation_run_recovery(&workspace, "review", "review-1") {
            Ok(reservation) => {
                reservation
                    .release()
                    .expect("unexpected reservation releases");
                panic!("invalid recovery header must fail closed")
            }
            Err(error) => error,
        };
        assert!(error.to_string().contains(expected), "{name}: {error}");
    }

    let oversized = empty_workspace("conversation-recovery-header-total-limit");
    copy_workspace_runtime(&base, &oversized);
    let oversized_run =
        crate::tests::helpers::workspace_session_dir(&oversized).join("review/runs/review-1");
    OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(oversized_run.join("recovery.jsonl"))
        .expect("recovery opens")
        .set_len(MAX_SESSION_METADATA_BYTES + 1)
        .expect("sparse oversized recovery writes");
    let error = match reserve_conversation_run_recovery(&oversized, "review", "review-1") {
        Ok(reservation) => {
            reservation
                .release()
                .expect("unexpected reservation releases");
            panic!("oversized recovery must fail before reading")
        }
        Err(error) => error,
    };
    assert!(error.to_string().contains("read size"), "{error}");
}

#[test]
fn selected_continuation_rejects_every_incomplete_or_foreign_recovery_snapshot() {
    for case in [
        "hash-mismatch",
        "no-header",
        "foreign-address",
        "no-terminal",
        "missing-history-object",
        "invalid-history-object",
    ] {
        let workspace = empty_workspace(&format!("conversation-selected-recovery-{case}"));
        create_review_run(&workspace);
        write_terminal_recovery_fixture(&workspace, "review", "review-1", "root");
        write_terminal_run(&workspace, "review", "review-1");
        let recovery_path = crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/runs/review-1/recovery.jsonl");
        let mut records = fs::read_to_string(&recovery_path)
            .expect("recovery reads")
            .lines()
            .map(|line| serde_json::from_str(line).expect("recovery record parses"))
            .collect::<Vec<serde_json::Value>>();
        let expected = match case {
            "hash-mismatch" => {
                fs::write(&recovery_path, "{}\n").expect("mismatched recovery writes");
                "does not match its conversation entry hash"
            }
            "no-header" => {
                records.remove(0);
                replace_terminal_recovery_snapshot(&workspace, "review", "review-1", &records);
                "must begin with a header"
            }
            "foreign-address" => {
                records[0]["run_session_id"] = serde_json::json!("other-run");
                replace_terminal_recovery_snapshot(&workspace, "review", "review-1", &records);
                "does not match its addressed run"
            }
            "no-terminal" => {
                records.pop();
                replace_terminal_recovery_snapshot(&workspace, "review", "review-1", &records);
                "has no terminal record"
            }
            "missing-history-object" => {
                records[1]["history_object"] =
                    serde_json::json!(format!("session-object:sha256:{}", "d".repeat(64)));
                replace_terminal_recovery_snapshot(&workspace, "review", "review-1", &records);
                "dddddddddddddddd"
            }
            "invalid-history-object" => {
                let invalid_history = b"{}";
                let digest = sha256_hex(invalid_history);
                fs::write(
                    crate::tests::helpers::workspace_session_dir(&workspace)
                        .join("review/runs/review-1/objects")
                        .join(&digest),
                    invalid_history,
                )
                .expect("invalid history object writes");
                records[1]["history_object"] =
                    serde_json::json!(format!("session-object:sha256:{digest}"));
                replace_terminal_recovery_snapshot(&workspace, "review", "review-1", &records);
                "missing field"
            }
            _ => unreachable!("bounded selected recovery matrix"),
        };

        let error = match reserve_conversation_continuation(&workspace, "review", None) {
            Ok(reservation) => {
                reservation
                    .release()
                    .expect("unexpected reservation releases");
                panic!("incomplete or foreign snapshot must fail closed")
            }
            Err(error) => error,
        };
        assert!(error.to_string().contains(expected), "{case}: {error}");
    }
}

#[test]
fn continuation_refuses_an_uncertain_attempt_in_its_selected_ancestry() {
    let workspace = empty_workspace("conversation-uncertain-continuation");
    create_terminal_review_run(&workspace);
    append_productive_run_checkpoint(
        &workspace,
        "review",
        "review-1",
        None,
        &write_terminal_recovery_snapshot(&workspace, "review", "review-1"),
        2,
        "2026-07-30T12:00:01Z",
    )
    .expect("root checkpoint appends");
    append_uncertain_provider_intent(&workspace);

    let error = match reserve_conversation_continuation(&workspace, "review", None) {
        Ok(_) => panic!("continuation must not redispatch after an uncertain ancestral attempt"),
        Err(error) => error,
    };
    assert_eq!(error.exit_code(), 65);
    assert!(error.to_string().contains("uncertain"));
    assert!(
        !crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/runs/review-2")
            .exists()
    );
}
