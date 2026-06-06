use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum WorkerState {
    Starting,
    Running,
    Stopped,
    Failed,
}

impl WorkerState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }

    pub fn from_storage(value: &str) -> Option<Self> {
        match value {
            "starting" => Some(Self::Starting),
            "running" => Some(Self::Running),
            "stopped" => Some(Self::Stopped),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct WorkerRecord {
    pub worker_id: String,
    pub owner_user_id: String,
    pub workspace_id: String,
    pub session_id: String,
    pub state: WorkerState,
    #[schema(value_type = String)]
    pub last_heartbeat_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub struct WorkerSupervisor {
    workers: HashMap<String, WorkerRecord>,
}

impl WorkerSupervisor {
    pub fn starting(
        &mut self,
        owner_user_id: impl Into<String>,
        workspace_id: impl Into<String>,
        session_id: impl Into<String>,
    ) -> WorkerRecord {
        let worker = WorkerRecord {
            worker_id: Uuid::new_v4().to_string(),
            owner_user_id: owner_user_id.into(),
            workspace_id: workspace_id.into(),
            session_id: session_id.into(),
            state: WorkerState::Starting,
            last_heartbeat_at: Utc::now(),
        };
        self.workers
            .insert(worker.worker_id.clone(), worker.clone());
        worker
    }

    pub fn get(&self, worker_id: &str) -> Option<&WorkerRecord> {
        self.workers.get(worker_id)
    }
}
