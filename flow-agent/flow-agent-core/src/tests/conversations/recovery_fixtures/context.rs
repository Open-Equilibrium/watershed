use super::super::create_review_run;
use super::events::{message_completed_event, message_delta_event, message_prefix_events};
use crate::{
    runtime::{
        context::{ContextManifest, ContextManifestCheckpoint, ContextObject},
        conversations::{RunObjectStore, canonical_json},
        digest::sha256_hex,
    },
    tests::{helpers::empty_workspace, test_support::TempWorkspace},
};
use proto::EventEnvelope;
use std::fs;

pub(in crate::tests::conversations) fn context_only_recovery_fixture(
    label: &str,
    extra_context: bool,
) -> (
    TempWorkspace,
    Vec<EventEnvelope>,
    EventEnvelope,
    ContextManifestCheckpoint,
) {
    let workspace = empty_workspace(label);
    create_review_run(&workspace);
    let mut prefix = message_prefix_events().to_vec();
    prefix.push(message_delta_event());
    let canonical_prefix = prefix
        .iter()
        .map(|event| event.canonical_jsonl().expect("prefix event canonicalizes"))
        .collect::<String>();
    let run = crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review-1");
    fs::write(run.join("events.jsonl"), canonical_prefix).expect("event prefix writes");

    let checkpoint = context_checkpoint();
    RunObjectStore::open(&workspace, "review", "review-1")
        .and_then(|store| store.persist(&checkpoint.objects))
        .expect("context object is durable");
    let contexts = if extra_context {
        checkpoint.manifest.line.repeat(2)
    } else {
        checkpoint.manifest.line.clone()
    };
    fs::write(run.join("contexts.jsonl"), contexts).expect("context prefix writes");

    (workspace, prefix, message_completed_event(), checkpoint)
}

pub(in crate::tests::conversations) fn context_checkpoint() -> ContextManifestCheckpoint {
    let bytes = b"durable context object\n".to_vec();
    let object = ContextObject {
        digest: sha256_hex(&bytes),
        bytes,
    };
    let manifest_value = serde_json::json!({
        "checkpoint": 1,
        "object_uri": format!("session-object:sha256:{}", object.digest),
    });
    ContextManifestCheckpoint {
        manifest: ContextManifest {
            line: format!(
                "{}\n",
                canonical_json(&manifest_value).expect("context manifest canonicalizes")
            ),
        },
        objects: vec![object],
        ordinal: 1,
    }
}

pub(in crate::tests::conversations) fn context_checkpoint_with_exact_canonical_bytes(
    canonical_bytes: usize,
) -> ContextManifestCheckpoint {
    let bytes = b"durable padded context object\n".to_vec();
    let object = ContextObject {
        digest: sha256_hex(&bytes),
        bytes,
    };
    let build = |padding_bytes| {
        serde_json::json!({
            "checkpoint": 1,
            "object_uri": format!("session-object:sha256:{}", object.digest),
            "padding": "x".repeat(padding_bytes),
        })
    };
    let empty = format!(
        "{}\n",
        canonical_json(&build(0)).expect("unpadded context manifest canonicalizes")
    );
    assert!(canonical_bytes >= empty.len());
    let line = format!(
        "{}\n",
        canonical_json(&build(canonical_bytes - empty.len()))
            .expect("padded context manifest canonicalizes")
    );
    assert_eq!(line.len(), canonical_bytes);
    ContextManifestCheckpoint {
        manifest: ContextManifest { line },
        objects: vec![object],
        ordinal: 1,
    }
}
