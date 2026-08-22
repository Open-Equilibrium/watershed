use super::super::helpers::{
    create_directory_alias, empty_workspace, remove_directory_alias, workspace_session_dir,
};
use super::{create_review_run, file_tree_bytes};
use crate::runtime::{
    context::ContextObject,
    conversations::{RunObjectStore, RunObjectUsageSnapshot},
    digest::sha256_hex,
};
use std::{
    fs::{self},
    sync::{Arc, Barrier},
    thread,
};

#[test]
fn run_object_store_rejects_an_intermediate_runs_link_without_touching_its_target() {
    let workspace = empty_workspace("run-object-store-runs-link");
    let outside = empty_workspace("run-object-store-runs-link-target");
    create_review_run(&workspace);
    let runs = workspace_session_dir(&workspace).join("review/runs");
    let outside_runs = outside.join("runs");
    fs::rename(&runs, &outside_runs).expect("run tree moves outside the workspace");
    let before = file_tree_bytes(&outside_runs);
    create_directory_alias(&runs, &outside_runs);

    let result = RunObjectStore::open(&workspace, "review", "review-1");

    remove_directory_alias(&runs);
    assert!(
        result.is_err(),
        "an intermediate runs link must fail closed"
    );
    assert_eq!(
        file_tree_bytes(&outside_runs),
        before,
        "the external run tree remains untouched"
    );
}

#[test]
fn shared_run_object_store_reuses_and_accounts_persisted_objects() {
    let workspace = empty_workspace("run-object-store-boundedness");
    create_review_run(&workspace);
    let first = ContextObject {
        digest: sha256_hex(b"first"),
        bytes: b"first".to_vec(),
    };
    let second = ContextObject {
        digest: sha256_hex(b"second"),
        bytes: b"second".to_vec(),
    };
    let first_owner = RunObjectStore::open(&workspace, "review", "review-1")
        .expect("run object store opens once");
    let second_owner = first_owner.clone();
    first_owner
        .persist(std::slice::from_ref(&first))
        .expect("first owner seeds the object store");

    first_owner
        .persist(&[])
        .expect("first owner accepts an empty batch");
    second_owner
        .persist(std::slice::from_ref(&first))
        .expect("second owner accepts the existing object");
    first_owner
        .persist(std::slice::from_ref(&second))
        .expect("first owner publishes a new object");
    second_owner
        .persist(&[first.clone(), second.clone()])
        .expect("second owner reuses both objects");

    assert_eq!(
        first_owner.usage_snapshot().expect("usage snapshot reads"),
        RunObjectUsageSnapshot {
            object_bytes: u64::try_from(first.bytes.len() + second.bytes.len()).unwrap(),
            object_count: 2,
        }
    );
    let objects = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/objects");
    assert_eq!(fs::read(objects.join(first.digest)).unwrap(), first.bytes);
    assert_eq!(fs::read(objects.join(second.digest)).unwrap(), second.bytes);
}

#[test]
fn shared_run_object_store_admits_only_one_final_slot() {
    let workspace = empty_workspace("run-object-store-final-slot");
    create_review_run(&workspace);
    let store =
        RunObjectStore::open_with_object_limit_for_test(&workspace, "review", "review-1", 1)
            .expect("single-slot object store opens");
    let barrier = Arc::new(Barrier::new(2));
    let attempts = [b"first".to_vec(), b"second".to_vec()]
        .into_iter()
        .map(|bytes| {
            let store = store.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let object = ContextObject {
                    digest: sha256_hex(&bytes),
                    bytes,
                };
                barrier.wait();
                store.persist(&[object]).map_err(|error| error.to_string())
            })
        })
        .collect::<Vec<_>>();
    let results = attempts
        .into_iter()
        .map(|attempt| attempt.join().expect("object owner joins"))
        .collect::<Vec<_>>();

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .filter(|error| error.contains("run object count exceeds its limit"))
            .count(),
        1
    );
    assert_eq!(
        fs::read_dir(
            crate::tests::helpers::workspace_session_dir(&workspace)
                .join("review/runs/review-1/objects")
        )
        .expect("object directory reads")
        .count(),
        1
    );
}

#[test]
fn reopened_run_object_store_preserves_usage_and_verifies_object_integrity() {
    let workspace = empty_workspace("run-object-store-reopen");
    create_review_run(&workspace);
    let object = ContextObject {
        digest: sha256_hex(b"durable object"),
        bytes: b"durable object".to_vec(),
    };
    let uri = format!("session-object:sha256:{}", object.digest);
    let store =
        RunObjectStore::open(&workspace, "review", "review-1").expect("run object store opens");
    store
        .persist(std::slice::from_ref(&object))
        .expect("object persists");
    drop(store);

    let reopened = RunObjectStore::open(&workspace, "review", "review-1")
        .expect("run object store reopens from its durable inventory");
    assert_eq!(
        reopened.usage_snapshot().expect("usage snapshot reads"),
        RunObjectUsageSnapshot {
            object_bytes: u64::try_from(object.bytes.len()).unwrap(),
            object_count: 1,
        }
    );
    assert_eq!(reopened.read(&uri).expect("object reads"), object.bytes);
    reopened
        .persist(std::slice::from_ref(&object))
        .expect("reopened inventory verifies the existing object");

    let object_path = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/objects")
        .join(&object.digest);
    fs::write(&object_path, b"tampered objct").expect("fixture tampers with equal-length bytes");
    let error = reopened
        .read(&uri)
        .expect_err("an object's bytes must continue to match its URI digest");
    assert!(error.to_string().contains("does not match its URI digest"));
    let error = reopened
        .persist(std::slice::from_ref(&object))
        .expect_err("cached verification must not accept changed object bytes");
    assert!(error.to_string().contains("does not match its digest"));
    assert!(
        reopened.usage_snapshot().is_err(),
        "the store remains closed after cached verification fails"
    );
}

#[test]
fn reopened_run_object_store_closes_after_existing_object_verification_fails() {
    let workspace = empty_workspace("run-object-store-corrupt-reopen");
    create_review_run(&workspace);
    let object = ContextObject {
        digest: sha256_hex(b"durable object"),
        bytes: b"durable object".to_vec(),
    };
    let store =
        RunObjectStore::open(&workspace, "review", "review-1").expect("run object store opens");
    store
        .persist(std::slice::from_ref(&object))
        .expect("object persists");
    drop(store);

    let object_path = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/objects")
        .join(&object.digest);
    fs::write(&object_path, b"tampered objct").expect("fixture tampers with equal-length bytes");
    let reopened = RunObjectStore::open(&workspace, "review", "review-1")
        .expect("run object store reopens from its durable inventory");
    let error = reopened
        .persist(std::slice::from_ref(&object))
        .expect_err("reusing a corrupted object must fail exact verification");
    assert!(error.to_string().contains("does not match its digest"));
    assert!(
        reopened.usage_snapshot().is_err(),
        "the store remains closed after integrity failure"
    );
}
