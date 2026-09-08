use crate::{
    runtime::{
        context::ContextManifestSourceRecord,
        context_persistence::verify_context_manifest_objects,
        fs_guards::ensure_runtime_dirs,
        types::{MAX_SESSION_OBJECT_TOTAL_BYTES, MAX_SESSION_OBJECTS, RuntimeError},
    },
    tests::helpers::empty_workspace,
};
use std::{collections::BTreeSet, fs};

#[test]
fn context_object_verification_checks_the_aggregate_before_hashing() {
    let workspace = empty_workspace("context-object-aggregate");
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let session_id = "contextaggregate001";
    let digest = "0".repeat(64);
    fs::write(
        sessions
            .file(format!("{session_id}.object.sha256-{digest}"))
            .diagnostic_path(),
        b"x",
    )
    .expect("context object written");
    let source = ContextManifestSourceRecord {
        object_uri: format!("session-object:sha256:{digest}"),
        projection_hash: digest,
        source_id: String::new(),
    };
    let mut verified = BTreeSet::new();
    let mut verified_bytes = MAX_SESSION_OBJECT_TOTAL_BYTES;

    let err = verify_context_manifest_objects(
        &sessions,
        session_id,
        std::slice::from_ref(&source),
        &mut verified,
        &mut verified_bytes,
    )
    .expect_err("aggregate overflow must precede hash validation");
    assert!(err.to_string().contains("object data size"), "{err}");
}

#[test]
fn context_object_verification_bounds_unique_digests_before_opening_the_excess() {
    let workspace = empty_workspace("context-object-count");
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let session_id = "contextcount001";
    let mut verified = (0..MAX_SESSION_OBJECTS)
        .map(|index| format!("{index:064x}"))
        .collect::<BTreeSet<_>>();
    let mut verified_bytes = 0;
    let duplicate = format!("{:064x}", 0);
    let source = |digest: &str| ContextManifestSourceRecord {
        object_uri: format!("session-object:sha256:{digest}"),
        projection_hash: digest.to_owned(),
        source_id: String::new(),
    };

    let duplicate_source = source(&duplicate);
    verify_context_manifest_objects(
        &sessions,
        session_id,
        std::slice::from_ref(&duplicate_source),
        &mut verified,
        &mut verified_bytes,
    )
    .expect("a duplicate digest does not consume the object-count budget");

    let novel = "f".repeat(64);
    let novel_source = source(&novel);
    let err = verify_context_manifest_objects(
        &sessions,
        session_id,
        std::slice::from_ref(&novel_source),
        &mut verified,
        &mut verified_bytes,
    )
    .expect_err("a novel digest beyond the object-count budget must be rejected");

    assert!(
        matches!(
            err,
            RuntimeError::Protocol(message)
                if message.ends_with("session object count exceeds max 131072")
        ),
        "the excess digest must be rejected before its missing object is opened"
    );
    assert_eq!(verified.len(), MAX_SESSION_OBJECTS);
}
