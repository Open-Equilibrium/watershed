use liquid_core::{
    AreaId, Created, Error, FileEntry, Mutation, Origin, PageId, Request, Shown, SourceEntry,
    SourceId, SourceKind,
};

use super::replica::LiquidWorkspace;
use super::workspace::{id, message};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeSourceKind {
    File,
    Folder,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeSource {
    pub id: String,
    pub parent: Option<String>,
    pub name: String,
    pub kind: BridgeSourceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeRead {
    Read,
    Missing,
    TooLarge,
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeFileEntry {
    pub name: String,
    pub folder: bool,
    pub size: u64,
    pub modified: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeFolder {
    pub read: BridgeRead,
    pub entries: Vec<BridgeFileEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeFileView {
    Table,
    Text,
    Image,
    Pdf,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeColumn {
    pub id: String,
    pub name: String,
    pub number: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeSourceFile {
    pub read: BridgeRead,
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified: Option<i64>,
    pub shown: BridgeFileView,
    pub columns: Vec<BridgeColumn>,
    pub rows: Vec<Vec<String>>,
    pub text: Option<String>,
    pub bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeSourceUser {
    pub page: String,
    pub block: String,
}

fn read_of(error: &Error) -> Option<BridgeRead> {
    match error {
        Error::NotFound => Some(BridgeRead::Missing),
        Error::TooLarge => Some(BridgeRead::TooLarge),
        Error::WrongKind => Some(BridgeRead::Unreadable),
        _ => None,
    }
}

impl BridgeSource {
    fn of(entry: SourceEntry) -> Self {
        Self {
            id: entry.id.to_string(),
            parent: entry.parent.map(|parent| parent.to_string()),
            name: entry.name,
            kind: match entry.kind {
                SourceKind::File => BridgeSourceKind::File,
                SourceKind::Folder => BridgeSourceKind::Folder,
            },
        }
    }
}

impl BridgeFileEntry {
    fn of(entry: FileEntry) -> Self {
        Self {
            name: entry.name,
            folder: entry.folder,
            size: entry.size,
            modified: entry.modified,
        }
    }
}

fn source(text: String) -> Result<SourceId, String> {
    id(&Some(text), SourceId::parse, "source")
}

impl LiquidWorkspace {
    pub fn sources(&self, area: String) -> Result<Vec<BridgeSource>, String> {
        let area = id(&Some(area), AreaId::parse, "area")?;
        let mut inner = self.state()?;
        let listed = inner.workspace.sources(&area).map_err(message)?;
        Ok(listed.into_iter().map(BridgeSource::of).collect())
    }

    pub fn source_path(&self, source_id: String) -> Result<Option<String>, String> {
        let source = source(source_id)?;
        self.state()?
            .workspace
            .source_path(&source)
            .map_err(message)
    }

    pub fn add_source(
        &self,
        area: String,
        parent: Option<String>,
        path: String,
    ) -> Result<String, String> {
        let area = id(&Some(area), AreaId::parse, "area")?;
        let parent = parent
            .map(|parent| id(&Some(parent), PageId::parse, "page"))
            .transpose()?;
        let mut added = None;
        self.change(|workspace, actor| {
            let applied = workspace.add_source(actor, &area, parent.as_ref(), &path)?;
            if let Some(Created::Source(source)) = &applied.created {
                added = Some(source.to_string());
            }
            Ok(applied)
        })
        .map_err(message)?;
        added.ok_or_else(|| "no Source was added".into())
    }

    pub fn rename_source(&self, source_id: String, name: String) -> Result<(), String> {
        let source = source(source_id)?;
        self.edit(Mutation::RenameSource { source, name })
    }

    pub fn remove_source(&self, source_id: String) -> Result<(), String> {
        let source = source(source_id)?;
        self.edit(Mutation::RemoveSource { source })
    }

    fn edit(&self, mutation: Mutation) -> Result<(), String> {
        self.change(|workspace, actor| {
            workspace.apply(Request {
                actor: actor.clone(),
                origin: Origin::Ui,
                mutation,
            })
        })
        .map(|_| ())
        .map_err(message)
    }

    pub fn locate_source(&self, source_id: String, path: String) -> Result<(), String> {
        if self.read_only {
            return Err("a version is read-only".into());
        }
        let source = source(source_id)?;
        let mut inner = self.state()?;
        let actor = liquid_core::Actor::Person(inner.workspace.owner().clone());
        inner
            .workspace
            .locate_source(&actor, &source, &path)
            .map_err(message)
    }

    pub fn source_folder(&self, source_id: String, inside: String) -> Result<BridgeFolder, String> {
        let source = source(source_id)?;
        let root = self.state()?.workspace.source_root(&source);
        match root.and_then(|root| liquid_core::list_source(&root, &inside)) {
            Ok(entries) => Ok(BridgeFolder {
                read: BridgeRead::Read,
                entries: entries.into_iter().map(BridgeFileEntry::of).collect(),
            }),
            Err(error) => match read_of(&error) {
                Some(read) => Ok(BridgeFolder {
                    read,
                    entries: Vec::new(),
                }),
                None => Err(message(error)),
            },
        }
    }

    pub fn source_file(
        &self,
        source_id: String,
        inside: String,
    ) -> Result<BridgeSourceFile, String> {
        let source = source(source_id)?;
        let root = self.state()?.workspace.source_root(&source);
        let mut answer = BridgeSourceFile {
            read: BridgeRead::Read,
            name: String::new(),
            path: String::new(),
            size: 0,
            modified: None,
            shown: BridgeFileView::Other,
            columns: Vec::new(),
            rows: Vec::new(),
            text: None,
            bytes: None,
        };
        let file = match root.and_then(|root| liquid_core::read_source(&root, &inside)) {
            Ok(file) => file,
            Err(error) => {
                answer.read = read_of(&error).ok_or_else(|| message(error))?;
                return Ok(answer);
            }
        };
        answer.name = file.name;
        answer.path = file.path;
        answer.size = file.size;
        answer.modified = file.modified;
        answer.shown = match file.shown {
            Shown::Table(table) => {
                answer.columns = table
                    .columns
                    .into_iter()
                    .map(|column| BridgeColumn {
                        id: column.id.to_string(),
                        name: column.name,
                        number: column.number,
                    })
                    .collect();
                answer.rows = table.rows;
                BridgeFileView::Table
            }
            Shown::Text(text) => {
                answer.text = Some(text);
                BridgeFileView::Text
            }
            Shown::Image(bytes) => {
                answer.bytes = Some(bytes);
                BridgeFileView::Image
            }
            Shown::Pdf(bytes) => {
                answer.bytes = Some(bytes);
                BridgeFileView::Pdf
            }
            Shown::Other => BridgeFileView::Other,
        };
        Ok(answer)
    }

    pub fn source_users(&self, source_id: String) -> Result<Vec<BridgeSourceUser>, String> {
        let source = source(source_id)?;
        let mut inner = self.state()?;
        let users = inner.workspace.source_users(&source).map_err(message)?;
        Ok(users
            .into_iter()
            .map(|user| BridgeSourceUser {
                page: user.page.to_string(),
                block: user.block.to_string(),
            })
            .collect())
    }
}
