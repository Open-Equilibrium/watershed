use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::api::database::BridgeLayout;
use crate::api::replica::LiquidWorkspace;
use crate::api::workspace::{
    BridgeChange, BridgeFound, BridgeMutation, BridgeMutationKind, BridgeQuery, BridgeQueryKind,
    BridgeResource, BridgeResourceKind, BridgeTextStyle, BridgeTile, MAX_WINDOW,
};
use crate::keys::{KEY_NAME_FILE, Keys, path_name};

#[derive(Default)]
pub(super) struct Memory(Mutex<HashMap<String, [u8; 32]>>);

impl Memory {
    pub(super) fn only(&self) -> Option<[u8; 32]> {
        let keys = self.0.lock().unwrap();
        (keys.len() == 1).then(|| *keys.values().next().unwrap())
    }
}

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
        sort: None,
        filter: None,
        tab: None,
        tile: None,
        last_view: None,
        layout: BridgeLayout::Table,
        tabs: Vec::new(),
        media_type: None,
        attachment: None,
        drawing_style: None,
        drawing: None,
        line: None,
        line_setup: None,
        total: None,
        chart: None,
        automation: None,
        automation_setup: None,
        on: false,
        role: None,
        role_setup: None,
        roles: Vec::new(),
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
        field: None,
        sort: None,
        filter: None,
        within: None,
        search: None,
        totals: Vec::new(),
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

fn areas(workspace: &LiquidWorkspace) -> i64 {
    workspace
        .query(query(BridgeQueryKind::Areas))
        .unwrap()
        .count
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
    assert_eq!(areas(&workspace), 1);
    assert!(moved.0.join(KEY_NAME_FILE).is_file());
}

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
    assert_eq!(areas(&workspace), 1);
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
fn a_path_entry_holding_another_key_is_kept() {
    let folder = Folder::new("other-key");
    let keys = Memory::default();
    lisbon(&LiquidWorkspace::open_with(&folder.0, &keys).unwrap());
    let path = path_name(&folder.0).unwrap();
    let other = [7; 32];
    keys.write(&path, &other).unwrap();

    assert!(LiquidWorkspace::open_with(&folder.0, &keys).is_ok());
    assert_eq!(keys.read(&path).unwrap(), Some(other));
}

struct PathUnreadable<'a>(&'a Memory, String);

impl Keys for PathUnreadable<'_> {
    fn read(&self, name: &str) -> Result<Option<[u8; 32]>, String> {
        if name == self.1 {
            return Err("the keychain is locked".into());
        }
        self.0.read(name)
    }

    fn write(&self, name: &str, key: &[u8; 32]) -> Result<(), String> {
        self.0.write(name, key)
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        self.0.delete(name)
    }
}

#[test]
fn a_failed_path_entry_lookup_does_not_block_the_key_named_by_the_folder() {
    let folder = Folder::new("path-unreadable");
    let keys = Memory::default();
    lisbon(&LiquidWorkspace::open_with(&folder.0, &keys).unwrap());
    let path = path_name(&folder.0).unwrap();

    LiquidWorkspace::open_with(&folder.0, &PathUnreadable(&keys, path))
        .expect("the key the folder names opens the replica");
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

#[cfg(unix)]
#[test]
fn a_key_name_that_is_no_regular_file_is_refused() {
    let folder = Folder::new("key-name-pipe");
    let keys = Memory::default();
    lisbon(&LiquidWorkspace::open_with(&folder.0, &keys).unwrap());
    let name = folder.0.join(KEY_NAME_FILE);
    let pipe = folder.0.join("pipe");
    let made = std::process::Command::new("mkfifo")
        .arg(&pipe)
        .status()
        .unwrap();
    assert!(made.success());
    let refusal = |dir: PathBuf| {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(LiquidWorkspace::open_with(&dir, &Memory::default()).err());
        });
        receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the open returns without waiting for a writer")
    };
    std::fs::remove_file(&name).unwrap();
    std::os::unix::fs::symlink(&pipe, &name).unwrap();
    let error = refusal(folder.0.clone()).expect("a link to a pipe is refused");
    assert!(error.contains("no plain file"), "{error}");
    std::fs::remove_file(&name).unwrap();
    std::fs::rename(&pipe, &name).unwrap();
    let error = refusal(folder.0.clone()).expect("a pipe is refused");
    assert!(error.contains("no plain file"), "{error}");
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

