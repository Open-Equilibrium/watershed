//! The document schema: where each object lives and how it is read.
//!
//! ```text
//! Workspace document  { owner, areas: { <area>: { name, sort, deleted } } }
//! area document       { pages: { <page>: { title, parent?, sort, deleted } } }
//! Page document       { blocks: { <block>: { kind, text, style, deleted } },
//!                       (a Database Block's entry is in `database.rs`)
//!                       views: { <view>: { block, sort, boundary, deleted } } }
//! ```

use std::collections::HashMap;

use automerge::transaction::Transactable;
use automerge::{
    ActorId, AutoCommit, LoadOptions, ObjId, ObjType, ROOT, ReadDoc, ScalarValue, ScalarValueRef,
    TextEncoding, Value, ValueRef,
};

use crate::{AreaId, BlockId, Error, PageId, TextStyle, ViewId};

pub(crate) const OWNER: &str = "owner";
pub(crate) const AREAS: &str = "areas";
pub(crate) const PAGES: &str = "pages";
pub(crate) const BLOCKS: &str = "blocks";
pub(crate) const VIEWS: &str = "views";
pub(crate) const NAME: &str = "name";
pub(crate) const TITLE: &str = "title";
pub(crate) const PARENT: &str = "parent";
pub(crate) const SORT: &str = "sort";
pub(crate) const DELETED: &str = "deleted";
pub(crate) const KIND: &str = "kind";
pub(crate) const TEXT: &str = "text";
pub(crate) const BLOCK: &str = "block";
pub(crate) const STYLE: &str = "style";
pub(crate) const BOUNDARY: &str = "boundary";

/// An area as the Explorer lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Area {
    /// Its identity.
    pub id: AreaId,
    /// Its name.
    pub name: String,
}

/// A Page in an area's tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageEntry {
    /// Its identity.
    pub id: PageId,
    /// The Page it nests under.
    pub parent: Option<PageId>,
    /// Its title.
    pub title: String,
}

/// A View on a Page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    /// Its identity.
    pub id: ViewId,
    /// The Block it shows.
    pub block: BlockId,
    /// Whether a Section starts here.
    pub boundary: bool,
    /// The kind of Block it shows.
    pub kind: BlockKind,
}

/// The kind of a Block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    /// A Text Block.
    Text,
    /// A Database Block.
    Database,
    /// A Block of a kind a later version adds, which shows as a placeholder (ADR-0245).
    Unknown,
}

impl BlockKind {
    /// The kind a Block's entry names.
    pub(crate) fn of(name: Option<&str>) -> Self {
        match name {
            Some("text") => Self::Text,
            Some("database") => Self::Database,
            _ => Self::Unknown,
        }
    }
}

/// A new document for this device. Text positions count UTF-16 code units, also when
/// [`load`] reads it back.
pub(crate) fn document(device: &ActorId) -> AutoCommit {
    AutoCommit::new_with_encoding(TextEncoding::Utf16CodeUnit).with_actor(device.clone())
}

/// A stored document, loaded for this device.
pub(crate) fn load(bytes: &[u8], device: &ActorId) -> Result<AutoCommit, Error> {
    let options = LoadOptions::new().text_encoding(TextEncoding::Utf16CodeUnit);
    Ok(AutoCommit::load_with_options(bytes, options)?.with_actor(device.clone()))
}

/// Creates the empty top-level maps of a new document.
pub(crate) fn create_maps(doc: &mut AutoCommit, keys: &[&str]) -> Result<(), Error> {
    for key in keys {
        doc.put_object(ROOT, *key, ObjType::Map)?;
    }
    Ok(())
}

/// Adds `id` to the top-level map `map` as a live object.
pub(crate) fn insert(
    doc: &mut AutoCommit,
    map: &str,
    id: &str,
    fields: &[(&str, &str)],
) -> Result<ObjId, Error> {
    let map = top(doc, map)?;
    insert_in(doc, &map, id, fields)
}

