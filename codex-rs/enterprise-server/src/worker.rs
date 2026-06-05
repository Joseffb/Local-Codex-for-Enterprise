use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerState {
    Starting,
    Running,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerRecord {
    pub worker_id: String,
    pub owner_user_id: String,
    pub workspace_id: String,
    pub session_id: String,
    pub state: WorkerState,
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
