use super::workspace::{Folder, Memory, lisbon, mutation, query};
use crate::api::history::{BridgeActorKind, BridgeEntryQuery};
use crate::api::replica::LiquidWorkspace;
use crate::api::workspace::{BridgeMutation, BridgeMutationKind, BridgeQuery, BridgeQueryKind};

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
    let folder = Folder::new("history");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    let (_, page, block, _) = lisbon(&workspace);
    let text = || {
        workspace
            .query(BridgeQuery {
                page: Some(page.clone()),
                block: Some(block.clone()),
                ..query(BridgeQueryKind::Text)
            })
            .unwrap()
            .text
            .unwrap()
    };
    let before = text();
    let typed = workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            block: Some(block.clone()),
            text: Some("Ours: ".into()),
            ..mutation(BridgeMutationKind::EditText)
        })
        .unwrap()
        .action;
    assert_eq!(text(), format!("Ours: {before}"));

    let entries = workspace.history(all()).unwrap();
    assert_eq!(entries[0].actions, std::slice::from_ref(&typed));
    assert_eq!(entries[0].actor.kind, BridgeActorKind::Person);
    assert_eq!(entries[0].allowed_by, "Can manage");
    assert!(entries[0].revertable);

    assert!(workspace.undo().unwrap());
    assert_eq!(text(), before);
    assert!(workspace.redo().unwrap());
    assert_eq!(text(), format!("Ours: {before}"));
    let reverted = workspace.revert(vec![typed.clone()]).unwrap();
    assert!(!reverted.conflict && !reverted.purged);
    assert_eq!(text(), before);

    let versions = workspace.versions(page.clone()).unwrap();
    assert!(!versions.is_empty());
    let preview = workspace
        .preview(page.clone(), versions[0].after.clone())
        .unwrap();
    assert_eq!(
        preview
            .query(BridgeQuery {
                page: Some(page.clone()),
                block: Some(block.clone()),
                ..query(BridgeQueryKind::Text)
            })
            .unwrap()
            .text
            .unwrap(),
        before
    );
    assert!(
        preview
            .apply(BridgeMutation {
                page: Some(page.clone()),
                block: Some(block.clone()),
                text: Some("x".into()),
                ..mutation(BridgeMutationKind::EditText)
            })
            .is_err()
    );
    workspace.restore(page.clone(), typed.clone()).unwrap();
    assert_eq!(text(), format!("Ours: {before}"));

    workspace.purge(Some(page.clone()), None).unwrap();
    let after = workspace.revert(vec![typed]).unwrap();
    assert!(after.purged);
    let newest = workspace.history(all()).unwrap().remove(0);
    assert!(newest.title.starts_with("Deleted the earlier versions of"));
}
