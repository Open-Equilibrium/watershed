//! Actions: what the mutation pipeline records for every write (ADR-0235).

use automerge::ChangeHash;
use serde::{Deserialize, Serialize};

use crate::{ActionId, Actor, AreaId, Mutation, Origin, PageId};

/// Why the pipeline allowed an Action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Permission {
    /// The Workspace's creator holds Can manage, which protection never removes.
    Owner,
}

/// What an Action did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    /// Created the Workspace with its empty structure.
    CreateWorkspace,
    /// Applied a requested mutation.
    Mutation(Mutation),
}

/// A Workspace document an Action changed.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Document {
    /// The Workspace document.
    Workspace,
    /// An area's document.
    Area(AreaId),
    /// A Page's document.
    Page(PageId),
}

/// One recorded Workspace mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    /// The Action's identity, also named in each of its changes.
    pub id: ActionId,
    /// Who made it.
    pub actor: Actor,
    /// The surface it came through.
    pub origin: Origin,
    /// Why it was allowed.
    pub permission: Permission,
    /// What it did.
    pub operation: Operation,
    /// The one Automerge change it made in each document it touched; a change's
    /// dependencies are that document's state before it.
    pub changes: Vec<(Document, ChangeHash)>,
}

/// The part of an Action that every one of its changes carries as its message, so a
/// change names who made it wherever it travels.
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
