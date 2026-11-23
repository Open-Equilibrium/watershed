use std::fmt;

use automerge::transaction::Transactable;
use automerge::{AutoCommit, ObjId, ObjType, ROOT, ReadDoc};
use serde::{Deserialize, Serialize};

use crate::model::{self, AREA, NAME, SORT, TABLES};
use crate::store::Store;
use crate::workspace::Written;
use crate::{AreaId, BlockId, Changed, Document, Error, RoleId, Workspace};

const ROLES: &str = "roles";
const GRANTS: &str = "grants";
pub(crate) const PROTECTED: &str = "protected";

pub const MAX_ROLES: usize = 100;
pub const MAX_GRANTS: usize = 500;
pub const MAX_ROLE_NAME: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    Read,
    Propose,
    Apply,
    Approve,
    Manage,
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Read => "Read",
            Self::Propose => "Propose",
            Self::Apply => "Apply",
            Self::Approve => "Approve",
            Self::Manage => "Manage",
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Allows {
    pub read: bool,
    pub propose: bool,
    pub apply: bool,
    pub approve: bool,
    pub manage: bool,
}

impl Allows {
    const ALL: Self = Self {
        read: true,
        propose: true,
        apply: true,
        approve: true,
        manage: true,
    };

    fn with(self, other: Self) -> Self {
        Self {
            read: self.read || other.read,
            propose: self.propose || other.propose,
            apply: self.apply || other.apply,
            approve: self.approve || other.approve,
            manage: self.manage || other.manage,
        }
    }

