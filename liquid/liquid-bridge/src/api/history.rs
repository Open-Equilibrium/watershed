use liquid_core::{
    ActionId, Actor, AutomationId, BlockId, Entry, EntryQuery, Error, Origin, PageId, Permission,
    Purge, Version,
};

use super::replica::LiquidWorkspace;
use super::workspace::{id, message};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeActorKind {
    Person,
    Automation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeActor {
    pub kind: BridgeActorKind,
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeOrigin {
    App,
    Api,
    Automation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeEntryQuery {
    pub page: Option<String>,
    pub automation: Option<String>,
    pub origin: Option<BridgeOrigin>,
    pub before: Option<String>,
    pub limit: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeEntry {
    pub actions: Vec<String>,
    pub at: i64,
    pub actor: BridgeActor,
    pub origin: BridgeOrigin,
    pub allowed_by: String,
    pub title: String,
    pub page: Option<String>,
    pub undone: bool,
    pub revertable: bool,
    pub purged: bool,
    pub deleted_table: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeVersion {
    pub after: String,
    pub at: i64,
    pub actor: BridgeActor,
    pub save_point: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeReverted {
    pub action: Option<String>,
    pub conflict: bool,
    pub purged: bool,
}

impl BridgeActor {
    fn of(actor: &Actor) -> Self {
        match actor {
            Actor::Person(person) => Self {
                kind: BridgeActorKind::Person,
                id: person.as_str().to_string(),
            },
            Actor::Automation(automation) => Self {
                kind: BridgeActorKind::Automation,
                id: automation.to_string(),
            },
        }
    }
}

impl BridgeOrigin {
    fn of(origin: &Origin) -> Self {
        match origin {
            Origin::Ui => Self::App,
            Origin::Api => Self::Api,
            Origin::Automation => Self::Automation,
        }
    }

    fn to_core(self) -> Origin {
        match self {
            Self::App => Origin::Ui,
            Self::Api => Origin::Api,
            Self::Automation => Origin::Automation,
        }
    }
}

fn ids(texts: &[String]) -> Result<Vec<ActionId>, String> {
    texts
        .iter()
        .map(|text| id(&Some(text.clone()), ActionId::parse, "action"))
        .collect()
}

impl LiquidWorkspace {
    pub fn history(&self, asked: BridgeEntryQuery) -> Result<Vec<BridgeEntry>, String> {
        let query = EntryQuery {
            page: asked
                .page
                .as_ref()
                .map(|_| id(&asked.page, PageId::parse, "page"))
                .transpose()?,
            actor: asked
                .automation
                .as_ref()
                .map(|_| id(&asked.automation, AutomationId::parse, "automation"))
                .transpose()?
                .map(Actor::Automation),
            origin: asked.origin.map(BridgeOrigin::to_core),
            before: asked
                .before
                .as_ref()
                .map(|_| id(&asked.before, ActionId::parse, "before"))
                .transpose()?,
            limit: usize::try_from(asked.limit.clamp(1, 500)).unwrap_or(1),
        };
        let mut inner = self.state()?;
        let entries = inner.workspace.entries(&query).map_err(message)?;
        let roles = inner.workspace.roles().map_err(message)?;
        Ok(entries
            .into_iter()
            .map(|entry: Entry| BridgeEntry {
                actions: entry.actions.iter().map(ToString::to_string).collect(),
                at: entry.at,
                actor: BridgeActor::of(&entry.actor),
                origin: BridgeOrigin::of(&entry.origin),
                allowed_by: match &entry.permission {
                    Permission::Owner => "Can manage".into(),
                    Permission::Role(role) => roles
                        .iter()
                        .find(|kept| kept.id == *role)
                        .map_or_else(|| "a deleted Role".into(), |kept| kept.setup.name.clone()),
                },
                title: entry.title,
                page: entry.page.map(|page| page.to_string()),
                undone: entry.undone,
                revertable: entry.revertable,
                purged: entry.purged,
                deleted_table: entry.deleted_table.map(|table| table.to_string()),
            })
            .collect())
    }

    pub fn revert(&self, actions: Vec<String>) -> Result<BridgeReverted, String> {
        let actions = ids(&actions)?;
        let reverted = self.change(|workspace, actor| workspace.revert(actor, &actions));
        match reverted {
            Ok(action) => Ok(BridgeReverted {
                action: Some(action),
                conflict: false,
                purged: false,
            }),
            Err(Error::Conflict) => Ok(BridgeReverted {
                action: None,
                conflict: true,
                purged: false,
            }),
            Err(Error::Purged) => Ok(BridgeReverted {
                action: None,
                conflict: false,
                purged: true,
            }),
            Err(error) => Err(message(error)),
        }
    }

    pub fn undo(&self) -> Result<bool, String> {
        self.change_maybe(|workspace, actor| workspace.undo(actor))
    }

    pub fn redo(&self) -> Result<bool, String> {
        self.change_maybe(|workspace, actor| workspace.redo(actor))
    }

    pub fn versions(&self, page: String) -> Result<Vec<BridgeVersion>, String> {
        let page = id(&Some(page), PageId::parse, "page")?;
        let mut inner = self.state()?;
        let versions = inner.workspace.versions(&page).map_err(message)?;
        Ok(versions
            .into_iter()
            .map(|version: Version| BridgeVersion {
                after: version.after.to_string(),
                at: version.at,
                actor: BridgeActor::of(&version.actor),
                save_point: version.save_point,
            })
            .collect())
    }

    pub fn restore(&self, page: String, to: String) -> Result<(), String> {
        let page = id(&Some(page), PageId::parse, "page")?;
        let to = id(&Some(to), ActionId::parse, "version")?;
        self.change(|workspace, actor| workspace.restore(actor, &page, &to))
            .map(|_| ())
            .map_err(message)
    }

    pub fn purge(&self, page: Option<String>, table: Option<String>) -> Result<(), String> {
        let target = match (&page, &table) {
            (Some(_), None) => Purge::Page(id(&page, PageId::parse, "page")?),
            (None, Some(_)) => Purge::Table(id(&table, BlockId::parse, "table")?),
            _ => return Err("purge one Page or one Table".into()),
        };
        self.change(|workspace, actor| workspace.purge(actor, &target))
            .map(|_| ())
            .map_err(message)
    }

    pub fn preview(&self, page: String, to: String) -> Result<LiquidWorkspace, String> {
        let page = id(&Some(page), PageId::parse, "page")?;
        let to = id(&Some(to), ActionId::parse, "version")?;
        let documents = self
            .state()?
            .workspace
            .version_documents(&page, &to)
            .map_err(message)?;
        LiquidWorkspace::version(documents)
    }
}
