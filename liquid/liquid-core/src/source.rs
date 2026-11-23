use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use automerge::transaction::Transactable;
use automerge::{AutoCommit, ObjType, ROOT, ReadDoc, ScalarValueRef, ValueRef};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::model::{self, DELETED, KIND, NAME, PAGES, PARENT, SORT};
use crate::store::Store;
use crate::workspace::Written;
use crate::{
    Actor, Applied, AreaId, BlockId, Changed, ChartSource, Created, Document, Error, FieldId,
    IndexEntry, IndexWrite, Mutation, Origin, PageId, Request, SourceId, Workspace,
};

pub const MAX_SOURCE_FILE: u64 = 50 * 1024 * 1024;

pub(crate) const SOURCES: &str = "sources";
const USED: &str = "used:";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    File,
    Folder,
}

impl SourceKind {
    fn key(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Folder => "folder",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceEntry {
    pub id: SourceId,
    pub parent: Option<PageId>,
    pub name: String,
    pub kind: SourceKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub folder: bool,
    pub size: u64,
    pub modified: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub id: FieldId,
    pub name: String,
    pub number: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceTable {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shown {
    Table(SourceTable),
    Text(String),
    Image,
    Pdf,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceFile {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified: Option<i64>,
    pub shown: Shown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceUser {
    pub page: PageId,
    pub block: BlockId,
}

pub(crate) fn source_entry(source: &SourceId, name: Option<&str>) -> IndexWrite {
    IndexWrite {
        found: crate::Found::Source,
        id: source.to_string(),
        entry: name.map(|name| IndexEntry {
            page: None,
            text: name.to_string(),
        }),
    }
}

pub(crate) fn column_id(name: &str) -> FieldId {
    let digest = Sha256::digest(name.as_bytes());
    let hex: String = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    FieldId::stored(&hex)
}

pub(crate) fn number(text: &str) -> Option<f64> {
    let text = text.trim();
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    let grouped = whole.contains(',');
    let groups: Vec<&str> = whole.split(',').collect();
    let whole_ok = if grouped {
        (1..=3).contains(&groups[0].len())
            && groups[1..].iter().all(|group| group.len() == 3)
            && groups
                .iter()
                .all(|group| group.bytes().all(|byte| byte.is_ascii_digit()))
    } else {
        !whole.is_empty() && whole.bytes().all(|byte| byte.is_ascii_digit())
    };
    let fraction_ok = fraction.bytes().all(|byte| byte.is_ascii_digit())
        && !(digits.contains('.') && fraction.is_empty());
    if !whole_ok || !fraction_ok {
        return None;
    }
    text.replace(',', "").parse().ok()
}

fn parse_csv(text: &str, separator: char) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(next) = chars.next() {
        match next {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                cell.push('"');
            }
            '"' if quoted => quoted = false,
            '"' if cell.is_empty() => quoted = true,
            '\r' if !quoted => {}
            '\n' if !quoted => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            separator_char if !quoted && separator_char == separator => {
                row.push(std::mem::take(&mut cell));
            }
            other => cell.push(other),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows.retain(|row| !(row.len() == 1 && row[0].is_empty()));
    rows
}

pub(crate) fn table(text: &str, tab: bool) -> SourceTable {
    let first = text.lines().next().unwrap_or_default();
    let separator = if tab {
        '\t'
    } else if first.matches(';').count() > first.matches(',').count() {
        ';'
    } else {
        ','
    };
    let mut rows = parse_csv(text.strip_prefix('\u{feff}').unwrap_or(text), separator);
    let header = if rows.is_empty() {
        Vec::new()
    } else {
        rows.remove(0)
    };
    let width = rows.iter().map(Vec::len).fold(header.len(), usize::max);
    for row in &mut rows {
        row.resize(width, String::new());
    }
    let columns = (0..width)
        .map(|at| {
            let name = header
                .get(at)
                .filter(|name| !name.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| format!("Column {}", at + 1));
            let mut filled = rows
                .iter()
                .map(|row| row[at].trim())
                .filter(|cell| !cell.is_empty());
            let first = filled.next();
            let number = first.is_some_and(|cell| number(cell).is_some())
                && filled.all(|cell| number(cell).is_some());
            Column {
                id: column_id(&name),
                name,
                number,
            }
        })
        .collect();
    SourceTable { columns, rows }
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn modified(metadata: &std::fs::Metadata) -> Option<i64> {
    let since = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(since.as_millis()).ok()
}

fn read_capped(path: &Path) -> Result<Vec<u8>, Error> {
    let file = std::fs::File::open(path).map_err(|_| Error::NotFound)?;
    let mut bytes = Vec::new();
    file.take(MAX_SOURCE_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::NotFound)?;
    if bytes.len() as u64 > MAX_SOURCE_FILE {
        return Err(Error::TooLarge);
    }
    Ok(bytes)
}

fn shown(path: &Path, size: u64) -> Result<Shown, Error> {
    let extension = extension(path);
    let text = |path: &Path| -> Result<String, Error> {
        let bytes = read_capped(path)?;
        String::from_utf8(bytes).map_err(|_| Error::WrongKind)
    };
    Ok(match extension.as_str() {
        "csv" | "tsv" => Shown::Table(table(&text(path)?, extension == "tsv")),
        "txt" | "md" | "markdown" => Shown::Text(text(path)?),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" if size <= MAX_SOURCE_FILE => Shown::Image,
        "pdf" if size <= MAX_SOURCE_FILE => Shown::Pdf,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "pdf" => return Err(Error::TooLarge),
        _ => Shown::Other,
    })
}

fn within(root: &Path, inside: &str) -> Result<PathBuf, Error> {
    if inside.is_empty() {
        return Ok(root.to_path_buf());
    }
    let relative = Path::new(inside);
    if !relative
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(Error::NotFound);
    }
    let base = root.canonicalize().map_err(|_| Error::NotFound)?;
    let path = base
        .join(relative)
        .canonicalize()
        .map_err(|_| Error::NotFound)?;
    if !path.starts_with(&base) {
        return Err(Error::NotFound);
    }
    Ok(path)
}

fn sources_map(doc: &mut AutoCommit) -> Result<automerge::ObjId, Error> {
    match model::child(doc, &ROOT, SOURCES) {
        Some(map) => Ok(map),
        None => Ok(doc.put_object(ROOT, SOURCES, ObjType::Map)?),
    }
}

pub(crate) fn entries(doc: &AutoCommit) -> Vec<SourceEntry> {
    model::sorted(doc, SOURCES)
        .into_iter()
        .map(|entry| SourceEntry {
            id: SourceId::stored(&entry.id),
            parent: entry.get(PARENT).map(PageId::stored),
            name: entry.get(NAME).unwrap_or_default().to_string(),
            kind: if entry.get(KIND) == Some("folder") {
                SourceKind::Folder
            } else {
                SourceKind::File
            },
        })
        .collect()
}

pub(crate) fn all_ids(doc: &AutoCommit) -> Vec<String> {
    model::child(doc, &ROOT, SOURCES).map_or_else(Vec::new, |map| {
        doc.map_range(&map, ..)
            .map(|item| item.key.to_string())
            .collect()
    })
}

impl<S: Store> Workspace<S> {
    pub fn sources(&mut self, area: &AreaId) -> Result<Vec<SourceEntry>, Error> {
        Ok(entries(self.doc(&Document::Area(area.clone()))?))
    }

    pub fn source_path(&mut self, source: &SourceId) -> Result<Option<String>, Error> {
        self.store.source_path(source)
    }

    pub fn add_source(
        &mut self,
        actor: &Actor,
        area: &AreaId,
        parent: Option<&PageId>,
        path: &str,
    ) -> Result<Applied, Error> {
        let metadata = std::fs::metadata(path).map_err(|_| Error::NotFound)?;
        let name = Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(Error::NotFound)?
            .to_string();
        let kind = if metadata.is_dir() {
            SourceKind::Folder
        } else {
            SourceKind::File
        };
        let applied = self.apply(Request {
            actor: actor.clone(),
            origin: Origin::Ui,
            mutation: Mutation::AddSource {
                area: area.clone(),
                parent: parent.cloned(),
                name,
                kind,
            },
        })?;
        if let Some(Created::Source(source)) = &applied.created {
            self.store.locate(source, Some(path))?;
        }
        Ok(applied)
    }

    pub fn locate_source(
        &mut self,
        actor: &Actor,
        source: &SourceId,
        path: &str,
    ) -> Result<(), Error> {
        self.permit(actor)?;
        self.area_of_source(source)?;
        std::fs::metadata(path).map_err(|_| Error::NotFound)?;
        self.store.locate(source, Some(path))
    }

    fn root(&mut self, source: &SourceId) -> Result<PathBuf, Error> {
        self.area_of_source(source)?;
        let path = self.store.source_path(source)?.ok_or(Error::NotFound)?;
        Ok(PathBuf::from(path))
    }

    pub fn source_entries(
        &mut self,
        source: &SourceId,
        inside: &str,
    ) -> Result<Vec<FileEntry>, Error> {
        let folder = within(&self.root(source)?, inside)?;
        let mut listed = Vec::new();
        for item in std::fs::read_dir(&folder).map_err(|_| Error::NotFound)? {
            let Ok(item) = item else { continue };
            let name = item.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(metadata) = item.metadata() else {
                continue;
            };
            listed.push(FileEntry {
                folder: metadata.is_dir(),
                size: if metadata.is_dir() { 0 } else { metadata.len() },
                modified: modified(&metadata),
                name,
            });
        }
        listed.sort_by(|a, b| {
            b.folder
                .cmp(&a.folder)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(listed)
    }

    pub fn source_file(&mut self, source: &SourceId, inside: &str) -> Result<SourceFile, Error> {
        let root = self.root(source)?;
        let path = within(&root, inside)?;
        let metadata = std::fs::metadata(&path).map_err(|_| Error::NotFound)?;
        if metadata.is_dir() {
            return Err(Error::WrongKind);
        }
        Ok(SourceFile {
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: path.to_string_lossy().into_owned(),
            size: metadata.len(),
            modified: modified(&metadata),
            shown: shown(&path, metadata.len())?,
        })
    }

    pub(crate) fn source_table(
        &mut self,
        source: &SourceId,
    ) -> Result<(String, SourceTable), Error> {
        let file = self.source_file(source, "")?;
        match file.shown {
            Shown::Table(table) => Ok((file.name, table)),
            _ => Err(Error::WrongKind),
        }
    }

    pub fn source_users(&mut self, source: &SourceId) -> Result<Vec<SourceUser>, Error> {
        let area = self.area_of_source(source)?;
        let doc = self.doc(&Document::Area(area))?;
        let map = model::child(doc, &ROOT, SOURCES).ok_or(Error::NotFound)?;
        let entry = model::child(doc, &map, source.as_str()).ok_or(Error::NotFound)?;
        let used: Vec<(BlockId, PageId)> = doc
            .map_range(&entry, ..)
            .filter_map(|item| {
                let block = item.key.strip_prefix(USED)?;
                let ValueRef::Scalar(ScalarValueRef::Str(page)) = item.value else {
                    return None;
                };
                Some((BlockId::stored(block), PageId::stored(&page)))
            })
            .collect();
        let mut users = Vec::new();
        for (block, page) in used {
            let reads = match self.chart(&page, &block) {
                Ok(Some(chart)) => chart.setup.source == Some(ChartSource::Source(source.clone())),
                _ => false,
            };
            if reads
                && self
                    .views(&page, 0, usize::MAX)
                    .is_ok_and(|views| views.iter().any(|view| view.block == block))
            {
                users.push(SourceUser { page, block });
            }
        }
        Ok(users)
    }

    pub(crate) fn area_of_source(&mut self, source: &SourceId) -> Result<AreaId, Error> {
        for area in model::areas(self.doc(&Document::Workspace)?) {
            let doc = self.doc(&Document::Area(area.id.clone()))?;
            if let Some(map) = model::child(doc, &ROOT, SOURCES)
                && model::live_in(doc, &map, source.as_str()).is_some()
            {
                return Ok(area.id);
            }
        }
        Err(Error::NotFound)
    }

    pub(crate) fn any_source_name(&mut self, source: &SourceId) -> Option<String> {
        for area in model::areas(self.doc(&Document::Workspace).ok()?) {
            let doc = self.doc(&Document::Area(area.id)).ok()?;
            let map = model::child(doc, &ROOT, SOURCES)?;
            if let Some(entry) = model::child(doc, &map, source.as_str()) {
                return model::string(doc, &entry, NAME);
            }
        }
        None
    }

    pub(crate) fn source_name(&mut self, source: &SourceId) -> Result<String, Error> {
        let area = self.area_of_source(source)?;
        let doc = self.doc(&Document::Area(area))?;
        let map = model::child(doc, &ROOT, SOURCES).ok_or(Error::NotFound)?;
        let entry = model::live_in(doc, &map, source.as_str()).ok_or(Error::NotFound)?;
        Ok(model::string(doc, &entry, NAME).unwrap_or_default())
    }

    pub(crate) fn write_add_source(
        &mut self,
        area: &AreaId,
        parent: Option<&PageId>,
        name: &str,
        kind: SourceKind,
    ) -> Result<Written, Error> {
        let doc = self.doc(&Document::Area(area.clone()))?;
        if let Some(parent) = parent {
            model::live(doc, PAGES, parent.as_str()).ok_or(Error::NotFound)?;
        }
        let map = sources_map(doc)?;
        let siblings: Vec<_> = model::sorted_in(doc, &map)
            .into_iter()
            .filter(|entry| entry.get(PARENT) == parent.map(PageId::as_str))
            .collect();
        let last = siblings.last().map(|entry| entry.id.as_str());
        let sort = model::sort_after(&siblings, last)?;
        let id = SourceId::new()?;
        let mut fields = vec![(NAME, name), (KIND, kind.key()), (SORT, &sort)];
        if let Some(parent) = parent {
            fields.push((PARENT, parent.as_str()));
        }
        model::insert_in(doc, &map, id.as_str(), &fields)?;
        let mut written = Written::new(Document::Area(area.clone()), Changed::Pages(area.clone()));
        written.created = Some(Created::Source(id));
        Ok(written)
    }

    pub(crate) fn write_source(
        &mut self,
        source: &SourceId,
        name: Option<&str>,
    ) -> Result<Written, Error> {
        let area = self.area_of_source(source)?;
        let doc = self.doc(&Document::Area(area.clone()))?;
        let map = sources_map(doc)?;
        let entry = model::live_in(doc, &map, source.as_str()).ok_or(Error::NotFound)?;
        match name {
            Some(name) => doc.put(&entry, NAME, name)?,
            None => doc.put(&entry, DELETED, true)?,
        }
        Ok(Written::new(
            Document::Area(area.clone()),
            Changed::Pages(area),
        ))
    }

    pub(crate) fn use_source(
        &mut self,
        source: &SourceId,
        page: &PageId,
        block: &BlockId,
        used: bool,
    ) -> Result<AreaId, Error> {
        let area = self.area_of_source(source)?;
        let doc = self.doc(&Document::Area(area.clone()))?;
        let map = sources_map(doc)?;
        let entry = model::child(doc, &map, source.as_str()).ok_or(Error::NotFound)?;
        let key = format!("{USED}{block}");
        if used {
            doc.put(&entry, key.as_str(), page.as_str())?;
        } else if doc.get(&entry, key.as_str())?.is_some() {
            doc.delete(&entry, key.as_str())?;
        }
        Ok(area)
    }
}
