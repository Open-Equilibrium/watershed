use super::*;

#[test]
fn session_bundle_inventory_owns_paths_segments_objects_and_byte_counts() {
    let workspace = empty_workspace("session-bundle-inventory");
    let reservation =
        reserve_session_log(&workspace, "inventory001").expect("session bundle reserved");
    let paths = SessionBundlePaths::from_reservation(&reservation);
    reservation.activate().expect("reservation activates");
    drop(reservation);
    fs::write(paths.events.diagnostic_path(), b"event-one\n").expect("event segment written");
    fs::write(
        paths
            .events
            .diagnostic_path()
            .with_file_name("inventory001.000002.jsonl"),
        b"event-two\n",
    )
    .expect("second event segment written");
    fs::write(paths.contexts.diagnostic_path(), b"context\n").expect("context segment written");
    fs::write(paths.metadata.diagnostic_path(), b"metadata").expect("metadata written");
    let object_bytes = b"object";
    fs::write(
        paths.sessions.path.join(format!(
            "inventory001.object.sha256-{}",
            sha256_hex(object_bytes)
        )),
        object_bytes,
    )
    .expect("object written");

    let inventory = SessionBundleInventory::inspect(paths).expect("bundle inventory");

    assert_eq!(inventory.event_segments.len(), 2);
    assert_eq!(inventory.context_segments.len(), 1);
    assert_eq!(inventory.objects.len(), 1);
    assert_eq!(inventory.event_bytes, 20);
    assert_eq!(inventory.context_bytes, 8);
    assert_eq!(inventory.metadata_bytes, 8);
    assert_eq!(inventory.object_bytes, 6);
    assert_eq!(inventory.total_bytes(), 42);
    assert!(inventory.lock_present);
}

#[test]
fn session_object_inventory_bounds_zero_byte_entries_before_opening_the_excess() {
    let workspace = empty_workspace("session-object-count");
    let reservation =
        reserve_session_log(&workspace, "objectcount001").expect("session bundle reserved");
    let sessions = SessionBundlePaths::from_reservation(&reservation).sessions;
    let opened = std::cell::Cell::new(0);

    let (accepted, bytes) = generated_zero_byte_session_objects_for_test(
        &sessions,
        "objectcount001",
        MAX_SESSION_OBJECTS,
        &opened,
    )
    .expect("the maximum zero-byte object count is accepted");
    assert_eq!(accepted.len(), MAX_SESSION_OBJECTS);
    assert_eq!(opened.get(), MAX_SESSION_OBJECTS);
    assert_eq!(bytes, 0);
    drop(accepted);

    opened.set(0);
    let excess = generated_zero_byte_session_objects_for_test(
        &sessions,
        "objectcount001",
        MAX_SESSION_OBJECTS + 1,
        &opened,
    );
    let err = match excess {
        Err(err) => err,
        Ok(_) => panic!("the 131,073rd object must be rejected"),
    };
    assert!(
        matches!(
            err,
            RuntimeError::Protocol(message)
                if message.ends_with("session object count exceeds max 131072")
        ),
        "unexpected object-count error"
    );
    assert_eq!(
        opened.get(),
        MAX_SESSION_OBJECTS,
        "the excess object must be rejected before it is opened"
    );
}
