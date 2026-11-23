use super::workspace::{lisbon, on_block, open};
use crate::api::history::{BridgeActorKind, BridgeEntryQuery};
use crate::api::workspace::{BridgeMutation, BridgeMutationKind};

fn all() -> BridgeEntryQuery {
    BridgeEntryQuery {
        page: None,
        automation: None,
        origin: None,
        before: None,
        limit: 50,
    }
}

#[test]
fn history_revert_undo_restore_and_purge_cross_the_bridge() {
    let (_folder, workspace) = open("history");
    let (_, page, block, _) = lisbon(&workspace);
    let typed = workspace
        .apply(BridgeMutation {
            text: Some("Ours: ".into()),
            ..on_block(&page, &block, BridgeMutationKind::EditText)
        })
        .unwrap()
        .action;

    let entries = workspace.history(all()).unwrap().entries;
    assert_eq!(entries[0].actions, std::slice::from_ref(&typed));
    assert_eq!(entries[0].actor.kind, BridgeActorKind::Person);
    assert_eq!(entries[0].allowed_by, "Can manage");
    assert!(entries[0].revertable);

    assert!(workspace.undo().unwrap());
    assert!(workspace.redo().unwrap());
    let reverted = workspace.revert(vec![typed.clone()]).unwrap();
    assert!(!reverted.conflict && !reverted.purged);

    let versions = workspace.versions(page.clone()).unwrap();
    let preview = workspace
        .preview(page.clone(), versions[0].after.clone())
        .unwrap();
    assert!(
        preview
            .apply(BridgeMutation {
                text: Some("x".into()),
                ..on_block(&page, &block, BridgeMutationKind::EditText)
            })
            .is_err()
    );
    workspace.restore(page.clone(), typed.clone()).unwrap();

    workspace.purge(Some(page.clone()), None).unwrap();
    let after = workspace.revert(vec![typed]).unwrap();
    assert!(after.purged);
    let newest = workspace.history(all()).unwrap().entries.remove(0);
    assert!(newest.title.starts_with("Deleted the earlier versions of"));
}