/// Adds `id` to the map `map` as a live object.
pub(crate) fn insert_in(
    doc: &mut AutoCommit,
    map: &ObjId,
    id: &str,
    fields: &[(&str, &str)],
) -> Result<ObjId, Error> {
    let entry = doc.put_object(map, id, ObjType::Map)?;
    for (key, value) in fields {
        doc.put(&entry, *key, *value)?;
    }
    doc.put(&entry, DELETED, false)?;
    Ok(entry)
}

fn top(doc: &AutoCommit, map: &str) -> Result<ObjId, Error> {
    child(doc, &ROOT, map).ok_or_else(|| Error::Document(format!("no top-level map {map}")))
}

pub(crate) fn child(doc: &AutoCommit, parent: &ObjId, key: &str) -> Option<ObjId> {
    match doc.get(parent, key).ok()? {
        Some((Value::Object(_), id)) => Some(id),
        _ => None,
    }
}

pub(crate) fn string(doc: &AutoCommit, obj: &ObjId, key: &str) -> Option<String> {
    match doc.get(obj, key).ok()? {
        Some((Value::Scalar(value), _)) => match value.as_ref() {
            ScalarValue::Str(text) => Some(text.to_string()),
            _ => None,
        },
        _ => None,
    }
}

pub(crate) fn deleted(doc: &AutoCommit, obj: &ObjId) -> bool {
    !matches!(
        doc.get(obj, DELETED),
        Ok(Some((Value::Scalar(value), _))) if value.as_ref() == &ScalarValue::Boolean(false)
    )
}

/// The live object `id` in the top-level map `map`.
pub(crate) fn live(doc: &AutoCommit, map: &str, id: &str) -> Option<ObjId> {
    live_in(doc, &child(doc, &ROOT, map)?, id)
}

/// The live object `id` in the map `map`.
pub(crate) fn live_in(doc: &AutoCommit, map: &ObjId, id: &str) -> Option<ObjId> {
    let obj = child(doc, map, id)?;
    (!deleted(doc, &obj)).then_some(obj)
}

/// A live object of a top-level map with its text fields, read in one pass: Automerge reads
/// a whole map faster than it looks up its keys one by one.
pub(crate) struct Entry {
    pub id: String,
    fields: Vec<(String, String)>,
}

impl Entry {
    pub(crate) fn get(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(field, _)| field == key)
            .map(|(_, value)| value.as_str())
    }
}

/// The live objects in the top-level map `map`, ordered by sort key, then identity.
pub(crate) fn sorted(doc: &AutoCommit, map: &str) -> Vec<Entry> {
    match child(doc, &ROOT, map) {
        Some(top) => sorted_in(doc, &top),
        None => Vec::new(),
    }
}

/// The live objects in the map `map`, ordered by sort key, then identity.
pub(crate) fn sorted_in(doc: &AutoCommit, map: &ObjId) -> Vec<Entry> {
    let mut entries: Vec<Entry> = doc
        .map_range(map, ..)
        .filter(|item| matches!(item.value, ValueRef::Object(ObjType::Map)))
        .filter_map(|item| {
            let obj = item.id();
            let mut live = false;
            let mut fields = Vec::new();
            for field in doc.map_range(&obj, ..) {
                match field.value {
                    ValueRef::Scalar(ScalarValueRef::Str(value)) => {
                        fields.push((field.key.to_string(), value.to_string()));
                    }
                    ValueRef::Scalar(ScalarValueRef::Boolean(false)) if field.key == DELETED => {
                        live = true;
                    }
                    ValueRef::Scalar(ScalarValueRef::Boolean(value)) => {
                        fields.push((field.key.to_string(), value.to_string()));
                    }
                    _ => {}
                }
            }
            live.then(|| Entry {
                id: item.key.to_string(),
                fields,
            })
        })
        .collect();
    entries.sort_by(|a, b| (a.get(SORT), &a.id).cmp(&(b.get(SORT), &b.id)));
    entries
}

pub(crate) fn mark_deleted(doc: &mut AutoCommit, obj: &ObjId) -> Result<(), Error> {
    Ok(doc.put(obj, DELETED, true)?)
}

