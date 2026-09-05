use crate::runtime::session_bundle::SessionBundlePaths;

#[test]
fn session_bundle_leaf_builders_and_parsers_share_one_grammar() {
    let session_id = "bundle001";
    let digest = "a".repeat(64);

    let contexts = SessionBundlePaths::contexts_leaf(session_id);
    let events = SessionBundlePaths::events_leaf(session_id);
    let lock = SessionBundlePaths::lock_leaf(session_id);
    let metadata = SessionBundlePaths::metadata_leaf(session_id);
    let object = SessionBundlePaths::object_leaf(session_id, &digest);

    assert_eq!(
        SessionBundlePaths::split_contexts_leaf(&contexts),
        Some(session_id)
    );
    assert_eq!(
        SessionBundlePaths::split_events_leaf(&events),
        Some(session_id)
    );
    assert_eq!(SessionBundlePaths::split_lock_leaf(&lock), Some(session_id));
    assert_eq!(
        SessionBundlePaths::split_metadata_leaf(&metadata),
        Some(session_id)
    );
    assert_eq!(
        SessionBundlePaths::split_object_leaf(&object),
        Some((session_id, digest.as_str()))
    );
}
