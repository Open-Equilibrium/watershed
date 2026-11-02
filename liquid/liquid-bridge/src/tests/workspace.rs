//! The Workspace calls as Dart makes them: the replica's key comes from the keychain, writes
//! reach their subscribers, and reads stay bounded.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::api::workspace::{
    BridgeChange, BridgeMutation, BridgeMutationKind, BridgeQuery, BridgeQueryKind, BridgeResource,
    BridgeResourceKind, LiquidWorkspace, MAX_WINDOW,
};
use crate::keys::Keys;

/// A keychain in memory.
#[derive(Default)]
struct Memory(Mutex<Option<[u8; 32]>>);

impl Keys for Memory {
    fn read(&self) -> Result<Option<[u8; 32]>, String> {
        Ok(*self.0.lock().unwrap())
    }

    fn write(&self, key: &[u8; 32]) -> Result<(), String> {
        *self.0.lock().unwrap() = Some(*key);
        Ok(())
    }
}

/// A fresh replica folder, removed when dropped.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "liquid-bridge-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn mutation(kind: BridgeMutationKind) -> BridgeMutation {
    BridgeMutation {
        kind,
        area: None,
        page: None,
        parent: None,
        block: None,
        view: None,
        after: None,
        text: None,
        at: 0,
        delete: 0,
    }
}

fn query(kind: BridgeQueryKind) -> BridgeQuery {
    BridgeQuery {
        kind,
        area: None,
        page: None,
        block: None,
        start: 0,
        limit: MAX_WINDOW,
    }
}

fn resource(kind: BridgeResourceKind) -> BridgeResource {
    BridgeResource {
        kind,
        area: None,
        page: None,
        block: None,
    }
}

/// A Workspace with the area "Trips", its Page "Lisbon" and a Text Block on it.
fn lisbon(workspace: &LiquidWorkspace) -> (String, String, String, String) {
    let area = workspace
        .apply(BridgeMutation {
            text: Some("Trips".into()),
            ..mutation(BridgeMutationKind::CreateArea)
        })
        .unwrap()
        .area
        .unwrap();
    let page = workspace
        .apply(BridgeMutation {
            area: Some(area.clone()),
            text: Some("Lisbon".into()),
            ..mutation(BridgeMutationKind::CreatePage)
        })
        .unwrap()
        .page
        .unwrap();
    let added = workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            text: Some("Five days".into()),
            ..mutation(BridgeMutationKind::AddTextBlock)
        })
        .unwrap();
    (area, page, added.block.unwrap(), added.view.unwrap())
}

fn text(workspace: &LiquidWorkspace, page: &str, block: &str) -> Option<String> {
    workspace
        .query(BridgeQuery {
            page: Some(page.into()),
            block: Some(block.into()),
            ..query(BridgeQueryKind::Text)
        })
        .unwrap()
        .text
}

#[test]
fn a_replica_opens_again_with_the_key_the_keychain_holds() {
    let folder = Folder::new("reopen");
    let keys = Memory::default();
    let (area, page, block, _) = lisbon(&LiquidWorkspace::open_with(&folder.0, &keys).unwrap());

    let workspace = LiquidWorkspace::open_with(&folder.0, &keys).unwrap();
    let areas = workspace.query(query(BridgeQueryKind::Areas)).unwrap();
    assert_eq!((areas.count, areas.areas[0].name.as_str()), (1, "Trips"));
    let pages = workspace
        .query(BridgeQuery {
            area: Some(area),
            ..query(BridgeQueryKind::Pages)
        })
        .unwrap();
    assert_eq!(pages.pages[0].title, "Lisbon");
    assert_eq!(text(&workspace, &page, &block).unwrap(), "Five days");
}

#[test]
fn a_replica_without_its_key_is_refused() {
    let folder = Folder::new("key");
    lisbon(&LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap());

    let missing = LiquidWorkspace::open_with(&folder.0, &Memory::default());
    assert_eq!(
        missing.err().unwrap(),
        "the replica's key is missing from the keychain"
    );
    let wrong = Memory(Mutex::new(Some([9; 32])));
    assert!(LiquidWorkspace::open_with(&folder.0, &wrong).is_err());
}

