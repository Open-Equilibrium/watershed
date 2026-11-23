#![allow(dead_code)]

use liquid_core::{
    Actor, Applied, Created, Error, MemoryStore, Mutation, Origin, PersonId, Request, RowQuery,
    Store,
};

pub type Workspace = liquid_core::Workspace<MemoryStore>;

pub fn owner() -> Actor {
    Actor::Person(PersonId::new("benni"))
}

pub fn workspace() -> Workspace {
    Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap()
}

pub fn apply<S: Store>(
    workspace: &mut liquid_core::Workspace<S>,
    mutation: Mutation,
) -> Result<Applied, Error> {
    workspace.apply(Request {
        actor: owner(),
        origin: Origin::Ui,
        mutation,
    })
}

pub fn created<S: Store>(workspace: &mut liquid_core::Workspace<S>, mutation: Mutation) -> Created {
    apply(workspace, mutation).unwrap().created.unwrap()
}

pub fn all() -> RowQuery {
    RowQuery {
        sort: None,
        filter: None,
        within: None,
        start: 0,
        limit: usize::MAX,
        search: None,
        totals: Vec::new(),
    }
}