#[derive(Default)]
struct PausedLookup {
    keys: Memory,
    paused: std::sync::atomic::AtomicBool,
    reached: Option<std::sync::Barrier>,
    resume: Option<std::sync::Barrier>,
}

impl Keys for PausedLookup {
    fn read(&self, name: &str) -> Result<Option<[u8; 32]>, String> {
        let found = self.keys.read(name);
        if !self.paused.swap(true, std::sync::atomic::Ordering::SeqCst) {
            self.reached.as_ref().unwrap().wait();
            self.resume.as_ref().unwrap().wait();
        }
        found
    }

    fn write(&self, name: &str, key: &[u8; 32]) -> Result<(), String> {
        self.keys.write(name, key)
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        self.keys.delete(name)
    }
}

#[test]
fn a_second_session_cannot_make_a_key_while_the_first_is_making_one() {
    let folder = Folder::new("first-key");
    let keys = PausedLookup {
        reached: Some(std::sync::Barrier::new(2)),
        resume: Some(std::sync::Barrier::new(2)),
        ..PausedLookup::default()
    };
    let (first, second) = std::thread::scope(|scope| {
        let first = scope.spawn(|| LiquidWorkspace::open_with(&folder.0, &keys).map(drop).err());
        keys.reached.as_ref().unwrap().wait();
        let second = LiquidWorkspace::open_with(&folder.0, &keys).map(drop).err();
        keys.resume.as_ref().unwrap().wait();
        (first.join().unwrap(), second)
    });
    assert_eq!(first, None);
    assert_eq!(
        second.as_deref(),
        Some("another Liquid session holds this replica")
    );
    assert!(keys.keys.only().is_some());
    assert!(LiquidWorkspace::open_with(&folder.0, &keys.keys).is_ok());
}

#[test]
fn a_lock_held_only_for_a_moment_is_waited_for() {
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
    workspace.watch(1, areas(), |_| false).unwrap();
    workspace.close(1);
    let mut first = true;
    workspace
        .watch(2, areas(), move |_| std::mem::take(&mut first))
        .unwrap();
    lisbon(&workspace);
    workspace.close(2);
    workspace.close(3);
    workspace.watch(3, areas(), |_| true).unwrap();
    workspace.watch(4, areas(), |_| true).unwrap();
    workspace.close(4);
    let nameless = || resource(BridgeResourceKind::Views);
    assert!(workspace.watch(5, nameless(), |_| true).is_err());
    workspace.close(5);
    workspace.close(6);
    assert!(workspace.watch(6, nameless(), |_| true).is_err());

    assert_eq!(workspace.lifecycle_markers(), 0);
}

#[test]
fn text_blocks_split_and_join_through_the_bridge() {
    let folder = Folder::new("text");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    let (_, page, block, view) = lisbon(&workspace);
    let split = workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            block: Some(block.clone()),
            view: Some(view),
            at: 4,
            style: BridgeTextStyle::Todo,
            ..mutation(BridgeMutationKind::SplitTextBlock)
        })
        .unwrap();
    let (rest, rest_view) = (split.block.unwrap(), split.view.unwrap());
    assert_eq!(text(&workspace, &page, &block).as_deref(), Some("Five"));
    assert_eq!(text(&workspace, &page, &rest).as_deref(), Some(" days"));
    let style = workspace
        .query(BridgeQuery {
            page: Some(page.clone()),
            block: Some(rest.clone()),
            ..query(BridgeQueryKind::Text)
        })
        .unwrap()
        .style;
    assert_eq!(style, Some(BridgeTextStyle::Todo));

    let boundary = |on: bool| BridgeMutation {
        page: Some(page.clone()),
        view: Some(rest_view.clone()),
        boundary: on,
        ..mutation(BridgeMutationKind::SetSectionBoundary)
    };
    workspace.apply(boundary(true)).unwrap();
    let join = || BridgeMutation {
        page: Some(page.clone()),
        view: Some(rest_view.clone()),
        ..mutation(BridgeMutationKind::JoinTextBlock)
    };
    assert_eq!(
        workspace.apply(join()).unwrap_err(),
        "the Text Block starts a Section"
    );
    workspace.apply(boundary(false)).unwrap();
    workspace.apply(join()).unwrap();
    assert_eq!(
        text(&workspace, &page, &block).as_deref(),
        Some("Five days")
    );
    assert_eq!(text(&workspace, &page, &rest), None);
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

