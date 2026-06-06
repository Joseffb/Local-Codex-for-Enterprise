use crate::auth;
use crate::setup::BootstrapReceipt;
use crate::setup::SetupMode;
use crate::worker::WorkerRecord;
use crate::worker::WorkerState;
use anyhow::Context;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use sqlx::PgPool;
use sqlx::Row;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthPrincipal {
    pub user_id: String,
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapInput {
    pub owner_email: String,
    pub owner_password_hash: String,
    pub workspace_roots: Vec<String>,
    pub issued_token_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapOutcome {
    pub owner_user_id: String,
    pub owner_email: String,
    pub receipt: BootstrapReceipt,
}

#[async_trait]
pub trait EnterpriseStore: Clone + Send + Sync + 'static {
    async fn is_bootstrapped(&self) -> Result<bool>;
    async fn bootstrap_enterprise(&self, input: BootstrapInput) -> Result<BootstrapOutcome>;
    async fn authenticate_api_token(&self, plaintext_token: &str) -> Result<Option<AuthPrincipal>>;
    async fn start_worker(
        &self,
        principal: &AuthPrincipal,
        workspace_id: String,
        session_id: String,
    ) -> Result<WorkerRecord>;
    async fn list_workers(&self, principal: &AuthPrincipal) -> Result<Vec<WorkerRecord>>;
}

#[derive(Debug, Default)]
struct MemoryState {
    bootstrap: Option<BootstrapOutcome>,
    users: HashMap<String, AuthPrincipal>,
    token_hashes: HashMap<String, String>,
    workers: Vec<WorkerRecord>,
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryEnterpriseStore {
    state: Arc<Mutex<MemoryState>>,
}

#[async_trait]
impl EnterpriseStore for InMemoryEnterpriseStore {
    async fn is_bootstrapped(&self) -> Result<bool> {
        Ok(self.state.lock().await.bootstrap.is_some())
    }

    async fn bootstrap_enterprise(&self, input: BootstrapInput) -> Result<BootstrapOutcome> {
        let mut state = self.state.lock().await;
        if state.bootstrap.is_some() {
            anyhow::bail!("enterprise server is already bootstrapped");
        }

        let owner_user_id = Uuid::new_v4().to_string();
        let principal = AuthPrincipal {
            user_id: owner_user_id.clone(),
            email: input.owner_email.clone(),
        };
        state
            .token_hashes
            .insert(input.issued_token_hash, owner_user_id.clone());
        state.users.insert(owner_user_id.clone(), principal);

        let outcome = BootstrapOutcome {
            owner_user_id,
            owner_email: input.owner_email.clone(),
            receipt: BootstrapReceipt::new(
                SetupMode::EnterpriseServer,
                input.owner_email,
                input.workspace_roots,
            ),
        };
        state.bootstrap = Some(outcome.clone());
        Ok(outcome)
    }

    async fn authenticate_api_token(&self, plaintext_token: &str) -> Result<Option<AuthPrincipal>> {
        let state = self.state.lock().await;
        let token_hash = auth::api_token_hash(plaintext_token);
        let Some(user_id) = state.token_hashes.get(&token_hash) else {
            return Ok(None);
        };

        Ok(state.users.get(user_id).cloned())
    }

    async fn start_worker(
        &self,
        principal: &AuthPrincipal,
        workspace_id: String,
        session_id: String,
    ) -> Result<WorkerRecord> {
        let mut state = self.state.lock().await;
        let worker = WorkerRecord {
            worker_id: Uuid::new_v4().to_string(),
            owner_user_id: principal.user_id.clone(),
            workspace_id,
            session_id,
            state: WorkerState::Starting,
            last_heartbeat_at: Utc::now(),
        };
        state.workers.push(worker.clone());
        Ok(worker)
    }

    async fn list_workers(&self, principal: &AuthPrincipal) -> Result<Vec<WorkerRecord>> {
        let state = self.state.lock().await;
        Ok(state
            .workers
            .iter()
            .filter(|worker| worker.owner_user_id == principal.user_id)
            .cloned()
            .collect())
    }
}

#[derive(Debug, Clone)]
pub struct PostgresEnterpriseStore {
    pool: PgPool,
}

impl PostgresEnterpriseStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl EnterpriseStore for PostgresEnterpriseStore {
    async fn is_bootstrapped(&self) -> Result<bool> {
        let (exists,): (bool,) =
            sqlx::query_as("SELECT EXISTS(SELECT 1 FROM enterprise_bootstrap)")
                .fetch_one(&self.pool)
                .await
                .context("check enterprise bootstrap state")?;
        Ok(exists)
    }

    async fn bootstrap_enterprise(&self, input: BootstrapInput) -> Result<BootstrapOutcome> {
        if self.is_bootstrapped().await? {
            anyhow::bail!("enterprise server is already bootstrapped");
        }

        let owner_user_id = Uuid::new_v4();
        let bootstrap_id = Uuid::new_v4();
        let mut tx = self.pool.begin().await.context("begin bootstrap tx")?;

        sqlx::query(
            "INSERT INTO enterprise_users (user_id, email, password_hash, role)
             VALUES ($1, $2, $3, 'owner')",
        )
        .bind(owner_user_id)
        .bind(&input.owner_email)
        .bind(&input.owner_password_hash)
        .execute(&mut *tx)
        .await
        .context("insert owner user")?;

        sqlx::query(
            "INSERT INTO enterprise_api_tokens (token_id, user_id, label, token_hash)
             VALUES ($1, $2, 'owner-bootstrap', $3)",
        )
        .bind(Uuid::new_v4())
        .bind(owner_user_id)
        .bind(&input.issued_token_hash)
        .execute(&mut *tx)
        .await
        .context("insert owner api token")?;

        for root in &input.workspace_roots {
            sqlx::query(
                "INSERT INTO enterprise_workspaces (workspace_id, root_path, created_by)
                 VALUES ($1, $2, $3)",
            )
            .bind(Uuid::new_v4())
            .bind(root)
            .bind(owner_user_id)
            .execute(&mut *tx)
            .await
            .context("insert workspace root")?;
        }

        sqlx::query(
            "INSERT INTO enterprise_bootstrap (bootstrap_id, owner_user_id, owner_email)
             VALUES ($1, $2, $3)",
        )
        .bind(bootstrap_id)
        .bind(owner_user_id)
        .bind(&input.owner_email)
        .execute(&mut *tx)
        .await
        .context("insert bootstrap receipt")?;

        sqlx::query(
            "INSERT INTO enterprise_audit_events (event_id, actor_user_id, event_type, event_json)
             VALUES ($1, $2, 'enterprise.bootstrap', $3)",
        )
        .bind(Uuid::new_v4())
        .bind(owner_user_id)
        .bind(serde_json::json!({ "owner_email": input.owner_email }))
        .execute(&mut *tx)
        .await
        .context("insert bootstrap audit event")?;

        tx.commit().await.context("commit bootstrap tx")?;

        Ok(BootstrapOutcome {
            owner_user_id: owner_user_id.to_string(),
            owner_email: input.owner_email.clone(),
            receipt: BootstrapReceipt::new(
                SetupMode::EnterpriseServer,
                input.owner_email,
                input.workspace_roots,
            ),
        })
    }

    async fn authenticate_api_token(&self, plaintext_token: &str) -> Result<Option<AuthPrincipal>> {
        let token_hash = auth::api_token_hash(plaintext_token);
        let row = sqlx::query(
            "SELECT u.user_id::text AS user_id, u.email AS email
             FROM enterprise_api_tokens t
             JOIN enterprise_users u ON u.user_id = t.user_id
             WHERE t.token_hash = $1 AND t.revoked_at IS NULL",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .context("authenticate api token")?;

        row.map(|row| {
            Ok(AuthPrincipal {
                user_id: row.try_get("user_id")?,
                email: row.try_get("email")?,
            })
        })
        .transpose()
    }

    async fn start_worker(
        &self,
        principal: &AuthPrincipal,
        workspace_id: String,
        session_id: String,
    ) -> Result<WorkerRecord> {
        let worker = WorkerRecord {
            worker_id: Uuid::new_v4().to_string(),
            owner_user_id: principal.user_id.clone(),
            workspace_id,
            session_id,
            state: WorkerState::Starting,
            last_heartbeat_at: Utc::now(),
        };

        sqlx::query(
            "INSERT INTO enterprise_workers
             (worker_id, owner_user_id, workspace_id, session_id, state, last_heartbeat_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(Uuid::parse_str(&worker.worker_id).context("parse worker id")?)
        .bind(Uuid::parse_str(&worker.owner_user_id).context("parse owner user id")?)
        .bind(&worker.workspace_id)
        .bind(&worker.session_id)
        .bind(worker.state.as_str())
        .bind(worker.last_heartbeat_at)
        .execute(&self.pool)
        .await
        .context("insert worker record")?;

        Ok(worker)
    }

    async fn list_workers(&self, principal: &AuthPrincipal) -> Result<Vec<WorkerRecord>> {
        let rows = sqlx::query(
            "SELECT worker_id::text, owner_user_id::text, workspace_id, session_id, state, last_heartbeat_at
             FROM enterprise_workers
             WHERE owner_user_id = $1
             ORDER BY created_at DESC",
        )
        .bind(Uuid::parse_str(&principal.user_id).context("parse principal user id")?)
        .fetch_all(&self.pool)
        .await
        .context("list worker records")?;

        rows.into_iter()
            .map(|row| {
                let state: String = row.try_get("state")?;
                Ok(WorkerRecord {
                    worker_id: row.try_get("worker_id")?,
                    owner_user_id: row.try_get("owner_user_id")?,
                    workspace_id: row.try_get("workspace_id")?,
                    session_id: row.try_get("session_id")?,
                    state: WorkerState::from_storage(&state)
                        .context("unknown stored worker state")?,
                    last_heartbeat_at: row.try_get("last_heartbeat_at")?,
                })
            })
            .collect()
    }
}
