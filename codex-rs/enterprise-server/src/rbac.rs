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
    Developer,
    Viewer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnterpriseAction {
    AdministerUsers,
    ManageWorkspaces,
    StartWorker,
    ReadThreads,
}

impl EnterpriseRole {
    fn as_policy_subject(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Developer => "developer",
            Self::Viewer => "viewer",
        }
    }
}

impl EnterpriseAction {
    fn as_policy_action(self) -> &'static str {
        match self {
            Self::AdministerUsers => "administer_users",
            Self::ManageWorkspaces => "manage_workspaces",
            Self::StartWorker => "start_worker",
            Self::ReadThreads => "read_threads",
        }
    }
}

pub fn role_allows(role: EnterpriseRole, action: EnterpriseAction) -> bool {
    use EnterpriseAction::*;
    use EnterpriseRole::*;

    match role {
        Owner => true,
        Admin => !matches!(action, AdministerUsers),
        Developer => matches!(action, StartWorker | ReadThreads),
        Viewer => matches!(action, ReadThreads),
    }
}

pub async fn casbin_role_allows(role: EnterpriseRole, action: EnterpriseAction) -> Result<bool> {
    let enforcer = enterprise_enforcer().await?;
    enforcer
        .enforce((
            role.as_policy_subject(),
            "enterprise",
            action.as_policy_action(),
        ))
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
        ("owner", "administer_users"),
        ("owner", "manage_workspaces"),
        ("owner", "start_worker"),
        ("owner", "read_threads"),
        ("admin", "manage_workspaces"),
        ("admin", "start_worker"),
        ("admin", "read_threads"),
        ("developer", "start_worker"),
        ("developer", "read_threads"),
        ("viewer", "read_threads"),
    ]
}
