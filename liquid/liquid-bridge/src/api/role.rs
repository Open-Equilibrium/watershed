use liquid_core::{Allows, AreaId, BlockId, Grant, Role, RoleId, RoleSetup, Target, Template};

use super::workspace::id;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeAllows {
    pub read: bool,
    pub propose: bool,
    pub apply: bool,
    pub approve: bool,
    pub manage: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeTargetKind {
    Workspace,
    Area,
    Table,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeGrant {
    pub kind: BridgeTargetKind,
    pub id: Option<String>,
    pub allows: BridgeAllows,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRoleSetup {
    pub name: String,
    pub grants: Vec<BridgeGrant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeTemplate {
    CanManage,
    CanEdit,
    CanView,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRole {
    pub id: String,
    pub setup: BridgeRoleSetup,
    pub template: Option<BridgeTemplate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRoleAllows {
    pub role: String,
    pub allows: BridgeAllows,
}

impl BridgeAllows {
    pub(crate) fn of(allows: Allows) -> Self {
        Self {
            read: allows.read,
            propose: allows.propose,
            apply: allows.apply,
            approve: allows.approve,
            manage: allows.manage,
        }
    }

    fn to_core(self) -> Allows {
        Allows {
            read: self.read,
            propose: self.propose,
            apply: self.apply,
            approve: self.approve,
            manage: self.manage,
        }
    }
}

impl BridgeRoleSetup {
    fn of(setup: RoleSetup) -> Self {
        Self {
            name: setup.name,
            grants: setup
                .grants
                .into_iter()
                .map(|grant| {
                    let (kind, id) = match grant.target {
                        Target::Workspace => (BridgeTargetKind::Workspace, None),
                        Target::Area(area) => (BridgeTargetKind::Area, Some(area.to_string())),
                        Target::Table(table) => (BridgeTargetKind::Table, Some(table.to_string())),
                    };
                    BridgeGrant {
                        kind,
                        id,
                        allows: BridgeAllows::of(grant.allows),
                    }
                })
                .collect(),
        }
    }

    pub(crate) fn to_core(&self) -> Result<RoleSetup, String> {
        Ok(RoleSetup {
            name: self.name.clone(),
            grants: self
                .grants
                .iter()
                .map(|grant| {
                    Ok(Grant {
                        target: match grant.kind {
                            BridgeTargetKind::Workspace => Target::Workspace,
                            BridgeTargetKind::Area => {
                                Target::Area(id(&grant.id, AreaId::parse, "area")?)
                            }
                            BridgeTargetKind::Table => {
                                Target::Table(id(&grant.id, BlockId::parse, "table")?)
                            }
                        },
                        allows: grant.allows.to_core(),
                    })
                })
                .collect::<Result<_, String>>()?,
        })
    }
}

impl BridgeRole {
    pub(crate) fn of(role: Role) -> Self {
        Self {
            id: role.id.to_string(),
            setup: BridgeRoleSetup::of(role.setup),
            template: role.template.map(|template| match template {
                Template::CanManage => BridgeTemplate::CanManage,
                Template::CanEdit => BridgeTemplate::CanEdit,
                Template::CanView => BridgeTemplate::CanView,
            }),
        }
    }
}

pub(crate) fn role_id(text: &Option<String>) -> Result<RoleId, String> {
    id(text, RoleId::parse, "role")
}
