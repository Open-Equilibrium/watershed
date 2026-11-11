//! The Workspace calls as Dart makes them: the replica's key comes from the keychain, writes
//! reach their subscribers, and reads stay bounded.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::api::workspace::{
    BridgeChange, BridgeMutation, BridgeMutationKind, BridgeQuery, BridgeQueryKind, BridgeResource,
    BridgeResourceKind, BridgeTextStyle, LiquidWorkspace, MAX_WINDOW,
};
use crate::keys::{KEY_NAME_FILE, Keys, path_name};

/// A keychain in memory, with one entry per name.
#[derive(Default)]
pub(super) struct Memory(Mutex<HashMap<String, [u8; 32]>>);

impl Memory {
    /// The key it holds, if it holds exactly one.
    pub(super) fn only(&self) -> Option<[u8; 32]> {
        let keys = self.0.lock().unwrap();
        (keys.len() == 1).then(|| *keys.values().next().unwrap())
    }
}

/// A keychain that answers every name with the same key.
struct Any([u8; 32]);

impl Keys for Any {
    fn read(&self, _: &str) -> Result<Option<[u8; 32]>, String> {
        Ok(Some(self.0))
    }

    fn write(&self, _: &str, _: &[u8; 32]) -> Result<(), String> {
        Ok(())
    }

    fn delete(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
}

/// A keychain whose writes are refused.
struct Refusing<'a>(&'a Memory);

impl Keys for Refusing<'_> {
    fn read(&self, name: &str) -> Result<Option<[u8; 32]>, String> {
        self.0.read(name)
    }

    fn write(&self, _: &str, _: &[u8; 32]) -> Result<(), String> {
        Err("the keychain refused the write".into())
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        self.0.delete(name)
    }
}

impl Keys for Memory {
    fn read(&self, name: &str) -> Result<Option<[u8; 32]>, String> {
        Ok(self.0.lock().unwrap().get(name).copied())
    }

    fn write(&self, name: &str, key: &[u8; 32]) -> Result<(), String> {
        self.0.lock().unwrap().insert(name.into(), *key);
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(name);
        Ok(())
    }
}

/// A fresh replica folder, removed when dropped.
pub(super) struct Folder(pub(super) PathBuf);

