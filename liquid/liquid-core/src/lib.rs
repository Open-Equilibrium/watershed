//! Liquid's Workspace core: the object model, the one mutation pipeline every write takes,
//! and the History of Actions it records (ADR-0235), over Automerge documents (ADR-0238).
//!
//! The Workspace commits every write to a [`Store`] before acknowledging it;
//! `liquid-storage` implements one on SQLite. The Workspace's data lives in three kinds of
//! Automerge document, each created with its whole top-level structure in its first change:
//!
//! - the Workspace document: its owner and its areas;
//! - one document per area: its Page tree, so the Explorer opens without any Page;
//! - one document per Page: its Blocks and their Views.
//!
//! Removing an object marks it deleted instead of deleting it from the document, so a
//! concurrent edit or move cannot bring it back and a revert can restore it unchanged.
//! Every change of every document carries the header of the Action that made it.
//! These are the merge rules of the client architecture's rule 11
//! (`docs/concept/liquid-client-architecture.md`).

#![deny(missing_docs)]

mod history;
mod id;
mod model;
mod mutation;
mod order;
mod sort_key;
mod store;
mod workspace;

pub use history::{Action, Document, Operation, Permission};
pub use id::{ActionId, AreaId, BlockId, PageId, PersonId, ViewId};
pub use model::{Area, PageEntry, View};
pub use mutation::{Actor, Applied, Changed, Created, Error, Mutation, Origin, Request};
pub use store::{Commit, MemoryStore, Store, Write};
pub use workspace::Workspace;

#[cfg(test)]
mod tests;
