use automerge::ChangeHash;
use serde::{Deserialize, Serialize};

use crate::{ActionId, Actor, AreaId, AutomationId, BlockId, Mutation, Origin, PageId};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Permission {
    Owner,
    Role(crate::RoleId),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    CreateWorkspace,
    Mutation(Mutation),
    Run {
        automation: AutomationId,
        cause: Option<ActionId>,
    },
    Revert {
        reverted: Vec<ActionId>,
        kind: RevertKind,
    },
    Restore {
        page: PageId,
        to: ActionId,
    },
    Purge {
        target: Purge,
    },
    Purged {
        kind: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RevertKind {
    Revert,
    Undo,
    Redo,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Purge {
    Page(PageId),
    Table(BlockId),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Document {
    Workspace,
    Area(AreaId),
    Page(PageId),
    Table(BlockId),
}

impl Document {
    pub fn key(&self) -> String {
        match self {
            Self::Workspace => "workspace".to_string(),
            Self::Area(area) => format!("area/{area}"),
            Self::Page(page) => format!("page/{page}"),
            Self::Table(table) => format!("table/{table}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub id: ActionId,
    pub actor: Actor,
    pub origin: Origin,
    pub permission: Permission,
    pub operation: Operation,
    pub changes: Vec<(Document, ChangeHash)>,
    #[serde(default)]
    pub heads: Vec<(Document, Vec<ChangeHash>)>,
    #[serde(default)]
    pub at: i64,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Header {
    pub action: ActionId,
    pub actor: Actor,
    pub origin: Origin,
    pub permission: Permission,
}

impl Header {
    pub(crate) fn of(action: &Action) -> Self {
        Self {
            action: action.id.clone(),
            actor: action.actor.clone(),
            origin: action.origin.clone(),
            permission: action.permission.clone(),
        }
    }
}