    pub fn allows(&self, capability: Capability) -> bool {
        match capability {
            Capability::Read => self.read,
            Capability::Propose => self.propose,
            Capability::Apply => self.apply,
            Capability::Approve => self.approve,
            Capability::Manage => self.manage,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Workspace,
    Area(AreaId),
    Table(BlockId),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub target: Target,
    pub allows: Allows,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleSetup {
    pub name: String,
    pub grants: Vec<Grant>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Template {
    CanManage,
    CanEdit,
    CanView,
}

impl Template {
    const ALL: [Self; 3] = [Self::CanManage, Self::CanEdit, Self::CanView];

    pub fn id(self) -> RoleId {
        RoleId::stored(match self {
            Self::CanManage => "00000000000000000000000000000001",
            Self::CanEdit => "00000000000000000000000000000002",
            Self::CanView => "00000000000000000000000000000003",
        })
    }

    fn of(role: &RoleId) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|template| template.id() == *role)
    }

    fn setup(self) -> RoleSetup {
        let (name, allows) = match self {
            Self::CanManage => ("Can manage", Allows::ALL),
            Self::CanEdit => (
                "Can edit",
                Allows {
                    read: true,
                    apply: true,
                    ..Allows::default()
                },
            ),
            Self::CanView => (
                "Can view",
                Allows {
                    read: true,
                    ..Allows::default()
                },
            ),
        };
        RoleSetup {
            name: name.into(),
            grants: vec![Grant {
                target: Target::Workspace,
                allows,
            }],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Role {
    pub id: RoleId,
    pub setup: RoleSetup,
    pub template: Option<Template>,
}

fn stored(doc: &AutoCommit, map: &ObjId, role: &str) -> Option<RoleSetup> {
    let entry = model::live_in(doc, map, role)?;
    Some(RoleSetup {
        name: model::string(doc, &entry, NAME)?,
        grants: serde_json::from_str(&model::string(doc, &entry, GRANTS)?).ok()?,
    })
}

pub(crate) fn read(doc: &AutoCommit) -> Vec<Role> {
    let map = model::child(doc, &ROOT, ROLES);
    let mut roles: Vec<Role> = Template::ALL
        .into_iter()
        .map(|template| Role {
            id: template.id(),
            setup: map
                .as_ref()
                .and_then(|map| stored(doc, map, template.id().as_str()))
                .unwrap_or_else(|| template.setup()),
            template: Some(template),
        })
        .collect();
    if let Some(map) = map {
        for listed in model::sorted_in(doc, &map) {
            let id = RoleId::stored(&listed.id);
            if Template::of(&id).is_some() {
                continue;
            }
            if let Some(setup) = stored(doc, &map, &listed.id) {
                roles.push(Role {
                    id,
                    setup,
                    template: None,
                });
            }
        }
    }
    roles
}

fn top(doc: &mut AutoCommit) -> Result<ObjId, Error> {
    match model::child(doc, &ROOT, ROLES) {
        Some(map) => Ok(map),
        None => Ok(doc.put_object(ROOT, ROLES, ObjType::Map)?),
    }
}

fn put(doc: &mut AutoCommit, role: &RoleId, setup: &RoleSetup) -> Result<(), Error> {
    let map = top(doc)?;
    let grants = serde_json::to_string(&setup.grants).map_err(|_| Error::WrongKind)?;
    let entry = match model::live_in(doc, &map, role.as_str()) {
        Some(entry) => entry,
        None => {
            let siblings = model::sorted_in(doc, &map);
            let last = siblings.last().map(|entry| entry.id.as_str());
            let sort = model::sort_after(&siblings, last)?;
            model::insert_in(doc, &map, role.as_str(), &[(SORT, &sort)])?
        }
    };
    doc.put(&entry, NAME, setup.name.as_str())?;
    doc.put(&entry, GRANTS, grants)?;
    Ok(())
}

fn written() -> Written {
    Written::new(Document::Workspace, Changed::Roles)
}

impl<S: Store> Workspace<S> {
    pub fn roles(&mut self) -> Result<Vec<Role>, Error> {
        Ok(read(self.doc(&Document::Workspace)?))
    }

    pub(crate) fn role(&mut self, role: &RoleId) -> Result<Role, Error> {
        self.roles()?
            .into_iter()
            .find(|found| found.id == *role)
            .ok_or(Error::NotFound)
    }

    pub fn allows(&mut self, role: &RoleId, table: &BlockId) -> Result<Allows, Error> {
        let role = self.role(role)?;
        if role.template == Some(Template::CanManage) {
            return Ok(Allows::ALL);
        }
        let doc = self.doc(&Document::Workspace)?;
        let entry = model::live(doc, TABLES, table.as_str()).ok_or(Error::NotFound)?;
        let area = AreaId::stored(&model::string(doc, &entry, AREA).unwrap_or_default());
        let protected = matches!(
            doc.get(&entry, PROTECTED)?,
            Some((automerge::Value::Scalar(value), _))
                if value.as_ref() == &automerge::ScalarValue::Boolean(true)
        );
        let inherited = role
            .setup
            .grants
            .iter()
            .filter(|grant| match &grant.target {
                Target::Workspace => true,
                Target::Area(granted) => *granted == area,
                Target::Table(_) => false,
            })
            .fold(Allows::default(), |all, grant| all.with(grant.allows));
        let own = role
            .setup
            .grants
            .iter()
            .filter(|grant| grant.target == Target::Table(table.clone()))
            .fold(Allows::default(), |all, grant| all.with(grant.allows));
        Ok(if protected {
            Allows {
                read: inherited.read,
                ..Allows::default()
            }
            .with(own)
        } else {
            inherited.with(own)
        })
    }

    fn check_role(&mut self, setup: &RoleSetup) -> Result<(), Error> {
        if setup.name.chars().count() > MAX_ROLE_NAME {
            return Err(Error::TooLong);
        }
        if setup.grants.len() > MAX_GRANTS {
            return Err(Error::TooMany);
        }
        let areas = self.areas()?;
        for grant in &setup.grants {
            match &grant.target {
                Target::Workspace => {}
                Target::Area(area) => {
                    if !areas.iter().any(|known| known.id == *area) {
                        return Err(Error::NotFound);
                    }
                }
                Target::Table(table) => {
                    self.table_doc(table)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn write_create_role(&mut self, setup: &RoleSetup) -> Result<Written, Error> {
        self.check_role(setup)?;
        if self.roles()?.len() >= MAX_ROLES + Template::ALL.len() {
            return Err(Error::TooMany);
        }
        let role = RoleId::new()?;
        put(self.doc(&Document::Workspace)?, &role, setup)?;
        let mut written = written();
        written.created = Some(crate::Created::Role(role));
        Ok(written)
    }

    pub(crate) fn write_set_role(
        &mut self,
        role: &RoleId,
        setup: &RoleSetup,
    ) -> Result<Written, Error> {
        if Template::of(role) == Some(Template::CanManage) {
            return Err(Error::NotAllowed);
        }
        self.role(role)?;
        self.check_role(setup)?;
        put(self.doc(&Document::Workspace)?, role, setup)?;
        Ok(written())
    }

    pub(crate) fn write_remove_role(&mut self, role: &RoleId) -> Result<Written, Error> {
        if Template::of(role).is_some()
            || self
                .automations()?
                .iter()
                .any(|automation| automation.role == *role)
        {
            return Err(Error::NotAllowed);
        }
        self.role(role)?;
        let doc = self.doc(&Document::Workspace)?;
        let map = top(doc)?;
        let entry = model::live_in(doc, &map, role.as_str()).ok_or(Error::NotFound)?;
        model::mark_deleted(doc, &entry)?;
        Ok(written())
    }

    pub(crate) fn write_protect(
        &mut self,
        table: &BlockId,
        kept: &[RoleId],
    ) -> Result<Written, Error> {
        self.table_doc(table)?;
        self.set_protected(table, false)?;
        let mut snapshots = Vec::new();
        for role in self.roles()? {
            if role.template == Some(Template::CanManage) {
                continue;
            }
            let allows = if kept.contains(&role.id) {
                Some(self.allows(&role.id, table)?)
            } else {
                None
            };
            snapshots.push((role, allows));
        }
        for role in kept {
            self.role(role)?;
        }
        let doc = self.doc(&Document::Workspace)?;
        for (role, allows) in snapshots {
            let own = Target::Table(table.clone());
            let had = role.setup.grants.iter().any(|grant| grant.target == own);
            if allows.is_none() && !had {
                continue;
            }
            let mut setup = role.setup;
            setup.grants.retain(|grant| grant.target != own);
            if let Some(allows) = allows {
                setup.grants.push(Grant {
                    target: own,
                    allows,
                });
            }
            put(doc, &role.id, &setup)?;
        }
        self.set_protected(table, true)?;
        let mut written = written();
        written.changed.push(Changed::Tables);
        Ok(written)
    }

    pub(crate) fn write_unprotect(&mut self, table: &BlockId) -> Result<Written, Error> {
        self.table_doc(table)?;
        self.set_protected(table, false)?;
        let mut written = written();
        written.changed.push(Changed::Tables);
        Ok(written)
    }

    fn set_protected(&mut self, table: &BlockId, protected: bool) -> Result<(), Error> {
        let doc = self.doc(&Document::Workspace)?;
        let entry = model::live(doc, TABLES, table.as_str()).ok_or(Error::NotFound)?;
        doc.put(&entry, PROTECTED, protected)?;
        Ok(())
    }

    pub(crate) fn table_left_roles(
        &mut self,
        table: &BlockId,
        written: &mut Written,
    ) -> Result<(), Error> {
        let doc = self.doc(&Document::Workspace)?;
        let own = Target::Table(table.clone());
        let mut touched = false;
        for mut role in read(doc) {
            if !role.setup.grants.iter().any(|grant| grant.target == own) {
                continue;
            }
            role.setup.grants.retain(|grant| grant.target != own);
            put(doc, &role.id, &role.setup)?;
            touched = true;
        }
        if touched {
            written.changed.push(Changed::Roles);
        }
        Ok(())
    }
}
