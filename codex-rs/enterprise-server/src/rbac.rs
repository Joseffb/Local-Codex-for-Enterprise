use serde::Deserialize;
use serde::Serialize;

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