#[test]
fn a_closing_workspace_keeps_its_replica_lock_until_its_database_has_closed() {
    struct Probe(PathBuf, Arc<Mutex<Option<bool>>>);
    impl Drop for Probe {
        fn drop(&mut self) {
            let lock = std::fs::File::open(self.0.join("replica.lock")).unwrap();
            let free = lock.try_lock().is_ok();
            *self.1.lock().unwrap() = Some(free);
        }
    }
    let folder = Folder::new("closing");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    let free: Arc<Mutex<Option<bool>>> = Arc::default();
    let probe = Probe(folder.0.clone(), free.clone());
    workspace
        .watch(1, resource(BridgeResourceKind::Areas), move |_| {
            let _ = &probe;
            true
        })
        .unwrap();
    drop(workspace);
    assert_eq!(*free.lock().unwrap(), Some(false), "the lock is still held");
}

#[test]
fn a_workspace_whose_call_failed_midway_refuses_every_later_call() {
    let folder = Folder::new("poisoned");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    let mut registered = false;
    workspace
        .watch(1, resource(BridgeResourceKind::Areas), move |_| {
            assert!(!registered, "a subscriber failed");
            registered = true;
            true
        })
        .unwrap();
    let create = || BridgeMutation {
        text: Some("Trips".into()),
        ..mutation(BridgeMutationKind::CreateArea)
    };
    let failed =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| workspace.apply(create())));
    assert!(failed.is_err());

    let stopped = "the Workspace stopped after an internal error; quit and reopen Liquid";
    assert_eq!(workspace.apply(create()).err().unwrap(), stopped);
    assert_eq!(
        workspace
            .query(query(BridgeQueryKind::Areas))
            .err()
            .unwrap(),
        stopped
    );
    let watched = workspace.watch(2, resource(BridgeResourceKind::Areas), |_| true);
    assert_eq!(watched.err().unwrap(), stopped);
}

#[test]
fn a_tile_and_a_move_cross_the_bridge() {
    let folder = Folder::new("arrange");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    let (_, page, _, first) = lisbon(&workspace);
    let second = workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            after: Some(first.clone()),
            ..mutation(BridgeMutationKind::AddWhiteboardBlock)
        })
        .unwrap()
        .view
        .unwrap();
    let tile = BridgeTile { column: 4, span: 8 };
    for write in [
        BridgeMutation {
            view: Some(second.clone()),
            tile: Some(tile),
            ..mutation(BridgeMutationKind::SetTile)
        },
        BridgeMutation {
            view: Some(first.clone()),
            tile: Some(tile),
            ..mutation(BridgeMutationKind::SetInnerTile)
        },
        BridgeMutation {
            view: Some(second.clone()),
            last_view: Some(second.clone()),
            ..mutation(BridgeMutationKind::MoveViews)
        },
    ] {
        workspace
            .apply(BridgeMutation {
                page: Some(page.clone()),
                ..write
            })
            .unwrap();
    }
    let answer = workspace
        .query(BridgeQuery {
            page: Some(page),
            ..query(BridgeQueryKind::Views)
        })
        .unwrap();
    assert!(answer.arranged);
    let shown: Vec<_> = answer
        .views
        .iter()
        .map(|view| (view.id.clone(), view.tile, view.inner))
        .collect();
    assert_eq!(
        shown,
        [
            (second.clone(), Some(tile), None),
            (first.clone(), None, Some(tile))
        ]
    );
}

#[test]
fn search_finds_pages_and_text_across_the_bridge() {
    let folder = Folder::new("search");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    let (_, page, block, _) = lisbon(&workspace);
    let hits = workspace
        .query(BridgeQuery {
            search: Some("five da".into()),
            limit: 5,
            ..query(BridgeQueryKind::Search)
        })
        .unwrap()
        .hits;
    assert_eq!(
        hits.iter()
            .map(|hit| (hit.found, hit.id.as_str(), hit.page.as_deref()))
            .collect::<Vec<_>>(),
        [(BridgeFound::Text, block.as_str(), Some(page.as_str()))]
    );
    let pages = workspace
        .query(BridgeQuery {
            search: Some("lis".into()),
            ..query(BridgeQueryKind::Search)
        })
        .unwrap()
        .hits;
    assert_eq!(pages[0].found, BridgeFound::Page);
}
