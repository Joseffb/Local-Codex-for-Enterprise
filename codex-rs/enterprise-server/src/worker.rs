use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Child;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::Duration;
use tokio::time::Instant;
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
    pub workspace_path: String,
    pub session_id: String,
    pub state: WorkerState,
    pub pid: Option<u32>,
    pub socket_path: Option<String>,
    pub log_path: Option<String>,
    #[schema(value_type = String)]
    pub last_heartbeat_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerRuntime {
    pub pid: u32,
    pub socket_path: String,
    pub log_path: String,
}

#[derive(Debug, Clone, Default)]
pub struct WorkerRuntimeSupervisor {
    children: Arc<Mutex<HashMap<String, Child>>>,
}

impl WorkerRuntimeSupervisor {
    pub async fn launch(
        &self,
        worker: &WorkerRecord,
        config: &crate::config::EnterpriseConfig,
    ) -> anyhow::Result<WorkerRuntime> {
        std::fs::create_dir_all(&config.worker_socket_dir)?;
        std::fs::create_dir_all(&config.worker_log_dir)?;

        let socket_path = Path::new(&config.worker_socket_dir)
            .join(format!("{}.sock", worker.worker_id))
            .to_string_lossy()
            .to_string();
        let log_path = Path::new(&config.worker_log_dir)
            .join(format!("{}.log", worker.worker_id))
            .to_string_lossy()
            .to_string();

        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;
        let stderr = log.try_clone()?;
        let should_wait_for_socket = config
            .worker_args
            .iter()
            .any(|arg| arg.contains("{socket_path}"));
        let args = config
            .worker_args
            .iter()
            .map(|arg| {
                arg.replace("{worker_id}", &worker.worker_id)
                    .replace("{socket_path}", &socket_path)
                    .replace("{workspace_path}", &worker.workspace_path)
            })
            .collect::<Vec<_>>();

        let mut command = Command::new(&config.worker_command);
        command
            .args(args)
            .current_dir(PathBuf::from(&worker.workspace_path))
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(stderr));
        let child = command.spawn()?;
        let pid = child.id().unwrap_or_default();
        self.children
            .lock()
            .await
            .insert(worker.worker_id.clone(), child);
        if should_wait_for_socket {
            wait_for_socket_path(&socket_path).await?;
        }

        Ok(WorkerRuntime {
            pid,
            socket_path,
            log_path,
        })
    }

    pub async fn stop(&self, worker_id: &str) -> anyhow::Result<bool> {
        let child = self.children.lock().await.remove(worker_id);
        let Some(mut child) = child else {
            return Ok(false);
        };

        child.kill().await?;
        Ok(true)
    }
}

async fn wait_for_socket_path(socket_path: &str) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if Path::new(socket_path).exists() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            anyhow::bail!("worker socket was not created at {socket_path}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
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
            workspace_path: String::new(),
            session_id: session_id.into(),
            state: WorkerState::Starting,
            pid: None,
            socket_path: None,
            log_path: None,
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