impl Folder {
    pub(super) fn new(name: &str) -> Self {
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

pub(super) fn mutation(kind: BridgeMutationKind) -> BridgeMutation {
    BridgeMutation {
        kind,
        area: None,
        page: None,
        parent: None,
        block: None,
        view: None,
        after: None,
        last: false,
        text: None,
        at: 0,
        delete: 0,
        style: BridgeTextStyle::Paragraph,
        boundary: false,
        field: None,
        row: None,
        field_kind: None,
        color: 0,
        cell: None,
    }
}

pub(super) fn query(kind: BridgeQueryKind) -> BridgeQuery {
    BridgeQuery {
        kind,
        area: None,
        page: None,
        block: None,
        start: 0,
        limit: MAX_WINDOW,
        sort: None,
        filter: None,
    }
}

pub(super) fn resource(kind: BridgeResourceKind) -> BridgeResource {
    BridgeResource {
        kind,
        area: None,
        page: None,
        block: None,
    }
}

/// A Workspace with the area "Trips", its Page "Lisbon" and a Text Block on it.
pub(super) fn lisbon(workspace: &LiquidWorkspace) -> (String, String, String, String) {
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
    assert!(LiquidWorkspace::open_with(&folder.0, &Any([9; 32])).is_err());
}

#[test]
fn a_moved_replica_opens_with_its_key() {
    let folder = Folder::new("moved");
    let keys = Memory::default();
    lisbon(&LiquidWorkspace::open_with(&folder.0, &keys).unwrap());
    let moved = Folder::new("moved-to");
    std::fs::rename(&folder.0, &moved.0).unwrap();

    let workspace = LiquidWorkspace::open_with(&moved.0, &keys).unwrap();
    assert_eq!(
        workspace
            .query(query(BridgeQueryKind::Areas))
            .unwrap()
            .count,
        1
    );
    assert!(moved.0.join(KEY_NAME_FILE).is_file());
}

/// A replica as Liquid kept it before key names: its key filed under the folder's path and no
/// key name file.
fn filed_under_path(folder: &Folder) -> Memory {
    let keys = Memory::default();
    lisbon(&LiquidWorkspace::open_with(&folder.0, &keys).unwrap());
    let key = keys.only().unwrap();
    let _ = std::fs::remove_file(folder.0.join(KEY_NAME_FILE));
    let filed = Memory::default();
    filed.write(&path_name(&folder.0).unwrap(), &key).unwrap();
    filed
}

#[test]
fn a_key_filed_under_the_folder_path_moves_to_a_key_name_once() {
    let folder = Folder::new("migrate");
    let keys = filed_under_path(&folder);
    let path = path_name(&folder.0).unwrap();

    let workspace = LiquidWorkspace::open_with(&folder.0, &keys).unwrap();
    assert_eq!(
        workspace
            .query(query(BridgeQueryKind::Areas))
            .unwrap()
            .count,
        1
    );
    drop(workspace);
    assert_eq!(keys.read(&path).unwrap(), None);
    assert!(folder.0.join(KEY_NAME_FILE).is_file());
    assert!(LiquidWorkspace::open_with(&folder.0, &keys).is_ok());
}

#[test]
fn a_failed_key_migration_keeps_the_old_key_for_a_retry() {
    let folder = Folder::new("retry");
    let keys = filed_under_path(&folder);
    let path = path_name(&folder.0).unwrap();

    assert!(LiquidWorkspace::open_with(&folder.0, &Refusing(&keys)).is_err());
    assert!(keys.read(&path).unwrap().is_some());
    assert!(LiquidWorkspace::open_with(&folder.0, &keys).is_ok());
    assert_eq!(keys.read(&path).unwrap(), None);
}

#[test]
fn a_malformed_key_name_file_is_refused() {
    let folder = Folder::new("malformed");
    let keys = Memory::default();
    lisbon(&LiquidWorkspace::open_with(&folder.0, &keys).unwrap());
    std::fs::write(folder.0.join(KEY_NAME_FILE), "not a key name").unwrap();

    let error = LiquidWorkspace::open_with(&folder.0, &keys).err().unwrap();
    assert!(error.contains("malformed"), "{error}");
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
    let keychain = crate::keys::Keychain;
    let name = "liquid-bridge-test";
    keychain.delete(name).unwrap();
    assert_eq!(keychain.read(name).unwrap(), None);
    keychain.write(name, &[5; 32]).unwrap();
    assert_eq!(keychain.read(name).unwrap(), Some([5; 32]));
    keychain.delete(name).unwrap();
    assert_eq!(keychain.read(name).unwrap(), None);

    let folder = Folder::new("keychain");
    lisbon(&LiquidWorkspace::open(folder.0.display().to_string()).unwrap());
    let reopened = LiquidWorkspace::open(folder.0.display().to_string()).unwrap();
    assert_eq!(
        reopened.query(query(BridgeQueryKind::Areas)).unwrap().count,
        1
    );
    let key_name = std::fs::read_to_string(folder.0.join(KEY_NAME_FILE)).unwrap();
    keychain.delete(&key_name).unwrap();
}

#[cfg(unix)]
#[test]
fn a_folder_whose_path_is_not_utf8_has_no_key_name() {
    use std::os::unix::ffi::OsStrExt;
    let folder = std::ffi::OsStr::from_bytes(b"/nonexistent/replica-\x80");
    assert!(path_name(std::path::Path::new(folder)).is_err());
}

#[test]
fn one_session_at_a_time_opens_a_replica_folder() {
    let folder = Folder::new("exclusive");
    let keys = Memory::default();
    let first = LiquidWorkspace::open_with(&folder.0, &keys).unwrap();
    let key = keys.only();

    let second = LiquidWorkspace::open_with(&folder.0, &Memory::default());
    assert_eq!(
        second.err().unwrap(),
        "another Liquid session holds this replica"
    );
    assert_eq!(keys.only(), key);
    drop(first);
    assert!(LiquidWorkspace::open_with(&folder.0, &keys).is_ok());
}

#[test]
fn a_lock_held_only_for_a_moment_is_waited_for() {
    // A child process started between fork and exec holds the lock it inherited until exec.
    let folder = Folder::new("moment");
    std::fs::create_dir_all(&folder.0).unwrap();
    let held = std::fs::File::create(folder.0.join("replica.lock")).unwrap();
    held.lock().unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        drop(held);
    });
    assert!(LiquidWorkspace::open_with(&folder.0, &Memory::default()).is_ok());
    release.join().unwrap();
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
    let refused = workspace.apply(BridgeMutation {
        page: Some(page.clone()),
        block: Some(block),
        text: Some("x".into()),
        at: 99,
        ..mutation(BridgeMutationKind::EditText)
    });
    assert_eq!(refused.unwrap_err(), "text position out of range");
    assert!(!workspace.is_loaded(&id), "after a refused write");
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

#[test]
fn every_way_a_subscription_ends_leaves_nothing_behind() {
    let folder = Folder::new("ends");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    let areas = || resource(BridgeResourceKind::Areas);
    // Its stream was gone at registration, then Dart closed it.
    workspace.watch(1, areas(), |_| false).unwrap();
    workspace.close(1);
    // Its stream went away with a change, then Dart closed it.
    let mut first = true;
    workspace
        .watch(2, areas(), move |_| std::mem::take(&mut first))
        .unwrap();
    lisbon(&workspace);
    workspace.close(2);
    // Dart closed it before it arrived.
    workspace.close(3);
    workspace.watch(3, areas(), |_| true).unwrap();
    // Dart closed it after it arrived.
    workspace.watch(4, areas(), |_| true).unwrap();
    workspace.close(4);
    // Rust refused it, before and after Dart closed it.
    let nameless = || resource(BridgeResourceKind::Views);
    assert!(workspace.watch(5, nameless(), |_| true).is_err());
    workspace.close(5);
    workspace.close(6);
    assert!(workspace.watch(6, nameless(), |_| true).is_err());

    assert_eq!(workspace.lifecycle_markers(), 0);
}

#[test]
fn text_blocks_split_join_restyle_and_start_sections_through_the_bridge() {
    let folder = Folder::new("text");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    let (_, page, block, view) = lisbon(&workspace);
    let split = workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            block: Some(block.clone()),
            view: Some(view.clone()),
            at: 4,
            style: BridgeTextStyle::Todo,
            ..mutation(BridgeMutationKind::SplitTextBlock)
        })
        .unwrap();
    let (rest, rest_view) = (split.block.unwrap(), split.view.unwrap());
    let read = |block: &str| {
        let answer = workspace
            .query(BridgeQuery {
                page: Some(page.clone()),
                block: Some(block.into()),
                ..query(BridgeQueryKind::Text)
            })
            .unwrap();
        (answer.text, answer.style)
    };
    assert_eq!(
        read(&block),
        (Some("Five".into()), Some(BridgeTextStyle::Paragraph))
    );
    assert_eq!(
        read(&rest),
        (Some(" days".into()), Some(BridgeTextStyle::Todo))
    );

    workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            block: Some(rest.clone()),
            style: BridgeTextStyle::Heading2,
            ..mutation(BridgeMutationKind::SetTextStyle)
        })
        .unwrap();
    assert_eq!(read(&rest).1, Some(BridgeTextStyle::Heading2));
    workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            view: Some(rest_view.clone()),
            boundary: true,
            ..mutation(BridgeMutationKind::SetSectionBoundary)
        })
        .unwrap();
    let views = workspace
        .query(BridgeQuery {
            page: Some(page.clone()),
            ..query(BridgeQueryKind::Views)
        })
        .unwrap()
        .views;
    assert_eq!(
        views.iter().map(|view| view.boundary).collect::<Vec<_>>(),
        [false, true]
    );

    // ⌫ below the divider removes it; only then does ⌫ join the Text Blocks.
    let join = || BridgeMutation {
        page: Some(page.clone()),
        view: Some(rest_view.clone()),
        ..mutation(BridgeMutationKind::JoinTextBlock)
    };
    assert_eq!(
        workspace.apply(join()).unwrap_err(),
        "the Text Block starts a Section"
    );
    workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            view: Some(rest_view.clone()),
            boundary: false,
            ..mutation(BridgeMutationKind::SetSectionBoundary)
        })
        .unwrap();
    workspace.apply(join()).unwrap();
    assert_eq!(read(&block).0, Some("Five days".into()));
    assert_eq!(read(&rest), (None, None));
}

#[cfg(any(unix, windows))]
#[test]
fn opening_never_writes_through_a_linked_lock_file() {
    let folder = Folder::new("link");
    std::fs::create_dir_all(&folder.0).unwrap();
    let elsewhere = folder.0.join("notes.txt");
    std::fs::write(&elsewhere, "keep me").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&elsewhere, folder.0.join("replica.lock")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(&elsewhere, folder.0.join("replica.lock")).unwrap();

    let opened = LiquidWorkspace::open_with(&folder.0, &Memory::default());
    assert!(opened.is_err(), "a linked lock file is refused");
    assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "keep me");
}
