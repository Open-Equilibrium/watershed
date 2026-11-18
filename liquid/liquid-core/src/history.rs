use automerge::ChangeHash;
use serde::{Deserialize, Serialize};

use crate::{ActionId, Actor, AreaId, Mutation, Origin, PageId};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Permission {
    Owner,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    CreateWorkspace,
    Mutation(Mutation),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Document {
    Workspace,
    Area(AreaId),
    Page(PageId),
}

impl Document {
    pub fn key(&self) -> String {
        match self {
            Self::Workspace => "workspace".to_string(),
            Self::Area(area) => format!("area/{area}"),
            Self::Page(page) => format!("page/{page}"),
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
