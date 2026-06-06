use crate::auth;
use crate::rbac::EnterpriseRole;
use crate::setup::BootstrapReceipt;
use crate::setup::SetupMode;
use crate::trace::EvidenceRecordContext;
use crate::trace::ExecutionReceipt;
use crate::trace::TraceResult;
use crate::worker::WorkerRecord;
use crate::worker::WorkerRuntime;
use crate::worker::WorkerState;
use crate::workspace::WorkspacePolicy;
use anyhow::Context;
use anyhow::Result;
use async_trait::async_trait;
use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use sqlx::PgPool;
use sqlx::Row;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthPrincipal {
    pub user_id: String,
    pub email: String,
    pub role: EnterpriseRole,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerHandoffRecord {
    pub jti: String,
    pub worker_id: String,
    pub owner_user_id: String,
    pub workspace_id: String,
    pub session_id: String,
    pub socket_path: String,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SessionRecord {
    pub session_id: String,
    pub owner_user_id: String,
    pub workspace_id: String,
    pub workspace_path: String,
    pub title: Option<String>,
    pub last_worker_id: Option<String>,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String)]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEventRecord {
    pub trace_id: String,
    pub actor_user_id: Option<String>,
    pub workspace_id: Option<String>,
    pub session_id: Option<String>,
    pub worker_id: Option<String>,
    pub event_type: String,
    pub result: TraceResult,
    pub metadata_json: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

pub type ExecutionReceiptRecord = ExecutionReceipt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRecordInput {
    pub context: EvidenceRecordContext,
    pub event_type: String,
    pub result: TraceResult,
    pub metadata_json: serde_json::Value,
}

#[async_trait]
pub trait EnterpriseStore: Clone + Send + Sync + 'static {
    async fn is_bootstrapped(&self) -> Result<bool>;
    async fn bootstrap_enterprise(&self, input: BootstrapInput) -> Result<BootstrapOutcome>;
    async fn authenticate_api_token(&self, plaintext_token: &str) -> Result<Option<AuthPrincipal>>;
    async fn authenticate_password(
        &self,
        email: &str,
        password: &str,
    ) -> Result<Option<AuthPrincipal>>;
    async fn create_api_token(
        &self,
        principal: &AuthPrincipal,
        label: &str,
        token_hash: String,
    ) -> Result<()>;
    async fn create_session(
        &self,
        principal: &AuthPrincipal,
        session_id: Option<String>,
        workspace_path: String,
        title: Option<String>,
    ) -> Result<SessionRecord>;
    async fn list_sessions(&self, principal: &AuthPrincipal) -> Result<Vec<SessionRecord>>;
    async fn get_session(
        &self,
        principal: &AuthPrincipal,
        session_id: &str,
    ) -> Result<SessionRecord>;
    async fn start_worker(
        &self,
        principal: &AuthPrincipal,
        workspace_path: String,
        session_id: String,
    ) -> Result<WorkerRecord>;
    async fn update_worker_runtime(
        &self,
        principal: &AuthPrincipal,
        worker_id: &str,
        state: WorkerState,
        runtime: Option<WorkerRuntime>,
    ) -> Result<WorkerRecord>;
    async fn stop_worker(&self, principal: &AuthPrincipal, worker_id: &str)
    -> Result<WorkerRecord>;
    async fn list_workers(&self, principal: &AuthPrincipal) -> Result<Vec<WorkerRecord>>;
    async fn list_workspace_roots(&self, principal: &AuthPrincipal) -> Result<Vec<String>>;
    async fn get_worker(&self, principal: &AuthPrincipal, worker_id: &str) -> Result<WorkerRecord>;
    async fn create_worker_handoff(
        &self,
        principal: &AuthPrincipal,
        worker_id: &str,
        jti: String,
        expires_at: DateTime<Utc>,
    ) -> Result<WorkerHandoffRecord>;
    async fn consume_worker_handoff(
        &self,
        claims: &auth::WorkerHandoffClaims,
    ) -> Result<WorkerHandoffRecord>;
    async fn record_audit_event(&self, input: EvidenceRecordInput) -> Result<()>;
    async fn record_execution_receipt(&self, input: EvidenceRecordInput) -> Result<()>;
}

#[derive(Debug, Default)]
struct MemoryState {
    bootstrap: Option<BootstrapOutcome>,
    users: HashMap<String, AuthPrincipal>,
    workspace_roots: Vec<String>,
    token_hashes: HashMap<String, String>,
    password_hashes: HashMap<String, String>,
    workers: Vec<WorkerRecord>,
    handoffs: HashMap<String, WorkerHandoffRecord>,
    sessions: HashMap<String, SessionRecord>,
    audit_events: Vec<AuditEventRecord>,
    execution_receipts: Vec<ExecutionReceiptRecord>,
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryEnterpriseStore {
    state: Arc<Mutex<MemoryState>>,
}

impl InMemoryEnterpriseStore {
    pub async fn insert_user_for_test(
        &self,
        email: impl Into<String>,
        role: EnterpriseRole,
        token_hash: String,
    ) -> AuthPrincipal {
        let mut state = self.state.lock().await;
        let user_id = Uuid::new_v4().to_string();
        let principal = AuthPrincipal {
            user_id: user_id.clone(),
            email: email.into(),
            role,
        };
        state.token_hashes.insert(token_hash, user_id.clone());
        state.users.insert(user_id, principal.clone());
        principal
    }

    pub async fn audit_events_for_test(&self) -> Vec<AuditEventRecord> {
        self.state.lock().await.audit_events.clone()
    }

    pub async fn execution_receipts_for_test(&self) -> Vec<ExecutionReceiptRecord> {
        self.state.lock().await.execution_receipts.clone()
    }
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
            role: EnterpriseRole::Owner,
        };
        state
            .token_hashes
            .insert(input.issued_token_hash, owner_user_id.clone());
        state
            .password_hashes
            .insert(owner_user_id.clone(), input.owner_password_hash);
        state.users.insert(owner_user_id.clone(), principal);

        let workspace_roots = canonicalize_workspace_roots(input.workspace_roots)?;
        let outcome = BootstrapOutcome {
            owner_user_id,
            owner_email: input.owner_email.clone(),
            receipt: BootstrapReceipt::new(
                SetupMode::EnterpriseServer,
                input.owner_email,
                workspace_roots.clone(),
            ),
        };
        state.workspace_roots = workspace_roots;
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

    async fn authenticate_password(
        &self,
        email: &str,
        password: &str,
    ) -> Result<Option<AuthPrincipal>> {
        let state = self.state.lock().await;
        let Some(principal) = state.users.values().find(|user| user.email == email) else {
            return Ok(None);
        };
        let Some(password_hash) = state.password_hashes.get(&principal.user_id) else {
            return Ok(None);
        };
        if auth::verify_password(password, password_hash)? {
            return Ok(Some(principal.clone()));
        }
        Ok(None)
    }

    async fn create_api_token(
        &self,
        principal: &AuthPrincipal,
        _label: &str,
        token_hash: String,
    ) -> Result<()> {
        let mut state = self.state.lock().await;
        if !state.users.contains_key(&principal.user_id) {
            anyhow::bail!("principal not found");
        }
        state
            .token_hashes
            .insert(token_hash, principal.user_id.clone());
        Ok(())
    }

    async fn create_session(
        &self,
        principal: &AuthPrincipal,
        session_id: Option<String>,
        workspace_path: String,
        title: Option<String>,
    ) -> Result<SessionRecord> {
        let mut state = self.state.lock().await;
        let session = create_session_record(
            &state.workspace_roots,
            principal,
            session_id,
            workspace_path,
            title,
        )?;
        if state.sessions.contains_key(&session.session_id) {
            anyhow::bail!("session already exists");
        }
        state
            .sessions
            .insert(session.session_id.clone(), session.clone());
        Ok(session)
    }

    async fn list_sessions(&self, principal: &AuthPrincipal) -> Result<Vec<SessionRecord>> {
        let state = self.state.lock().await;
        let mut sessions = state
            .sessions
            .values()
            .filter(|session| session.owner_user_id == principal.user_id)
            .cloned()
            .collect::<Vec<_>>();
        sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        Ok(sessions)
    }

    async fn get_session(
        &self,
        principal: &AuthPrincipal,
        session_id: &str,
    ) -> Result<SessionRecord> {
        let state = self.state.lock().await;
        state
            .sessions
            .get(session_id)
            .filter(|session| session.owner_user_id == principal.user_id)
            .cloned()
            .context("session not found")
    }

    async fn start_worker(
        &self,
        principal: &AuthPrincipal,
        workspace_path: String,
        session_id: String,
    ) -> Result<WorkerRecord> {
        let mut state = self.state.lock().await;
        let resolved_workspace_path = authorize_workspace(&state.workspace_roots, workspace_path)?;
        let workspace_roots = state.workspace_roots.clone();
        let now = Utc::now();
        let worker = WorkerRecord {
            worker_id: Uuid::new_v4().to_string(),
            owner_user_id: principal.user_id.clone(),
            workspace_id: resolved_workspace_path.clone(),
            workspace_path: resolved_workspace_path,
            session_id,
            state: WorkerState::Starting,
            pid: None,
            socket_path: None,
            log_path: None,
            last_heartbeat_at: now,
        };
        attach_worker_to_session(&mut state.sessions, &workspace_roots, principal, &worker)?;
        state.workers.push(worker.clone());
        Ok(worker)
    }

    async fn update_worker_runtime(
        &self,
        principal: &AuthPrincipal,
        worker_id: &str,
        state_value: WorkerState,
        runtime: Option<WorkerRuntime>,
    ) -> Result<WorkerRecord> {
        let mut state = self.state.lock().await;
        let worker = state
            .workers
            .iter_mut()
            .find(|worker| {
                worker.worker_id == worker_id && worker.owner_user_id == principal.user_id
            })
            .context("worker not found")?;
        worker.state = state_value;
        worker.last_heartbeat_at = Utc::now();
        if let Some(runtime) = runtime {
            worker.pid = Some(runtime.pid);
            worker.socket_path = Some(runtime.socket_path);
            worker.log_path = Some(runtime.log_path);
        }
        Ok(worker.clone())
    }

    async fn stop_worker(
        &self,
        principal: &AuthPrincipal,
        worker_id: &str,
    ) -> Result<WorkerRecord> {
        self.update_worker_runtime(principal, worker_id, WorkerState::Stopped, None)
            .await
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

    async fn list_workspace_roots(&self, _principal: &AuthPrincipal) -> Result<Vec<String>> {
        let state = self.state.lock().await;
        Ok(state.workspace_roots.clone())
    }

    async fn get_worker(&self, principal: &AuthPrincipal, worker_id: &str) -> Result<WorkerRecord> {
        let state = self.state.lock().await;
        state
            .workers
            .iter()
            .find(|worker| {
                worker.worker_id == worker_id && worker.owner_user_id == principal.user_id
            })
            .cloned()
            .context("worker not found")
    }

    async fn create_worker_handoff(
        &self,
        principal: &AuthPrincipal,
        worker_id: &str,
        jti: String,
        expires_at: DateTime<Utc>,
    ) -> Result<WorkerHandoffRecord> {
        let mut state = self.state.lock().await;
        let worker = state
            .workers
            .iter()
            .find(|worker| {
                worker.worker_id == worker_id && worker.owner_user_id == principal.user_id
            })
            .context("worker not found")?;
        if worker.state != WorkerState::Running {
            anyhow::bail!("worker is not running");
        }
        let socket_path = worker
            .socket_path
            .clone()
            .context("worker socket is not available")?;
        let handoff = WorkerHandoffRecord {
            jti: jti.clone(),
            worker_id: worker.worker_id.clone(),
            owner_user_id: worker.owner_user_id.clone(),
            workspace_id: worker.workspace_id.clone(),
            session_id: worker.session_id.clone(),
            socket_path,
            expires_at,
            consumed_at: None,
        };
        state.handoffs.insert(jti, handoff.clone());
        Ok(handoff)
    }

    async fn consume_worker_handoff(
        &self,
        claims: &auth::WorkerHandoffClaims,
    ) -> Result<WorkerHandoffRecord> {
        let mut state = self.state.lock().await;
        let handoff = state
            .handoffs
            .get_mut(&claims.jti)
            .context("worker handoff not found")?;
        validate_handoff_record(handoff, claims)?;
        handoff.consumed_at = Some(Utc::now());
        Ok(handoff.clone())
    }

    async fn record_audit_event(&self, input: EvidenceRecordInput) -> Result<()> {
        self.state.lock().await.audit_events.push(AuditEventRecord {
            trace_id: input.context.trace_id,
            actor_user_id: input.context.actor_user_id,
            workspace_id: input.context.workspace_id,
            session_id: input.context.session_id,
            worker_id: input.context.worker_id,
            event_type: input.event_type,
            result: input.result,
            metadata_json: input.metadata_json,
            created_at: Utc::now(),
        });
        Ok(())
    }

    async fn record_execution_receipt(&self, input: EvidenceRecordInput) -> Result<()> {
        self.state
            .lock()
            .await
            .execution_receipts
            .push(ExecutionReceiptRecord {
                receipt_id: Uuid::new_v4().to_string(),
                execution_id: Uuid::new_v4().to_string(),
                trace_id: input.context.trace_id,
                actor_user_id: input.context.actor_user_id,
                workspace_id: input.context.workspace_id,
                session_id: input.context.session_id,
                worker_id: input.context.worker_id,
                event_type: input.event_type,
                result: input.result,
                metadata_json: input.metadata_json,
                created_at: Utc::now(),
            });
        Ok(())
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

        let workspace_roots = canonicalize_workspace_roots(input.workspace_roots)?;

        for root in &workspace_roots {
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

        tx.commit().await.context("commit bootstrap tx")?;

        Ok(BootstrapOutcome {
            owner_user_id: owner_user_id.to_string(),
            owner_email: input.owner_email.clone(),
            receipt: BootstrapReceipt::new(
                SetupMode::EnterpriseServer,
                input.owner_email,
                workspace_roots,
            ),
        })
    }

    async fn authenticate_api_token(&self, plaintext_token: &str) -> Result<Option<AuthPrincipal>> {
        let token_hash = auth::api_token_hash(plaintext_token);
        let row = sqlx::query(
            "SELECT u.user_id::text AS user_id, u.email AS email, u.role AS role
             FROM enterprise_api_tokens t
             JOIN enterprise_users u ON u.user_id = t.user_id
             WHERE t.token_hash = $1 AND t.revoked_at IS NULL",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .context("authenticate api token")?;

        row.map(|row| {
            let role: String = row.try_get("role")?;
            Ok(AuthPrincipal {
                user_id: row.try_get("user_id")?,
                email: row.try_get("email")?,
                role: EnterpriseRole::from_storage(&role).context("unknown stored role")?,
            })
        })
        .transpose()
    }

    async fn authenticate_password(
        &self,
        email: &str,
        password: &str,
    ) -> Result<Option<AuthPrincipal>> {
        let row = sqlx::query(
            "SELECT user_id::text, email, password_hash, role
             FROM enterprise_users
             WHERE email = $1",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .context("load user for password auth")?;
        let Some(row) = row else {
            return Ok(None);
        };
        let password_hash: String = row.try_get("password_hash")?;
        if !auth::verify_password(password, &password_hash)? {
            return Ok(None);
        }
        let role: String = row.try_get("role")?;
        Ok(Some(AuthPrincipal {
            user_id: row.try_get("user_id")?,
            email: row.try_get("email")?,
            role: EnterpriseRole::from_storage(&role).context("unknown stored role")?,
        }))
    }

    async fn create_api_token(
        &self,
        principal: &AuthPrincipal,
        label: &str,
        token_hash: String,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO enterprise_api_tokens (token_id, user_id, label, token_hash)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::parse_str(&principal.user_id).context("parse principal user id")?)
        .bind(label)
        .bind(token_hash)
        .execute(&self.pool)
        .await
        .context("insert api token")?;
        Ok(())
    }

    async fn create_session(
        &self,
        principal: &AuthPrincipal,
        session_id: Option<String>,
        workspace_path: String,
        title: Option<String>,
    ) -> Result<SessionRecord> {
        let session_id = session_id.unwrap_or_else(|| Uuid::new_v4().to_string());
        let resolved_workspace_path = self.authorize_workspace_path(workspace_path).await?;
        let row = sqlx::query(
            "INSERT INTO enterprise_sessions
             (session_id, owner_user_id, workspace_id, workspace_path, title)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING session_id, owner_user_id::text, workspace_id, workspace_path, title, last_worker_id::text, created_at, updated_at",
        )
        .bind(&session_id)
        .bind(Uuid::parse_str(&principal.user_id).context("parse principal user id")?)
        .bind(&resolved_workspace_path)
        .bind(&resolved_workspace_path)
        .bind(title)
        .fetch_one(&self.pool)
        .await
        .context("insert session record")?;

        session_from_row(row)
    }

    async fn list_sessions(&self, principal: &AuthPrincipal) -> Result<Vec<SessionRecord>> {
        let rows = sqlx::query(
            "SELECT session_id, owner_user_id::text, workspace_id, workspace_path, title, last_worker_id::text, created_at, updated_at
             FROM enterprise_sessions
             WHERE owner_user_id = $1
             ORDER BY updated_at DESC",
        )
        .bind(Uuid::parse_str(&principal.user_id).context("parse principal user id")?)
        .fetch_all(&self.pool)
        .await
        .context("list session records")?;

        rows.into_iter().map(session_from_row).collect()
    }

    async fn get_session(
        &self,
        principal: &AuthPrincipal,
        session_id: &str,
    ) -> Result<SessionRecord> {
        let row = sqlx::query(
            "SELECT session_id, owner_user_id::text, workspace_id, workspace_path, title, last_worker_id::text, created_at, updated_at
             FROM enterprise_sessions
             WHERE session_id = $1 AND owner_user_id = $2",
        )
        .bind(session_id)
        .bind(Uuid::parse_str(&principal.user_id).context("parse principal user id")?)
        .fetch_optional(&self.pool)
        .await
        .context("get session record")?
        .context("session not found")?;

        session_from_row(row)
    }

    async fn start_worker(
        &self,
        principal: &AuthPrincipal,
        workspace_path: String,
        session_id: String,
    ) -> Result<WorkerRecord> {
        let resolved_workspace_path = self.authorize_workspace_path(workspace_path).await?;
        let worker = WorkerRecord {
            worker_id: Uuid::new_v4().to_string(),
            owner_user_id: principal.user_id.clone(),
            workspace_id: resolved_workspace_path.clone(),
            workspace_path: resolved_workspace_path,
            session_id,
            state: WorkerState::Starting,
            pid: None,
            socket_path: None,
            log_path: None,
            last_heartbeat_at: Utc::now(),
        };

        sqlx::query(
            "INSERT INTO enterprise_workers
             (worker_id, owner_user_id, workspace_id, workspace_path, session_id, state, last_heartbeat_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(Uuid::parse_str(&worker.worker_id).context("parse worker id")?)
        .bind(Uuid::parse_str(&worker.owner_user_id).context("parse owner user id")?)
        .bind(&worker.workspace_id)
        .bind(&worker.workspace_path)
        .bind(&worker.session_id)
        .bind(worker.state.as_str())
        .bind(worker.last_heartbeat_at)
        .execute(&self.pool)
        .await
        .context("insert worker record")?;

        self.attach_worker_to_session(principal, &worker).await?;

        Ok(worker)
    }

    async fn update_worker_runtime(
        &self,
        principal: &AuthPrincipal,
        worker_id: &str,
        state: WorkerState,
        runtime: Option<WorkerRuntime>,
    ) -> Result<WorkerRecord> {
        let pid = runtime.as_ref().map(|runtime| runtime.pid as i64);
        let socket_path = runtime.as_ref().map(|runtime| runtime.socket_path.as_str());
        let log_path = runtime.as_ref().map(|runtime| runtime.log_path.as_str());
        let row = sqlx::query(
            "UPDATE enterprise_workers
             SET state = $1,
                 pid = COALESCE($2, pid),
                 socket_path = COALESCE($3, socket_path),
                 log_path = COALESCE($4, log_path),
                 last_heartbeat_at = now()
             WHERE worker_id = $5 AND owner_user_id = $6
             RETURNING worker_id::text, owner_user_id::text, workspace_id, workspace_path, session_id, state, pid, socket_path, log_path, last_heartbeat_at",
        )
        .bind(state.as_str())
        .bind(pid)
        .bind(socket_path)
        .bind(log_path)
        .bind(Uuid::parse_str(worker_id).context("parse worker id")?)
        .bind(Uuid::parse_str(&principal.user_id).context("parse principal user id")?)
        .fetch_optional(&self.pool)
        .await
        .context("update worker runtime")?
        .context("worker not found")?;

        worker_from_row(row)
    }

    async fn stop_worker(
        &self,
        principal: &AuthPrincipal,
        worker_id: &str,
    ) -> Result<WorkerRecord> {
        self.update_worker_runtime(principal, worker_id, WorkerState::Stopped, None)
            .await
    }

    async fn list_workers(&self, principal: &AuthPrincipal) -> Result<Vec<WorkerRecord>> {
        let rows = sqlx::query(
            "SELECT worker_id::text, owner_user_id::text, workspace_id, workspace_path, session_id, state, pid, socket_path, log_path, last_heartbeat_at
             FROM enterprise_workers
             WHERE owner_user_id = $1
             ORDER BY created_at DESC",
        )
        .bind(Uuid::parse_str(&principal.user_id).context("parse principal user id")?)
        .fetch_all(&self.pool)
        .await
        .context("list worker records")?;

        rows.into_iter().map(worker_from_row).collect()
    }

    async fn list_workspace_roots(&self, _principal: &AuthPrincipal) -> Result<Vec<String>> {
        let rows = sqlx::query("SELECT root_path FROM enterprise_workspaces ORDER BY created_at")
            .fetch_all(&self.pool)
            .await
            .context("list workspace roots")?;
        rows.into_iter()
            .map(|row| row.try_get("root_path"))
            .collect::<Result<Vec<String>, sqlx::Error>>()
            .context("read workspace roots")
    }

    async fn get_worker(&self, principal: &AuthPrincipal, worker_id: &str) -> Result<WorkerRecord> {
        let row = sqlx::query(
            "SELECT worker_id::text, owner_user_id::text, workspace_id, workspace_path, session_id, state, pid, socket_path, log_path, last_heartbeat_at
             FROM enterprise_workers
             WHERE worker_id = $1 AND owner_user_id = $2",
        )
        .bind(Uuid::parse_str(worker_id).context("parse worker id")?)
        .bind(Uuid::parse_str(&principal.user_id).context("parse principal user id")?)
        .fetch_optional(&self.pool)
        .await
        .context("get worker record")?
        .context("worker not found")?;

        worker_from_row(row)
    }

    async fn create_worker_handoff(
        &self,
        principal: &AuthPrincipal,
        worker_id: &str,
        jti: String,
        expires_at: DateTime<Utc>,
    ) -> Result<WorkerHandoffRecord> {
        let worker = self.get_worker(principal, worker_id).await?;
        if worker.state != WorkerState::Running {
            anyhow::bail!("worker is not running");
        }
        let socket_path = worker
            .socket_path
            .clone()
            .context("worker socket is not available")?;

        let row = sqlx::query(
            "INSERT INTO enterprise_worker_handoffs
             (jti, worker_id, owner_user_id, workspace_id, session_id, socket_path, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             RETURNING jti, worker_id::text, owner_user_id::text, workspace_id, session_id, socket_path, expires_at, consumed_at",
        )
        .bind(&jti)
        .bind(Uuid::parse_str(&worker.worker_id).context("parse worker id")?)
        .bind(Uuid::parse_str(&worker.owner_user_id).context("parse owner user id")?)
        .bind(&worker.workspace_id)
        .bind(&worker.session_id)
        .bind(socket_path)
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await
        .context("insert worker handoff")?;

        handoff_from_row(row)
    }

    async fn consume_worker_handoff(
        &self,
        claims: &auth::WorkerHandoffClaims,
    ) -> Result<WorkerHandoffRecord> {
        let mut tx = self.pool.begin().await.context("begin handoff tx")?;
        let row = sqlx::query(
            "SELECT jti, worker_id::text, owner_user_id::text, workspace_id, session_id, socket_path, expires_at, consumed_at
             FROM enterprise_worker_handoffs
             WHERE jti = $1
             FOR UPDATE",
        )
        .bind(&claims.jti)
        .fetch_optional(&mut *tx)
        .await
        .context("load worker handoff")?
        .context("worker handoff not found")?;

        let mut handoff = handoff_from_row(row)?;
        validate_handoff_record(&handoff, claims)?;

        let row = sqlx::query(
            "UPDATE enterprise_worker_handoffs
             SET consumed_at = now()
             WHERE jti = $1
             RETURNING jti, worker_id::text, owner_user_id::text, workspace_id, session_id, socket_path, expires_at, consumed_at",
        )
        .bind(&claims.jti)
        .fetch_one(&mut *tx)
        .await
        .context("consume worker handoff")?;
        tx.commit().await.context("commit handoff tx")?;

        handoff = handoff_from_row(row)?;
        Ok(handoff)
    }

    async fn record_audit_event(&self, input: EvidenceRecordInput) -> Result<()> {
        let trace_id = Uuid::parse_str(&input.context.trace_id).context("parse audit trace id")?;
        let actor_user_id = input
            .context
            .actor_user_id
            .as_deref()
            .map(|value| Uuid::parse_str(value).context("parse audit actor user id"))
            .transpose()?;
        let worker_id = input
            .context
            .worker_id
            .as_deref()
            .map(|value| Uuid::parse_str(value).context("parse audit worker id"))
            .transpose()?;
        sqlx::query(
            "INSERT INTO enterprise_audit_events
             (event_id, trace_id, actor_user_id, workspace_id, session_id, worker_id, event_type, result, metadata_json)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(Uuid::new_v4())
        .bind(trace_id)
        .bind(actor_user_id)
        .bind(&input.context.workspace_id)
        .bind(&input.context.session_id)
        .bind(worker_id)
        .bind(&input.event_type)
        .bind(input.result.as_str())
        .bind(input.metadata_json)
        .execute(&self.pool)
        .await
        .context("insert audit event")?;
        Ok(())
    }

    async fn record_execution_receipt(&self, input: EvidenceRecordInput) -> Result<()> {
        let trace_id =
            Uuid::parse_str(&input.context.trace_id).context("parse receipt trace id")?;
        let actor_user_id = input
            .context
            .actor_user_id
            .as_deref()
            .map(|value| Uuid::parse_str(value).context("parse receipt actor user id"))
            .transpose()?;
        let worker_id = input
            .context
            .worker_id
            .as_deref()
            .map(|value| Uuid::parse_str(value).context("parse receipt worker id"))
            .transpose()?;
        sqlx::query(
            "INSERT INTO enterprise_execution_receipts
             (receipt_id, execution_id, trace_id, actor_user_id, workspace_id, session_id, worker_id, event_type, result, metadata_json)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(trace_id)
        .bind(actor_user_id)
        .bind(&input.context.workspace_id)
        .bind(&input.context.session_id)
        .bind(worker_id)
        .bind(&input.event_type)
        .bind(input.result.as_str())
        .bind(input.metadata_json)
        .execute(&self.pool)
        .await
        .context("insert execution receipt")?;
        Ok(())
    }
}

impl PostgresEnterpriseStore {
    async fn authorize_workspace_path(&self, requested_path: String) -> Result<String> {
        let rows = sqlx::query("SELECT root_path FROM enterprise_workspaces")
            .fetch_all(&self.pool)
            .await
            .context("load workspace allowlist")?;
        let roots = rows
            .into_iter()
            .map(|row| row.try_get("root_path"))
            .collect::<Result<Vec<String>, sqlx::Error>>()
            .context("read workspace allowlist")?;
        authorize_workspace(&roots, requested_path)
    }

    async fn attach_worker_to_session(
        &self,
        principal: &AuthPrincipal,
        worker: &WorkerRecord,
    ) -> Result<()> {
        let result = sqlx::query(
            "INSERT INTO enterprise_sessions
             (session_id, owner_user_id, workspace_id, workspace_path, title, last_worker_id)
             VALUES ($1, $2, $3, $4, NULL, $5)
             ON CONFLICT (session_id) DO UPDATE
             SET last_worker_id = EXCLUDED.last_worker_id,
                 updated_at = now()
             WHERE enterprise_sessions.owner_user_id = EXCLUDED.owner_user_id
               AND enterprise_sessions.workspace_path = EXCLUDED.workspace_path",
        )
        .bind(&worker.session_id)
        .bind(Uuid::parse_str(&principal.user_id).context("parse principal user id")?)
        .bind(&worker.workspace_id)
        .bind(&worker.workspace_path)
        .bind(Uuid::parse_str(&worker.worker_id).context("parse worker id")?)
        .execute(&self.pool)
        .await
        .context("attach worker to session")?;
        if result.rows_affected() == 0 {
            anyhow::bail!("session workspace does not match worker workspace");
        }
        Ok(())
    }
}

fn canonicalize_workspace_roots(roots: Vec<String>) -> Result<Vec<String>> {
    roots
        .into_iter()
        .map(|root| {
            PathBuf::from(&root)
                .canonicalize()
                .with_context(|| format!("canonicalize workspace root {root}"))
                .map(|path| path.to_string_lossy().to_string())
        })
        .collect()
}

fn authorize_workspace(roots: &[String], requested_path: String) -> Result<String> {
    let policy = WorkspacePolicy::new(roots.iter().map(PathBuf::from).collect())?;
    let decision = policy.authorize(PathBuf::from(&requested_path))?;
    if !decision.allowed {
        anyhow::bail!("workspace path is not allowlisted");
    }
    Ok(decision.resolved_path.to_string_lossy().to_string())
}

fn create_session_record(
    roots: &[String],
    principal: &AuthPrincipal,
    session_id: Option<String>,
    workspace_path: String,
    title: Option<String>,
) -> Result<SessionRecord> {
    let resolved_workspace_path = authorize_workspace(roots, workspace_path)?;
    let now = Utc::now();
    Ok(SessionRecord {
        session_id: session_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
        owner_user_id: principal.user_id.clone(),
        workspace_id: resolved_workspace_path.clone(),
        workspace_path: resolved_workspace_path,
        title,
        last_worker_id: None,
        created_at: now,
        updated_at: now,
    })
}

fn attach_worker_to_session(
    sessions: &mut HashMap<String, SessionRecord>,
    roots: &[String],
    principal: &AuthPrincipal,
    worker: &WorkerRecord,
) -> Result<()> {
    if let Some(session) = sessions.get_mut(&worker.session_id) {
        if session.owner_user_id != principal.user_id
            || session.workspace_path != worker.workspace_path
            || session.workspace_id != worker.workspace_id
        {
            anyhow::bail!("session workspace does not match worker workspace");
        }
        session.last_worker_id = Some(worker.worker_id.clone());
        session.updated_at = Utc::now();
        return Ok(());
    }

    let mut session = create_session_record(
        roots,
        principal,
        Some(worker.session_id.clone()),
        worker.workspace_path.clone(),
        None,
    )?;
    session.last_worker_id = Some(worker.worker_id.clone());
    sessions.insert(session.session_id.clone(), session);
    Ok(())
}

fn worker_from_row(row: sqlx::postgres::PgRow) -> Result<WorkerRecord> {
    let state: String = row.try_get("state")?;
    let pid: Option<i64> = row.try_get("pid")?;
    Ok(WorkerRecord {
        worker_id: row.try_get("worker_id")?,
        owner_user_id: row.try_get("owner_user_id")?,
        workspace_id: row.try_get("workspace_id")?,
        workspace_path: row.try_get("workspace_path")?,
        session_id: row.try_get("session_id")?,
        state: WorkerState::from_storage(&state).context("unknown stored worker state")?,
        pid: pid.map(|pid| pid as u32),
        socket_path: row.try_get("socket_path")?,
        log_path: row.try_get("log_path")?,
        last_heartbeat_at: row.try_get("last_heartbeat_at")?,
    })
}

fn session_from_row(row: sqlx::postgres::PgRow) -> Result<SessionRecord> {
    Ok(SessionRecord {
        session_id: row.try_get("session_id")?,
        owner_user_id: row.try_get("owner_user_id")?,
        workspace_id: row.try_get("workspace_id")?,
        workspace_path: row.try_get("workspace_path")?,
        title: row.try_get("title")?,
        last_worker_id: row.try_get("last_worker_id")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn handoff_from_row(row: sqlx::postgres::PgRow) -> Result<WorkerHandoffRecord> {
    Ok(WorkerHandoffRecord {
        jti: row.try_get("jti")?,
        worker_id: row.try_get("worker_id")?,
        owner_user_id: row.try_get("owner_user_id")?,
        workspace_id: row.try_get("workspace_id")?,
        session_id: row.try_get("session_id")?,
        socket_path: row.try_get("socket_path")?,
        expires_at: row.try_get("expires_at")?,
        consumed_at: row.try_get("consumed_at")?,
    })
}

fn validate_handoff_record(
    handoff: &WorkerHandoffRecord,
    claims: &auth::WorkerHandoffClaims,
) -> Result<()> {
    if handoff.consumed_at.is_some() {
        anyhow::bail!("worker handoff already consumed");
    }
    if handoff.expires_at <= Utc::now() {
        anyhow::bail!("worker handoff expired");
    }
    if handoff.owner_user_id != claims.sub
        || handoff.worker_id != claims.worker_id
        || handoff.workspace_id != claims.workspace_id
        || handoff.session_id != claims.session_id
    {
        anyhow::bail!("worker handoff claims do not match record");
    }
    Ok(())
}