#[test]
fn a_write_reaches_only_the_subscribers_of_what_it_changed() {
    let folder = Folder::new("notify");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    let (_, page, block, view) = lisbon(&workspace);
    let seen: Arc<Mutex<Vec<(i64, BridgeChange)>>> = Arc::default();
    let follow = |id: i64, resource: BridgeResource| {
        let seen = seen.clone();
        workspace
            .watch(id, resource, move |change| {
                seen.lock().unwrap().push((id, change));
                true
            })
            .unwrap();
    };
    follow(1, resource(BridgeResourceKind::Areas));
    follow(
        2,
        BridgeResource {
            page: Some(page.clone()),
            ..resource(BridgeResourceKind::Views)
        },
    );
    follow(
        3,
        BridgeResource {
            page: Some(page.clone()),
            block: Some(block.clone()),
            ..resource(BridgeResourceKind::Block)
        },
    );

    for insert in ["Our ", "Six "] {
        workspace
            .apply(BridgeMutation {
                page: Some(page.clone()),
                block: Some(block.clone()),
                text: Some(insert.into()),
                ..mutation(BridgeMutationKind::EditText)
            })
            .unwrap();
    }
    workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            view: Some(view),
            ..mutation(BridgeMutationKind::RemoveView)
        })
        .unwrap();

    let seen: Vec<(i64, i64)> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|(id, change)| (*id, change.seq))
        .collect();
    // Each subscription starts with its resource's count; two edits and the removal reach
    // the Block, and only the removal reaches the Views.
    assert_eq!(
        seen,
        [(1, 1), (2, 1), (3, 0), (3, 1), (3, 2), (2, 2), (3, 3)]
    );
    assert_eq!(text(&workspace, &page, &block), None);
}

#[test]
fn a_subscription_ends_when_dart_closes_it_or_its_stream() {
    let folder = Folder::new("close");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    let (area, page, block, _) = lisbon(&workspace);
    let calls = Arc::new(Mutex::new(0));
    let counted = calls.clone();
    let pages = BridgeResource {
        area: Some(area.clone()),
        ..resource(BridgeResourceKind::Pages)
    };
    workspace
        .watch(7, pages.clone(), move |_| {
            *counted.lock().unwrap() += 1;
            false
        })
        .unwrap();
    let blocks = BridgeResource {
        page: Some(page.clone()),
        block: Some(block.clone()),
        ..resource(BridgeResourceKind::Block)
    };
    workspace.watch(8, blocks.clone(), |_| true).unwrap();
    assert!(workspace.watch(8, blocks, |_| true).is_err());

    let rename = |title: &str| {
        workspace
            .apply(BridgeMutation {
                page: Some(page.clone()),
                text: Some(title.into()),
                ..mutation(BridgeMutationKind::RenamePage)
            })
            .unwrap();
    };
    rename("Porto");
    rename("Faro");
    assert_eq!(
        *calls.lock().unwrap(),
        1,
        "the closed stream got one change"
    );

    // Closing the Page's last subscription releases it; it loads again for the next read.
    workspace.close(8);
    assert_eq!(text(&workspace, &page, &block).unwrap(), "Five days");
}

#[test]
fn reads_and_writes_refuse_malformed_requests() {
    let folder = Folder::new("refuse");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    let (_, page, block, _) = lisbon(&workspace);

    let unknown = workspace.apply(BridgeMutation {
        page: Some("trips".into()),
        text: Some("x".into()),
        ..mutation(BridgeMutationKind::AddTextBlock)
    });
    assert_eq!(unknown.unwrap_err(), "page is missing or no identity");
    let untitled = workspace.apply(BridgeMutation {
        page: Some(page.clone()),
        ..mutation(BridgeMutationKind::RenamePage)
    });
    assert_eq!(untitled.unwrap_err(), "text is missing");
    let backwards = workspace.apply(BridgeMutation {
        page: Some(page.clone()),
        block: Some(block),
        text: Some("x".into()),
        at: -1,
        ..mutation(BridgeMutationKind::EditText)
    });
    assert_eq!(backwards.unwrap_err(), "at is negative");
    let negative = workspace.query(BridgeQuery {
        start: -1,
        ..query(BridgeQueryKind::Areas)
    });
    assert_eq!(negative.unwrap_err(), "start is negative");
}

#[test]
fn a_window_never_exceeds_its_cap() {
    let folder = Folder::new("window");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    let (_, page, _, mut after) = lisbon(&workspace);
    // Followed, as the app follows the Page it edits, so it stays in memory between writes.
    let views = BridgeResource {
        page: Some(page.clone()),
        ..resource(BridgeResourceKind::Views)
    };
    workspace.watch(1, views, |_| true).unwrap();
    for day in 0..MAX_WINDOW + 10 {
        after = workspace
            .apply(BridgeMutation {
                page: Some(page.clone()),
                after: Some(after),
                text: Some(format!("Day {day}")),
                ..mutation(BridgeMutationKind::AddTextBlock)
            })
            .unwrap()
            .view
            .unwrap();
    }
    let views = workspace
        .query(BridgeQuery {
            page: Some(page),
            limit: i64::MAX,
            ..query(BridgeQueryKind::Views)
        })
        .unwrap();
    assert_eq!(views.views.len() as i64, MAX_WINDOW);
    assert_eq!(views.count, MAX_WINDOW + 11);
}