/// The sort key for an object placed after `after` among `siblings`, or first.
pub(crate) fn sort_after(siblings: &[Entry], after: Option<&str>) -> Result<String, Error> {
    let index = match after {
        None => 0,
        Some(after) => {
            siblings
                .iter()
                .position(|entry| entry.id == after)
                .ok_or(Error::NotFound)?
                + 1
        }
    };
    let key = |i: usize| siblings.get(i).and_then(|entry| entry.get(SORT));
    Ok(crate::sort_key::between(
        index.checked_sub(1).and_then(key),
        key(index),
    ))
}

pub(crate) fn areas(doc: &AutoCommit) -> Vec<Area> {
    sorted(doc, AREAS)
        .into_iter()
        .map(|entry| Area {
            id: AreaId::stored(&entry.id),
            name: entry.get(NAME).unwrap_or_default().to_string(),
        })
        .collect()
}

/// An area's live Pages, each followed by its descendants. A Page under a removed Page is
/// hidden with it.
pub(crate) fn pages(doc: &AutoCommit) -> Vec<PageEntry> {
    let mut children: HashMap<Option<String>, Vec<PageEntry>> = HashMap::new();
    for entry in sorted(doc, PAGES) {
        let parent = entry.get(PARENT).map(str::to_string);
        children.entry(parent.clone()).or_default().push(PageEntry {
            id: PageId::stored(&entry.id),
            parent: parent.as_deref().map(PageId::stored),
            title: entry.get(TITLE).unwrap_or_default().to_string(),
        });
    }
    let mut tree = Vec::new();
    let mut pending: Vec<PageEntry> = children.remove(&None).unwrap_or_default();
    pending.reverse();
    while let Some(entry) = pending.pop() {
        if let Some(mut nested) = children.remove(&Some(entry.id.as_str().to_string())) {
            nested.reverse();
            pending.extend(nested);
        }
        tree.push(entry);
    }
    tree
}

/// A live Text Block's text; none for a Block of another kind.
pub(crate) fn text(doc: &AutoCommit, block: &str) -> Option<String> {
    let entry = live(doc, BLOCKS, block)?;
    if string(doc, &entry, KIND).as_deref() != Some("text") {
        return None;
    }
    let text = child(doc, &entry, TEXT)?;
    doc.text(&text).ok()
}

/// Whether `at` lies at an end of `text` or between two of its characters, in UTF-16
/// units: inside a character both neighboring units belong to the same element.
pub(crate) fn on_boundary(doc: &AutoCommit, text: &ObjId, at: usize) -> bool {
    let element = |unit: usize| doc.get(text, unit).ok().flatten().map(|(_, id)| id);
    let length = doc.length(text);
    at == 0 || at == length || (at < length && element(at - 1) != element(at))
}

/// A style as a Text Block stores it.
pub(crate) fn style_name(style: TextStyle) -> Result<String, Error> {
    Ok(match style {
        TextStyle::Paragraph => "paragraph".into(),
        TextStyle::Heading(level @ 1..=3) => format!("heading{level}"),
        TextStyle::Heading(_) => return Err(Error::OutOfRange),
        TextStyle::Bullet => "bullet".into(),
        TextStyle::Numbered => "numbered".into(),
        TextStyle::Todo { done: false } => "todo".into(),
        TextStyle::Todo { done: true } => "done".into(),
        TextStyle::Quote => "quote".into(),
    })
}

/// A stored style; a Text Block without one, or with one this version does not know, reads
/// as a paragraph.
pub(crate) fn style(name: Option<&str>) -> TextStyle {
    match name {
        Some("heading1") => TextStyle::Heading(1),
        Some("heading2") => TextStyle::Heading(2),
        Some("heading3") => TextStyle::Heading(3),
        Some("bullet") => TextStyle::Bullet,
        Some("numbered") => TextStyle::Numbered,
        Some("todo") => TextStyle::Todo { done: false },
        Some("done") => TextStyle::Todo { done: true },
        Some("quote") => TextStyle::Quote,
        _ => TextStyle::Paragraph,
    }
}
