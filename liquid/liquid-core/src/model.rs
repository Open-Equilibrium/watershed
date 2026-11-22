use std::collections::HashMap;

use automerge::transaction::Transactable;
use automerge::{
    ActorId, AutoCommit, LoadOptions, ObjId, ObjType, ROOT, ReadDoc, ScalarValue, ScalarValueRef,
    TextEncoding, Value, ValueRef,
};

use crate::{AreaId, AttachmentId, BlockId, Error, Media, PageId, Place, TextStyle, ViewId};

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
pub(crate) const ROW_SORT: &str = "row-sort";
pub(crate) const ROW_FILTER: &str = "row-filter";
pub(crate) const LANGUAGE: &str = "language";
pub(crate) const TYPE: &str = "type";
pub(crate) const ATTACHMENT: &str = "attachment";
pub(crate) const SIZE: &str = "size";
pub(crate) const TABLES: &str = "tables";
pub(crate) const TABLE: &str = "table";
pub(crate) const AREA: &str = "area";
pub(crate) const PAGE: &str = "page";
pub(crate) const ICON: &str = "icon";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Area {
    pub id: AreaId,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageEntry {
    pub id: PageId,
    pub parent: Option<PageId>,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableEntry {
    pub id: BlockId,
    pub area: AreaId,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub id: ViewId,
    pub block: BlockId,
    pub title: String,
    pub boundary: bool,
    pub kind: BlockKind,
    pub tabs: Vec<crate::Tab>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Text,
    Database,
    Code,
    Media,
    Whiteboard,
    Unknown,
}

impl BlockKind {
    pub(crate) fn of(name: Option<&str>) -> Self {
        match name {
            Some("text") => Self::Text,
            Some("database") => Self::Database,
            Some("code") => Self::Code,
            Some("media") => Self::Media,
            Some("whiteboard") => Self::Whiteboard,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Code {
    pub text: String,
    pub language: String,
}

pub(crate) fn document(device: &ActorId) -> AutoCommit {
    AutoCommit::new_with_encoding(TextEncoding::Utf16CodeUnit).with_actor(device.clone())
}

pub(crate) fn load(bytes: &[u8], device: &ActorId) -> Result<AutoCommit, Error> {
    let options = LoadOptions::new().text_encoding(TextEncoding::Utf16CodeUnit);
    Ok(AutoCommit::load_with_options(bytes, options)?.with_actor(device.clone()))
}

pub(crate) fn create_maps(doc: &mut AutoCommit, keys: &[&str]) -> Result<(), Error> {
    for key in keys {
        doc.put_object(ROOT, *key, ObjType::Map)?;
    }
    Ok(())
}

pub(crate) fn insert(
    doc: &mut AutoCommit,
    map: &str,
    id: &str,
    fields: &[(&str, &str)],
) -> Result<ObjId, Error> {
    let map = top(doc, map)?;
    insert_in(doc, &map, id, fields)
}

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

pub(crate) fn live(doc: &AutoCommit, map: &str, id: &str) -> Option<ObjId> {
    live_in(doc, &child(doc, &ROOT, map)?, id)
}

pub(crate) fn live_in(doc: &AutoCommit, map: &ObjId, id: &str) -> Option<ObjId> {
    let obj = child(doc, map, id)?;
    (!deleted(doc, &obj)).then_some(obj)
}

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

    pub(crate) fn sort(&self) -> Option<&str> {
        crate::sort_key::usable(self.get(SORT))
    }
}

pub(crate) fn sorted(doc: &AutoCommit, map: &str) -> Vec<Entry> {
    match child(doc, &ROOT, map) {
        Some(top) => sorted_in(doc, &top),
        None => Vec::new(),
    }
}

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
    entries.sort_by(|a, b| (a.sort(), &a.id).cmp(&(b.sort(), &b.id)));
    entries
}

pub(crate) fn mark_deleted(doc: &mut AutoCommit, obj: &ObjId) -> Result<(), Error> {
    Ok(doc.put(obj, DELETED, true)?)
}

pub(crate) fn sort_after(siblings: &[Entry], after: Option<&str>) -> Result<String, Error> {
    sort_at(siblings, after.map_or(Place::First, Place::After))
}

pub(crate) fn sort_at(siblings: &[Entry], place: Place<&str>) -> Result<String, Error> {
    crate::sort_key::at(siblings, place, |entry| entry.id.as_str(), Entry::sort)
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

pub(crate) fn tables(doc: &AutoCommit) -> Vec<TableEntry> {
    sorted(doc, TABLES)
        .into_iter()
        .map(|entry| TableEntry {
            id: BlockId::stored(&entry.id),
            area: AreaId::stored(entry.get(AREA).unwrap_or_default()),
            name: entry.get(NAME).unwrap_or_default().to_string(),
        })
        .collect()
}

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

pub(crate) fn text(doc: &AutoCommit, block: &str) -> Option<String> {
    let entry = of_kind(doc, block, BlockKind::Text)?;
    doc.text(&child(doc, &entry, TEXT)?).ok()
}

pub(crate) fn code(doc: &AutoCommit, block: &str) -> Option<Code> {
    let entry = of_kind(doc, block, BlockKind::Code)?;
    Some(Code {
        text: doc.text(&child(doc, &entry, TEXT)?).ok()?,
        language: string(doc, &entry, LANGUAGE).unwrap_or_default(),
    })
}

pub(crate) fn media(doc: &AutoCommit, block: &str) -> Option<Media> {
    file(doc, &of_kind(doc, block, BlockKind::Media)?)
}

pub(crate) fn file(doc: &AutoCommit, obj: &ObjId) -> Option<Media> {
    let size = match doc.get(obj, SIZE).ok()? {
        Some((Value::Scalar(value), _)) => match value.as_ref() {
            ScalarValue::Uint(size) => *size,
            _ => return None,
        },
        _ => return None,
    };
    Some(Media {
        name: string(doc, obj, NAME)?,
        media_type: string(doc, obj, TYPE).unwrap_or_default(),
        attachment: AttachmentId::parse(&string(doc, obj, ATTACHMENT)?)?,
        size,
    })
}

pub(crate) fn put_file(doc: &mut AutoCommit, obj: &ObjId, file: &Media) -> Result<(), Error> {
    doc.put(obj, NAME, file.name.as_str())?;
    doc.put(obj, TYPE, file.media_type.as_str())?;
    doc.put(obj, ATTACHMENT, file.attachment.as_str())?;
    doc.put(obj, SIZE, file.size)?;
    Ok(())
}

pub(crate) fn of_kind(doc: &AutoCommit, block: &str, kind: BlockKind) -> Option<ObjId> {
    let entry = live(doc, BLOCKS, block)?;
    (BlockKind::of(string(doc, &entry, KIND).as_deref()) == kind && kind != BlockKind::Unknown)
        .then_some(entry)
}

pub(crate) fn on_boundary(doc: &AutoCommit, text: &ObjId, at: usize) -> bool {
    let element = |unit: usize| doc.get(text, unit).ok().flatten().map(|(_, id)| id);
    let length = doc.length(text);
    at == 0 || at == length || (at < length && element(at - 1) != element(at))
}

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
