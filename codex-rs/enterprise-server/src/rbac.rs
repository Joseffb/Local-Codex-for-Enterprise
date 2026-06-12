use serde::Deserialize;
use serde::Serialize;

use anyhow::Context;
use anyhow::Result;
use casbin::CoreApi;
use casbin::DefaultModel;
use casbin::Enforcer;
use casbin::MemoryAdapter;
use casbin::MgmtApi;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnterpriseRole {
    Owner,
    Admin,
    Manager,
    Developer,
    Viewer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnterpriseAction {
    AdministerUsers,
    AssignRoles,
    ManageWorkspaces,
    GrantWorkspaceAccess,
    ManageContextPacks,
    ManageOutputs,
    ManageOwnContextPacks,
    ManageSchedules,
    StartWorker,
    ReadThreads,
    ReadAudit,
}

impl EnterpriseRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "admin",
            Self::Admin => "admin",
            Self::Manager => "manager",
            Self::Developer => "developer",
            Self::Viewer => "viewer",
        }
    }

    pub fn from_storage(value: &str) -> Option<Self> {
        match value {
            "owner" => Some(Self::Admin),
            "admin" => Some(Self::Admin),
            "manager" => Some(Self::Manager),
            "developer" => Some(Self::Developer),
            "viewer" => Some(Self::Viewer),
            _ => None,
        }
    }
}

impl EnterpriseAction {
    fn as_policy_action(self) -> &'static str {
        match self {
            Self::AdministerUsers => "administer_users",
            Self::AssignRoles => "assign_roles",
            Self::ManageWorkspaces => "manage_workspaces",
            Self::GrantWorkspaceAccess => "grant_workspace_access",
            Self::ManageContextPacks => "manage_context_packs",
            Self::ManageOutputs => "manage_outputs",
            Self::ManageOwnContextPacks => "manage_own_context_packs",
            Self::ManageSchedules => "manage_schedules",
            Self::StartWorker => "start_worker",
            Self::ReadThreads => "read_threads",
            Self::ReadAudit => "read_audit",
        }
    }
}

pub fn role_allows(role: EnterpriseRole, action: EnterpriseAction) -> bool {
    use EnterpriseAction::*;
    use EnterpriseRole::*;

    match role {
        Owner => true,
        Admin => matches!(
            action,
            AdministerUsers
                | AssignRoles
                | ManageWorkspaces
                | GrantWorkspaceAccess
                | ManageContextPacks
                | ManageOutputs
                | ManageOwnContextPacks
                | ManageSchedules
                | StartWorker
                | ReadThreads
                | ReadAudit
        ),
        Manager => matches!(
            action,
            GrantWorkspaceAccess
                | ManageOutputs
                | ManageSchedules
                | StartWorker
                | ReadThreads
                | ReadAudit
        ),
        Developer => matches!(
            action,
            GrantWorkspaceAccess | ManageOwnContextPacks | StartWorker | ReadThreads
        ),
        Viewer => matches!(action, StartWorker | ReadThreads),
    }
}

pub async fn casbin_role_allows(role: EnterpriseRole, action: EnterpriseAction) -> Result<bool> {
    let enforcer = enterprise_enforcer().await?;
    enforcer
        .enforce((role.as_str(), "enterprise", action.as_policy_action()))
        .context("evaluate enterprise rbac policy")
}

async fn enterprise_enforcer() -> Result<Enforcer> {
    let model = DefaultModel::from_str(
        r#"
[request_definition]
r = sub, obj, act

[policy_definition]
p = sub, obj, act

[policy_effect]
e = some(where (p.eft == allow))

[matchers]
m = r.sub == p.sub && r.obj == p.obj && r.act == p.act
"#,
    )
    .await
    .context("load enterprise rbac model")?;

    let adapter = MemoryAdapter::default();
    let mut enforcer = Enforcer::new(model, adapter)
        .await
        .context("create enterprise rbac enforcer")?;

    for (role, action) in policy_matrix() {
        enforcer
            .add_policy(vec![
                role.to_string(),
                "enterprise".to_string(),
                action.to_string(),
            ])
            .await
            .context("add enterprise rbac policy")?;
    }

    Ok(enforcer)
}

fn policy_matrix() -> Vec<(&'static str, &'static str)> {
    vec![
        ("admin", "administer_users"),
        ("admin", "assign_roles"),
        ("admin", "manage_workspaces"),
        ("admin", "grant_workspace_access"),
        ("admin", "manage_context_packs"),
        ("admin", "manage_outputs"),
        ("admin", "manage_own_context_packs"),
        ("admin", "manage_schedules"),
        ("admin", "start_worker"),
        ("admin", "read_threads"),
        ("admin", "read_audit"),
        ("manager", "grant_workspace_access"),
        ("manager", "manage_outputs"),
        ("manager", "manage_schedules"),
        ("manager", "start_worker"),
        ("manager", "read_threads"),
        ("manager", "read_audit"),
        ("developer", "grant_workspace_access"),
        ("developer", "manage_own_context_packs"),
        ("developer", "start_worker"),
        ("developer", "read_threads"),
        ("viewer", "read_threads"),
        ("viewer", "start_worker"),
    ]
}
