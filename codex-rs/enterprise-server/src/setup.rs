use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SetupMode {
    SimpleDocker,
    EnterpriseServer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapReceipt {
    pub mode: SetupMode,
    pub initial_owner: String,
    pub registered_workspace_roots: Vec<String>,
    pub created_at: DateTime<Utc>,
}

impl BootstrapReceipt {
    pub fn new(
        mode: SetupMode,
        initial_owner: impl Into<String>,
        registered_workspace_roots: Vec<String>,
    ) -> Self {
        Self {
            mode,
            initial_owner: initial_owner.into(),
            registered_workspace_roots,
            created_at: Utc::now(),
        }
    }
}