/// Needs the operating system's keychain, which CI hosts lack; run it by hand with
/// `cargo test -p liquid_bridge -- --ignored keychain`.
#[test]
#[ignore = "needs the operating system's keychain"]
fn the_keychain_keeps_a_replica_key() {
    let folder = Folder::new("keychain");
    std::fs::create_dir_all(&folder.0).unwrap();
    let keychain = crate::keys::Keychain::for_replica(&folder.0);
    assert_eq!(keychain.read().unwrap(), None);
    keychain.write(&[5; 32]).unwrap();
    assert_eq!(keychain.read().unwrap(), Some([5; 32]));
    lisbon(&LiquidWorkspace::open(folder.0.display().to_string()).unwrap());
    let reopened = LiquidWorkspace::open(folder.0.display().to_string()).unwrap();
    assert_eq!(
        reopened.query(query(BridgeQueryKind::Areas)).unwrap().count,
        1
    );
    keychain.delete().unwrap();
}

#[test]
fn one_session_at_a_time_opens_a_replica_folder() {
    let folder = Folder::new("exclusive");
    let keys = Memory::default();
    let first = LiquidWorkspace::open_with(&folder.0, &keys).unwrap();
    let key = keys.read().unwrap();

    let second = LiquidWorkspace::open_with(&folder.0, &Memory::default());
    assert_eq!(
        second.err().unwrap(),
        "another Liquid session holds this replica"
    );
    assert_eq!(keys.read().unwrap(), key);
    drop(first);
    assert!(LiquidWorkspace::open_with(&folder.0, &keys).is_ok());
}

#[test]
fn a_subscription_starts_with_its_resource_s_current_count() {
    let folder = Folder::new("ack");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    lisbon(&workspace);
    let seen: Arc<Mutex<Vec<i64>>> = Arc::default();
    let record = seen.clone();
    workspace
        .watch(1, resource(BridgeResourceKind::Areas), move |change| {
            record.lock().unwrap().push(change.seq);
            true
        })
        .unwrap();
    workspace
        .apply(BridgeMutation {
            text: Some("Home".into()),
            ..mutation(BridgeMutationKind::CreateArea)
        })
        .unwrap();
    // "Trips" counted once before the subscription began.
    assert_eq!(*seen.lock().unwrap(), [1, 2]);
}

#[test]
fn a_close_that_overtakes_its_subscription_still_ends_it() {
    let folder = Folder::new("overtake");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    workspace.close(4);
    let calls = Arc::new(Mutex::new(0));
    let counted = calls.clone();
    let opened = workspace.watch(4, resource(BridgeResourceKind::Areas), move |_| {
        *counted.lock().unwrap() += 1;
        true
    });
    assert!(
        opened.is_ok(),
        "an already closed subscription ends quietly"
    );
    lisbon(&workspace);
    assert_eq!(*calls.lock().unwrap(), 0);
}

#[test]
fn a_page_nothing_follows_leaves_memory_after_each_call() {
    let folder = Folder::new("release");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    let (_, page, block, _) = lisbon(&workspace);
    let id = liquid_core::PageId::parse(&page).unwrap();
    assert!(!workspace.is_loaded(&id), "after the write that made it");

    let views = BridgeResource {
        page: Some(page.clone()),
        ..resource(BridgeResourceKind::Views)
    };
    workspace.watch(1, views, |_| true).unwrap();
    assert_eq!(text(&workspace, &page, &block).unwrap(), "Five days");
    assert!(workspace.is_loaded(&id), "while followed");
    workspace.close(1);
    assert!(!workspace.is_loaded(&id));
    assert_eq!(text(&workspace, &page, &block).unwrap(), "Five days");
    assert!(!workspace.is_loaded(&id), "after a read nothing follows");
}

#[test]
fn a_subscription_follows_its_resource_whatever_else_it_names() {
    let folder = Folder::new("normalize");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    let (area, page, _, _) = lisbon(&workspace);
    let seen: Arc<Mutex<Vec<i64>>> = Arc::default();
    let record = seen.clone();
    let views = BridgeResource {
        area: Some(area),
        page: Some(page.clone()),
        block: Some("ignored".into()),
        ..resource(BridgeResourceKind::Views)
    };
    workspace
        .watch(1, views, move |change| {
            record.lock().unwrap().push(change.seq);
            true
        })
        .unwrap();
    workspace
        .apply(BridgeMutation {
            page: Some(page),
            text: Some("Tram 28".into()),
            ..mutation(BridgeMutationKind::AddTextBlock)
        })
        .unwrap();
    assert_eq!(*seen.lock().unwrap(), [1, 2]);
    let nameless = workspace.watch(2, resource(BridgeResourceKind::Views), |_| true);
    assert_eq!(nameless.unwrap_err(), "page is missing or no identity");
}
