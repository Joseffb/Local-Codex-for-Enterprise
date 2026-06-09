use crate::auth;
use crate::context_packs;
use crate::context_packs::ContextPackDocumentInput;
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
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UserStatus {
    Active,
    Inactive,
}

impl UserStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Inactive => "inactive",
        }
    }

    pub fn from_storage(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "inactive" => Some(Self::Inactive),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthPrincipal {
    pub user_id: String,
    pub email: String,
    pub role: EnterpriseRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UserRecord {
    pub user_id: String,
    pub email: String,
    pub role: String,
    pub status: UserStatus,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String)]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateUserInput {
    pub email: String,
    pub password_hash: String,
    pub role: EnterpriseRole,
    pub workspace_roots: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct WorkspaceAssignmentRecord {
    pub assignment_id: String,
    pub user_id: String,
    pub workspace_id: Option<String>,
    pub workspace_root: String,
    pub assigned_by: String,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct WorkspaceRootRecord {
    pub workspace_id: String,
    pub root_path: String,
    pub created_by: String,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UserWorkspaceRecord {
    pub user_workspace_id: String,
    pub owner_user_id: String,
    pub workspace_root_id: Option<String>,
    pub path: String,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RepositoryRecord {
    pub repository_id: String,
    pub project_id: String,
    pub repo_url: Option<String>,
    pub name: String,
    pub repository_path: String,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProjectRecord {
    pub project_id: String,
    pub owner_user_id: String,
    pub user_workspace_id: String,
    pub name: String,
    pub slug: String,
    pub project_path: String,
    pub repositories: Vec<RepositoryRecord>,
    pub threads: Vec<SessionRecord>,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String)]
    pub updated_at: DateTime<Utc>,
    #[schema(value_type = Option<String>)]
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateProjectInput {
    pub name: String,
    pub user_workspace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateProjectInput {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateProjectThreadInput {
    pub project_id: String,
    pub repository_id: Option<String>,
    pub session_id: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneProjectRepositoryInput {
    pub project_id: String,
    pub repo_url: String,
    pub destination_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ContextPackRecord {
    pub pack_id: String,
    pub name: String,
    pub status: String,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ContextDocumentRecord {
    pub document_id: String,
    pub pack_id: String,
    pub filename: String,
    pub content_hash: String,
    pub load_order: i32,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ContextPackAssignmentRecord {
    pub assignment_id: String,
    pub pack_id: String,
    pub user_id: Option<String>,
    pub workspace_id: Option<String>,
    pub assignment_source: String,
    pub assignment_order: i32,
    pub required_session: bool,
    pub required_worker: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ContextPackReceiptRecord {
    pub receipt_id: String,
    pub trace_id: String,
    pub pack_id: String,
    pub document_id: String,
    pub content_hash: String,
    pub load_order: i32,
    pub assignment_source: String,
    pub actor_user_id: String,
    pub workspace_id: String,
    pub session_id: String,
    pub worker_id: Option<String>,
    pub phase: String,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutputCategory {
    Operational,
    Deliverable,
}

impl OutputCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Operational => "operational",
            Self::Deliverable => "deliverable",
        }
    }

    pub fn from_storage(value: &str) -> Option<Self> {
        match value {
            "operational" => Some(Self::Operational),
            "deliverable" => Some(Self::Deliverable),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct OutputRecord {
    pub output_id: String,
    pub owner_user_id: String,
    pub workspace_id: Option<String>,
    pub session_id: Option<String>,
    pub worker_id: Option<String>,
    pub category: OutputCategory,
    pub output_type: String,
    pub title: String,
    pub artifact_path: String,
    pub status: String,
    pub metadata_json: serde_json::Value,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String)]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateOutputInput {
    pub owner_user_id: String,
    pub workspace_id: Option<String>,
    pub session_id: Option<String>,
    pub worker_id: Option<String>,
    pub category: OutputCategory,
    pub output_type: String,
    pub title: String,
    pub artifact_path: String,
    pub status: String,
    pub metadata_json: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateContextPackInput {
    pub name: String,
    pub documents: Vec<ContextPackDocumentInput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignContextPackInput {
    pub pack_id: String,
    pub user_id: Option<String>,
    pub workspace_id: Option<String>,
    pub assignment_order: i32,
    pub required_session: bool,
    pub required_worker: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextLoadInput {
    pub trace_id: String,
    pub actor_user_id: String,
    pub workspace_id: String,
    pub session_id: String,
    pub worker_id: Option<String>,
    pub phase: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceQuery {
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct EvidenceQueryResult {
    pub audit_events: Vec<AuditEventRecord>,
    pub execution_receipts: Vec<ExecutionReceiptRecord>,
    pub context_receipts: Vec<ContextPackReceiptRecord>,
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
    pub project_id: Option<String>,
    pub repository_id: Option<String>,
    pub title: Option<String>,
    pub last_worker_id: Option<String>,
    #[schema(value_type = String)]
    pub deleted_at: Option<DateTime<Utc>>,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String)]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SessionMessageRecord {
    pub message_id: String,
    pub session_id: String,
    pub owner_user_id: String,
    pub kind: String,
    pub label: String,
    pub text: String,
    pub sequence: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feedback_rating: Option<String>,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ResponseFeedbackRecord {
    pub feedback_id: String,
    pub owner_user_id: String,
    pub session_id: String,
    pub message_id: String,
    pub rating: String,
    pub reason_tags: Vec<String>,
    pub comment: Option<String>,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String)]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UserResponsePreferencesRecord {
    pub owner_user_id: String,
    pub profile_summary: String,
    pub positive_tags: Vec<String>,
    pub negative_tags: Vec<String>,
    pub sample_count: i64,
    #[schema(value_type = String)]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSessionMessageInput {
    pub kind: String,
    pub label: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpsertResponseFeedbackInput {
    pub rating: String,
    pub reason_tags: Vec<String>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateSessionTitleInput {
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AuditEventRecord {
    pub trace_id: String,
    pub actor_user_id: Option<String>,
    pub workspace_id: Option<String>,
    pub session_id: Option<String>,
    pub worker_id: Option<String>,
    pub event_type: String,
    pub result: TraceResult,
    pub metadata_json: serde_json::Value,
    #[schema(value_type = String)]
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
    async fn create_user(
        &self,
        principal: &AuthPrincipal,
        input: CreateUserInput,
    ) -> Result<UserRecord>;
    async fn list_users(&self, principal: &AuthPrincipal) -> Result<Vec<UserRecord>>;
    async fn assign_user_role(
        &self,
        principal: &AuthPrincipal,
        user_id: &str,
        role: EnterpriseRole,
    ) -> Result<UserRecord>;
    async fn set_user_status(
        &self,
        principal: &AuthPrincipal,
        user_id: &str,
        status: UserStatus,
    ) -> Result<UserRecord>;
    async fn register_workspace_root(
        &self,
        principal: &AuthPrincipal,
        root_path: String,
    ) -> Result<WorkspaceRootRecord>;
    async fn validate_workspace_path(
        &self,
        principal: &AuthPrincipal,
        workspace_path: String,
    ) -> Result<String>;
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
    async fn update_session_title(
        &self,
        principal: &AuthPrincipal,
        session_id: &str,
        input: UpdateSessionTitleInput,
    ) -> Result<SessionRecord>;
    async fn delete_session(
        &self,
        principal: &AuthPrincipal,
        session_id: &str,
    ) -> Result<SessionRecord>;
    async fn create_session_message(
        &self,
        principal: &AuthPrincipal,
        session_id: &str,
        input: CreateSessionMessageInput,
    ) -> Result<SessionMessageRecord>;
    async fn list_session_messages(
        &self,
        principal: &AuthPrincipal,
        session_id: &str,
    ) -> Result<Vec<SessionMessageRecord>>;
    async fn upsert_response_feedback(
        &self,
        principal: &AuthPrincipal,
        session_id: &str,
        message_id: &str,
        input: UpsertResponseFeedbackInput,
    ) -> Result<(ResponseFeedbackRecord, UserResponsePreferencesRecord)>;
    async fn get_response_preferences(
        &self,
        principal: &AuthPrincipal,
    ) -> Result<UserResponsePreferencesRecord>;
    async fn reset_response_preferences(&self, principal: &AuthPrincipal) -> Result<()>;
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
    async fn list_user_workspaces(
        &self,
        principal: &AuthPrincipal,
    ) -> Result<Vec<UserWorkspaceRecord>>;
    async fn assign_workspace_to_user(
        &self,
        principal: &AuthPrincipal,
        user_id: &str,
        workspace_root: String,
    ) -> Result<WorkspaceAssignmentRecord>;
    async fn list_projects(
        &self,
        principal: &AuthPrincipal,
        include_deleted: bool,
    ) -> Result<Vec<ProjectRecord>>;
    async fn create_project(
        &self,
        principal: &AuthPrincipal,
        input: CreateProjectInput,
    ) -> Result<ProjectRecord>;
    async fn update_project(
        &self,
        principal: &AuthPrincipal,
        project_id: &str,
        input: UpdateProjectInput,
    ) -> Result<ProjectRecord>;
    async fn delete_project(
        &self,
        principal: &AuthPrincipal,
        project_id: &str,
    ) -> Result<ProjectRecord>;
    async fn restore_project(
        &self,
        principal: &AuthPrincipal,
        project_id: &str,
    ) -> Result<ProjectRecord>;
    async fn create_project_thread(
        &self,
        principal: &AuthPrincipal,
        input: CreateProjectThreadInput,
    ) -> Result<SessionRecord>;
    async fn clone_project_repository(
        &self,
        principal: &AuthPrincipal,
        input: CloneProjectRepositoryInput,
    ) -> Result<RepositoryRecord>;
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
    async fn create_context_pack(
        &self,
        principal: &AuthPrincipal,
        input: CreateContextPackInput,
    ) -> Result<(ContextPackRecord, Vec<ContextDocumentRecord>)>;
    async fn list_context_packs(&self, principal: &AuthPrincipal)
    -> Result<Vec<ContextPackRecord>>;
    async fn assign_context_pack(
        &self,
        principal: &AuthPrincipal,
        input: AssignContextPackInput,
    ) -> Result<ContextPackAssignmentRecord>;
    async fn list_context_pack_assignments(
        &self,
        principal: &AuthPrincipal,
    ) -> Result<Vec<ContextPackAssignmentRecord>>;
    async fn delete_context_pack_assignment(
        &self,
        principal: &AuthPrincipal,
        assignment_id: &str,
    ) -> Result<ContextPackAssignmentRecord>;
    async fn record_context_load(
        &self,
        input: ContextLoadInput,
    ) -> Result<Vec<ContextPackReceiptRecord>>;
    async fn query_evidence(
        &self,
        principal: &AuthPrincipal,
        query: EvidenceQuery,
    ) -> Result<EvidenceQueryResult>;
    async fn create_output(&self, input: CreateOutputInput) -> Result<OutputRecord>;
    async fn list_outputs(&self, principal: &AuthPrincipal) -> Result<Vec<OutputRecord>>;
    async fn get_output(&self, principal: &AuthPrincipal, output_id: &str) -> Result<OutputRecord>;
    async fn record_audit_event(&self, input: EvidenceRecordInput) -> Result<()>;
    async fn record_execution_receipt(&self, input: EvidenceRecordInput) -> Result<()>;
}

#[derive(Debug, Default)]
struct MemoryState {
    bootstrap: Option<BootstrapOutcome>,
    users: HashMap<String, AuthPrincipal>,
    user_statuses: HashMap<String, UserStatus>,
    user_created_at: HashMap<String, DateTime<Utc>>,
    user_updated_at: HashMap<String, DateTime<Utc>>,
    workspace_roots: Vec<String>,
    workspaces: Vec<WorkspaceRootRecord>,
    token_hashes: HashMap<String, String>,
    password_hashes: HashMap<String, String>,
    workers: Vec<WorkerRecord>,
    handoffs: HashMap<String, WorkerHandoffRecord>,
    sessions: HashMap<String, SessionRecord>,
    session_messages: HashMap<String, Vec<SessionMessageRecord>>,
    response_feedback: HashMap<String, ResponseFeedbackRecord>,
    response_preferences: HashMap<String, UserResponsePreferencesRecord>,
    audit_events: Vec<AuditEventRecord>,
    execution_receipts: Vec<ExecutionReceiptRecord>,
    context_packs: HashMap<String, ContextPackRecord>,
    context_documents: Vec<ContextDocumentRecord>,
    context_assignments: Vec<ContextPackAssignmentRecord>,
    context_receipts: Vec<ContextPackReceiptRecord>,
    outputs: Vec<OutputRecord>,
    workspace_assignments: Vec<WorkspaceAssignmentRecord>,
    user_workspaces: Vec<UserWorkspaceRecord>,
    projects: Vec<ProjectRecord>,
    repositories: Vec<RepositoryRecord>,
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
        let now = Utc::now();
        state.token_hashes.insert(token_hash, user_id.clone());
        state
            .user_statuses
            .insert(user_id.clone(), UserStatus::Active);
        state.user_created_at.insert(user_id.clone(), now);
        state.user_updated_at.insert(user_id.clone(), now);
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

fn empty_response_preferences(owner_user_id: &str) -> UserResponsePreferencesRecord {
    UserResponsePreferencesRecord {
        owner_user_id: owner_user_id.to_string(),
        profile_summary: String::new(),
        positive_tags: Vec::new(),
        negative_tags: Vec::new(),
        sample_count: 0,
        updated_at: Utc::now(),
    }
}

fn build_response_preferences_from_feedback<'a>(
    owner_user_id: &str,
    feedback: impl Iterator<Item = &'a ResponseFeedbackRecord>,
) -> UserResponsePreferencesRecord {
    let mut positive_tags = BTreeSet::new();
    let mut negative_tags = BTreeSet::new();
    let mut sample_count = 0_i64;
    for record in feedback {
        if record.owner_user_id != owner_user_id {
            continue;
        }
        sample_count += 1;
        let target = if record.rating == "good" {
            &mut positive_tags
        } else {
            &mut negative_tags
        };
        for tag in &record.reason_tags {
            target.insert(tag.clone());
        }
    }
    let negative_tags_vec = negative_tags.into_iter().collect::<Vec<_>>();
    let positive_tags_vec = positive_tags.into_iter().collect::<Vec<_>>();
    let profile_summary =
        response_preference_summary(&positive_tags_vec, &negative_tags_vec).join("\n");
    UserResponsePreferencesRecord {
        owner_user_id: owner_user_id.to_string(),
        profile_summary,
        positive_tags: positive_tags_vec,
        negative_tags: negative_tags_vec,
        sample_count,
        updated_at: Utc::now(),
    }
}

fn response_preference_summary(positive_tags: &[String], negative_tags: &[String]) -> Vec<String> {
    let mut lines = BTreeSet::new();
    if negative_tags.iter().any(|tag| tag == "too_verbose") {
        lines.insert("Prefer concise answers.".to_string());
    }
    if negative_tags.iter().any(|tag| tag == "too_generic") {
        lines.insert("Make answers specific to the user's stated goal.".to_string());
    }
    if negative_tags.iter().any(|tag| tag == "wrong_context") {
        lines.insert("Check whether the request is conceptual, product, or codebase-specific before choosing tools.".to_string());
    }
    if negative_tags
        .iter()
        .any(|tag| tag == "used_repo_when_not_needed")
    {
        lines.insert(
            "Do not inspect repositories unless explicitly requested or clearly required."
                .to_string(),
        );
    }
    if negative_tags.iter().any(|tag| tag == "poor_formatting") {
        lines.insert("Format longer answers as readable Markdown.".to_string());
    }
    if negative_tags
        .iter()
        .any(|tag| tag == "missed_business_goal")
    {
        lines.insert(
            "Start business-planning answers with goals, users, decisions, and data sources."
                .to_string(),
        );
    }
    if negative_tags.iter().any(|tag| tag == "raw_tool_output") {
        lines.insert(
            "Collapse or summarize tool output; never show raw HTML unless requested.".to_string(),
        );
    }
    if positive_tags.iter().any(|tag| tag == "concise") {
        lines.insert("Keep answers concise when the request is simple.".to_string());
    }
    lines.into_iter().collect()
}

fn feedback_key(owner_user_id: &str, message_id: &str) -> String {
    format!("{owner_user_id}:{message_id}")
}

fn feedback_reason_tags_from_json(value: serde_json::Value) -> Result<Vec<String>> {
    serde_json::from_value::<Vec<String>>(value).context("parse feedback reason tags")
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
        let now = Utc::now();
        let principal = AuthPrincipal {
            user_id: owner_user_id.clone(),
            email: input.owner_email.clone(),
            role: EnterpriseRole::Admin,
        };
        state
            .token_hashes
            .insert(input.issued_token_hash, owner_user_id.clone());
        state
            .password_hashes
            .insert(owner_user_id.clone(), input.owner_password_hash);
        state.users.insert(owner_user_id.clone(), principal);
        state
            .user_statuses
            .insert(owner_user_id.clone(), UserStatus::Active);
        state.user_created_at.insert(owner_user_id.clone(), now);
        state.user_updated_at.insert(owner_user_id.clone(), now);

        let workspace_roots = canonicalize_workspace_roots(input.workspace_roots)?;
        let outcome = BootstrapOutcome {
            owner_user_id: owner_user_id.clone(),
            owner_email: input.owner_email.clone(),
            receipt: BootstrapReceipt::new(
                SetupMode::EnterpriseServer,
                input.owner_email,
                workspace_roots.clone(),
            ),
        };
        state.workspace_roots = workspace_roots.clone();
        state.workspaces = workspace_roots
            .iter()
            .map(|root| WorkspaceRootRecord {
                workspace_id: root.clone(),
                root_path: root.clone(),
                created_by: owner_user_id.clone(),
                created_at: now,
            })
            .collect();
        state.bootstrap = Some(outcome.clone());
        Ok(outcome)
    }

    async fn authenticate_api_token(&self, plaintext_token: &str) -> Result<Option<AuthPrincipal>> {
        let state = self.state.lock().await;
        let token_hash = auth::api_token_hash(plaintext_token);
        let Some(user_id) = state.token_hashes.get(&token_hash) else {
            return Ok(None);
        };

        if state.user_statuses.get(user_id) != Some(&UserStatus::Active) {
            return Ok(None);
        }
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
        if state.user_statuses.get(&principal.user_id) != Some(&UserStatus::Active) {
            return Ok(None);
        }
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

    async fn create_user(
        &self,
        _principal: &AuthPrincipal,
        input: CreateUserInput,
    ) -> Result<UserRecord> {
        let mut state = self.state.lock().await;
        if state.users.values().any(|user| user.email == input.email) {
            anyhow::bail!("user email already exists");
        }
        let now = Utc::now();
        let user_id = Uuid::new_v4().to_string();
        let principal = AuthPrincipal {
            user_id: user_id.clone(),
            email: input.email,
            role: input.role,
        };
        state
            .password_hashes
            .insert(user_id.clone(), input.password_hash);
        state
            .user_statuses
            .insert(user_id.clone(), UserStatus::Active);
        state.user_created_at.insert(user_id.clone(), now);
        state.user_updated_at.insert(user_id.clone(), now);
        state.users.insert(user_id, principal.clone());
        for workspace_root in input.workspace_roots {
            let workspace_root = authorize_workspace(&state.workspace_roots, workspace_root)?;
            let user_workspace_id =
                ensure_user_workspace_record(&mut state, &principal.user_id, &workspace_root, now);
            state.workspace_assignments.push(WorkspaceAssignmentRecord {
                assignment_id: Uuid::new_v4().to_string(),
                user_id: principal.user_id.clone(),
                workspace_id: Some(user_workspace_id),
                workspace_root,
                assigned_by: _principal.user_id.clone(),
                created_at: now,
            });
        }
        Ok(user_record_from_memory(&state, &principal)?)
    }

    async fn list_users(&self, _principal: &AuthPrincipal) -> Result<Vec<UserRecord>> {
        let state = self.state.lock().await;
        let mut users = state
            .users
            .values()
            .map(|user| user_record_from_memory(&state, user))
            .collect::<Result<Vec<_>>>()?;
        users.sort_by(|left, right| left.email.cmp(&right.email));
        Ok(users)
    }

    async fn assign_user_role(
        &self,
        _principal: &AuthPrincipal,
        user_id: &str,
        role: EnterpriseRole,
    ) -> Result<UserRecord> {
        let mut state = self.state.lock().await;
        ensure_admin_role_change_allowed(&state, _principal, user_id, role)?;
        {
            let user = state.users.get_mut(user_id).context("user not found")?;
            user.role = role;
        }
        state
            .user_updated_at
            .insert(user_id.to_string(), Utc::now());
        let user = state.users.get(user_id).context("user not found")?;
        user_record_from_memory(&state, user)
    }

    async fn set_user_status(
        &self,
        _principal: &AuthPrincipal,
        user_id: &str,
        status: UserStatus,
    ) -> Result<UserRecord> {
        let mut state = self.state.lock().await;
        ensure_admin_status_change_allowed(&state, _principal, user_id, status)?;
        let user = state.users.get(user_id).context("user not found")?.clone();
        state.user_statuses.insert(user_id.to_string(), status);
        state
            .user_updated_at
            .insert(user_id.to_string(), Utc::now());
        user_record_from_memory(&state, &user)
    }

    async fn register_workspace_root(
        &self,
        principal: &AuthPrincipal,
        root_path: String,
    ) -> Result<WorkspaceRootRecord> {
        let mut state = self.state.lock().await;
        let mut roots = canonicalize_workspace_roots(vec![root_path])?;
        let root_path = roots.pop().context("canonical workspace root")?;
        if state.workspace_roots.contains(&root_path) {
            anyhow::bail!("workspace root already registered");
        }
        let workspace = WorkspaceRootRecord {
            workspace_id: root_path.clone(),
            root_path: root_path.clone(),
            created_by: principal.user_id.clone(),
            created_at: Utc::now(),
        };
        state.workspace_roots.push(root_path);
        state.workspaces.push(workspace.clone());
        Ok(workspace)
    }

    async fn validate_workspace_path(
        &self,
        _principal: &AuthPrincipal,
        workspace_path: String,
    ) -> Result<String> {
        let state = self.state.lock().await;
        authorize_workspace(
            &authorized_roots_for_user(&state, _principal),
            workspace_path,
        )
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
            &authorized_roots_for_user(&state, principal),
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
            .filter(|session| {
                session.owner_user_id == principal.user_id && session.deleted_at.is_none()
            })
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
            .filter(|session| {
                session.owner_user_id == principal.user_id && session.deleted_at.is_none()
            })
            .cloned()
            .context("session not found")
    }

    async fn update_session_title(
        &self,
        principal: &AuthPrincipal,
        session_id: &str,
        input: UpdateSessionTitleInput,
    ) -> Result<SessionRecord> {
        let mut state = self.state.lock().await;
        let session = state
            .sessions
            .get_mut(session_id)
            .filter(|session| {
                session.owner_user_id == principal.user_id && session.deleted_at.is_none()
            })
            .context("session not found")?;
        session.title = Some(input.title);
        session.updated_at = Utc::now();
        Ok(session.clone())
    }

    async fn delete_session(
        &self,
        principal: &AuthPrincipal,
        session_id: &str,
    ) -> Result<SessionRecord> {
        let mut state = self.state.lock().await;
        let session = state
            .sessions
            .get_mut(session_id)
            .filter(|session| {
                session.owner_user_id == principal.user_id && session.deleted_at.is_none()
            })
            .context("session not found")?;
        let now = Utc::now();
        session.deleted_at = Some(now);
        session.updated_at = now;
        Ok(session.clone())
    }

    async fn create_session_message(
        &self,
        principal: &AuthPrincipal,
        session_id: &str,
        input: CreateSessionMessageInput,
    ) -> Result<SessionMessageRecord> {
        let mut state = self.state.lock().await;
        state
            .sessions
            .get(session_id)
            .filter(|session| {
                session.owner_user_id == principal.user_id && session.deleted_at.is_none()
            })
            .context("session not found")?;
        let messages = state
            .session_messages
            .entry(session_id.to_string())
            .or_default();
        let message = SessionMessageRecord {
            message_id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            owner_user_id: principal.user_id.clone(),
            kind: input.kind,
            label: input.label,
            text: input.text,
            sequence: messages.len() as i64 + 1,
            feedback_rating: None,
            created_at: Utc::now(),
        };
        messages.push(message.clone());
        if let Some(session) = state.sessions.get_mut(session_id) {
            session.updated_at = Utc::now();
        }
        Ok(message)
    }

    async fn list_session_messages(
        &self,
        principal: &AuthPrincipal,
        session_id: &str,
    ) -> Result<Vec<SessionMessageRecord>> {
        let state = self.state.lock().await;
        state
            .sessions
            .get(session_id)
            .filter(|session| {
                session.owner_user_id == principal.user_id && session.deleted_at.is_none()
            })
            .context("session not found")?;
        let feedback_by_message = state
            .response_feedback
            .values()
            .filter(|feedback| feedback.owner_user_id == principal.user_id)
            .map(|feedback| (feedback.message_id.clone(), feedback.rating.clone()))
            .collect::<HashMap<_, _>>();
        Ok(state
            .session_messages
            .get(session_id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|mut message| {
                message.feedback_rating = feedback_by_message.get(&message.message_id).cloned();
                message
            })
            .collect())
    }

    async fn upsert_response_feedback(
        &self,
        principal: &AuthPrincipal,
        session_id: &str,
        message_id: &str,
        input: UpsertResponseFeedbackInput,
    ) -> Result<(ResponseFeedbackRecord, UserResponsePreferencesRecord)> {
        let mut state = self.state.lock().await;
        state
            .sessions
            .get(session_id)
            .filter(|session| {
                session.owner_user_id == principal.user_id && session.deleted_at.is_none()
            })
            .context("feedback message not found")?;
        let message = state
            .session_messages
            .get(session_id)
            .and_then(|messages| {
                messages
                    .iter()
                    .find(|message| message.message_id == message_id)
            })
            .filter(|message| message.owner_user_id == principal.user_id)
            .context("feedback message not found")?;
        if message.kind != "assistant" {
            anyhow::bail!("feedback target must be an assistant message");
        }
        let now = Utc::now();
        let key = feedback_key(&principal.user_id, message_id);
        let feedback = if let Some(existing) = state.response_feedback.get_mut(&key) {
            existing.rating = input.rating;
            existing.reason_tags = input.reason_tags;
            existing.comment = input.comment;
            existing.updated_at = now;
            existing.clone()
        } else {
            let record = ResponseFeedbackRecord {
                feedback_id: Uuid::new_v4().to_string(),
                owner_user_id: principal.user_id.clone(),
                session_id: session_id.to_string(),
                message_id: message_id.to_string(),
                rating: input.rating,
                reason_tags: input.reason_tags,
                comment: input.comment,
                created_at: now,
                updated_at: now,
            };
            state.response_feedback.insert(key, record.clone());
            record
        };
        let preferences = build_response_preferences_from_feedback(
            &principal.user_id,
            state.response_feedback.values(),
        );
        state
            .response_preferences
            .insert(principal.user_id.clone(), preferences.clone());
        Ok((feedback, preferences))
    }

    async fn get_response_preferences(
        &self,
        principal: &AuthPrincipal,
    ) -> Result<UserResponsePreferencesRecord> {
        let state = self.state.lock().await;
        Ok(state
            .response_preferences
            .get(&principal.user_id)
            .cloned()
            .unwrap_or_else(|| empty_response_preferences(&principal.user_id)))
    }

    async fn reset_response_preferences(&self, principal: &AuthPrincipal) -> Result<()> {
        let mut state = self.state.lock().await;
        state.response_preferences.remove(&principal.user_id);
        Ok(())
    }

    async fn start_worker(
        &self,
        principal: &AuthPrincipal,
        workspace_path: String,
        session_id: String,
    ) -> Result<WorkerRecord> {
        let mut state = self.state.lock().await;
        let authorized_roots = authorized_roots_for_user(&state, principal);
        let resolved_workspace_path = authorize_workspace(&authorized_roots, workspace_path)?;
        let workspace_roots = authorized_roots.clone();
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
        Ok(authorized_roots_for_user(&state, _principal))
    }

    async fn list_user_workspaces(
        &self,
        principal: &AuthPrincipal,
    ) -> Result<Vec<UserWorkspaceRecord>> {
        let state = self.state.lock().await;
        let mut workspaces = state
            .user_workspaces
            .iter()
            .filter(|workspace| {
                workspace.owner_user_id == principal.user_id
                    || matches!(
                        principal.role,
                        EnterpriseRole::Admin | EnterpriseRole::Owner | EnterpriseRole::Manager
                    )
            })
            .cloned()
            .collect::<Vec<_>>();
        workspaces.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(workspaces)
    }

    async fn assign_workspace_to_user(
        &self,
        principal: &AuthPrincipal,
        user_id: &str,
        workspace_root: String,
    ) -> Result<WorkspaceAssignmentRecord> {
        let mut state = self.state.lock().await;
        if !state.users.contains_key(user_id) {
            anyhow::bail!("workspace assignment user not found");
        }
        let grantable_roots = grantable_roots_for_user(&state, principal);
        let workspace_root = authorize_workspace(&grantable_roots, workspace_root)?;
        if state.workspace_assignments.iter().any(|assignment| {
            assignment.user_id == user_id && assignment.workspace_root == workspace_root
        }) {
            anyhow::bail!("workspace assignment already exists");
        }
        let assignment = WorkspaceAssignmentRecord {
            assignment_id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            workspace_id: Some(ensure_user_workspace_record(
                &mut state,
                user_id,
                &workspace_root,
                Utc::now(),
            )),
            workspace_root: workspace_root.clone(),
            assigned_by: principal.user_id.clone(),
            created_at: Utc::now(),
        };
        state.workspace_assignments.push(assignment.clone());
        Ok(assignment)
    }

    async fn list_projects(
        &self,
        principal: &AuthPrincipal,
        include_deleted: bool,
    ) -> Result<Vec<ProjectRecord>> {
        let state = self.state.lock().await;
        let mut projects = state
            .projects
            .iter()
            .filter(|project| include_deleted || project.deleted_at.is_none())
            .filter(|project| {
                project.owner_user_id == principal.user_id
                    || user_can_access_project(&state, principal, project)
            })
            .map(|project| hydrate_project_record(&state, project))
            .collect::<Vec<_>>();
        projects.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(projects)
    }

    async fn create_project(
        &self,
        principal: &AuthPrincipal,
        input: CreateProjectInput,
    ) -> Result<ProjectRecord> {
        let mut state = self.state.lock().await;
        let user_workspace = select_user_workspace(&state, principal, input.user_workspace_id)?;
        let slug = slugify_name(&input.name)?;
        let project_path = PathBuf::from(&user_workspace.path)
            .join("projects")
            .join(&slug);
        fs::create_dir_all(project_path.join("repos"))
            .with_context(|| format!("create project path {}", project_path.display()))?;
        fs::create_dir_all(project_path.join("outputs"))
            .with_context(|| format!("create project outputs {}", project_path.display()))?;
        let project_path = project_path
            .canonicalize()
            .with_context(|| format!("canonicalize project path {}", project_path.display()))?;
        let project_path_string = project_path.to_string_lossy().to_string();
        if state.projects.iter().any(|project| {
            project.user_workspace_id == user_workspace.user_workspace_id && project.slug == slug
        }) {
            anyhow::bail!("project already exists in user workspace");
        }
        let now = Utc::now();
        let project = ProjectRecord {
            project_id: Uuid::new_v4().to_string(),
            owner_user_id: user_workspace.owner_user_id.clone(),
            user_workspace_id: user_workspace.user_workspace_id,
            name: input.name,
            slug,
            project_path: project_path_string,
            repositories: Vec::new(),
            threads: Vec::new(),
            created_at: now,
            updated_at: now,
            deleted_at: None,
        };
        state.projects.push(project.clone());
        Ok(project)
    }

    async fn update_project(
        &self,
        principal: &AuthPrincipal,
        project_id: &str,
        input: UpdateProjectInput,
    ) -> Result<ProjectRecord> {
        let mut state = self.state.lock().await;
        let index = state
            .projects
            .iter()
            .position(|project| {
                project.project_id == project_id
                    && (project.owner_user_id == principal.user_id
                        || user_can_access_project(&state, principal, project))
            })
            .context("project not found")?;
        slugify_name(&input.name)?;
        state.projects[index].name = input.name;
        state.projects[index].updated_at = Utc::now();
        Ok(hydrate_project_record(&state, &state.projects[index]))
    }

    async fn delete_project(
        &self,
        principal: &AuthPrincipal,
        project_id: &str,
    ) -> Result<ProjectRecord> {
        let mut state = self.state.lock().await;
        let index = state
            .projects
            .iter()
            .position(|project| {
                project.project_id == project_id
                    && (project.owner_user_id == principal.user_id
                        || user_can_access_project(&state, principal, project))
            })
            .context("project not found")?;
        let now = Utc::now();
        state.projects[index].deleted_at = Some(now);
        state.projects[index].updated_at = now;
        Ok(hydrate_project_record(&state, &state.projects[index]))
    }

    async fn restore_project(
        &self,
        principal: &AuthPrincipal,
        project_id: &str,
    ) -> Result<ProjectRecord> {
        let mut state = self.state.lock().await;
        let index = state
            .projects
            .iter()
            .position(|project| {
                project.project_id == project_id
                    && matches!(
                        principal.role,
                        EnterpriseRole::Admin | EnterpriseRole::Owner | EnterpriseRole::Manager
                    )
            })
            .context("project not found")?;
        state.projects[index].deleted_at = None;
        state.projects[index].updated_at = Utc::now();
        Ok(hydrate_project_record(&state, &state.projects[index]))
    }

    async fn create_project_thread(
        &self,
        principal: &AuthPrincipal,
        input: CreateProjectThreadInput,
    ) -> Result<SessionRecord> {
        let mut state = self.state.lock().await;
        let project = state
            .projects
            .iter()
            .find(|project| project.project_id == input.project_id)
            .filter(|project| project.deleted_at.is_none())
            .filter(|project| {
                project.owner_user_id == principal.user_id
                    || user_can_access_project(&state, principal, project)
            })
            .cloned()
            .context("project not found")?;
        let workspace_path = match input.repository_id.as_ref() {
            Some(repository_id) => state
                .repositories
                .iter()
                .find(|repository| {
                    repository.repository_id == *repository_id
                        && repository.project_id == project.project_id
                })
                .map(|repository| repository.repository_path.clone())
                .context("repository not found")?,
            None => project.project_path.clone(),
        };
        let mut session = create_session_record(
            &authorized_roots_for_user(&state, principal),
            principal,
            input.session_id,
            workspace_path,
            input.title,
        )?;
        session.project_id = Some(project.project_id.clone());
        session.repository_id = input.repository_id;
        if state.sessions.contains_key(&session.session_id) {
            anyhow::bail!("session already exists");
        }
        state
            .sessions
            .insert(session.session_id.clone(), session.clone());
        if let Some(project) = state
            .projects
            .iter_mut()
            .find(|project| project.project_id == session.project_id.as_deref().unwrap_or(""))
        {
            project.updated_at = Utc::now();
        }
        Ok(session)
    }

    async fn clone_project_repository(
        &self,
        principal: &AuthPrincipal,
        input: CloneProjectRepositoryInput,
    ) -> Result<RepositoryRecord> {
        let project = {
            let state = self.state.lock().await;
            state
                .projects
                .iter()
                .find(|project| project.project_id == input.project_id)
                .filter(|project| project.deleted_at.is_none())
                .filter(|project| {
                    project.owner_user_id == principal.user_id
                        || user_can_access_project(&state, principal, project)
                })
                .cloned()
                .context("project not found")?
        };
        let repos_dir = PathBuf::from(&project.project_path).join("repos");
        fs::create_dir_all(&repos_dir)
            .with_context(|| format!("create project repositories path {}", repos_dir.display()))?;
        let plan =
            crate::repo_clone::plan_clone(&input.repo_url, &repos_dir, &input.destination_name)?;
        crate::repo_clone::clone_repo(&plan).await?;
        let mut state = self.state.lock().await;
        let repository = RepositoryRecord {
            repository_id: Uuid::new_v4().to_string(),
            project_id: project.project_id.clone(),
            repo_url: Some(crate::repo_clone::redact_repo_url_for_storage(
                &input.repo_url,
            )),
            name: input.destination_name,
            repository_path: plan.destination_path,
            created_at: Utc::now(),
        };
        state.repositories.push(repository.clone());
        if let Some(project) = state
            .projects
            .iter_mut()
            .find(|project| project.project_id == repository.project_id)
        {
            project.updated_at = Utc::now();
        }
        Ok(repository)
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

    async fn create_context_pack(
        &self,
        _principal: &AuthPrincipal,
        input: CreateContextPackInput,
    ) -> Result<(ContextPackRecord, Vec<ContextDocumentRecord>)> {
        let validated = context_packs::validate_documents(&input.documents)?;
        let mut state = self.state.lock().await;
        let pack_id = Uuid::new_v4().to_string();
        let pack = ContextPackRecord {
            pack_id: pack_id.clone(),
            name: input.name,
            status: "active".to_string(),
            created_at: Utc::now(),
        };
        let documents = validated
            .documents
            .into_iter()
            .map(|document| ContextDocumentRecord {
                document_id: Uuid::new_v4().to_string(),
                pack_id: pack_id.clone(),
                filename: document.filename,
                content_hash: document.content_hash,
                load_order: document.load_order,
                required: document.required,
            })
            .collect::<Vec<_>>();
        state.context_packs.insert(pack_id, pack.clone());
        state.context_documents.extend(documents.clone());
        Ok((pack, documents))
    }

    async fn list_context_packs(
        &self,
        _principal: &AuthPrincipal,
    ) -> Result<Vec<ContextPackRecord>> {
        let state = self.state.lock().await;
        let mut packs = state.context_packs.values().cloned().collect::<Vec<_>>();
        packs.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(packs)
    }

    async fn assign_context_pack(
        &self,
        _principal: &AuthPrincipal,
        input: AssignContextPackInput,
    ) -> Result<ContextPackAssignmentRecord> {
        let mut state = self.state.lock().await;
        if !state.context_packs.contains_key(&input.pack_id) {
            anyhow::bail!("context pack not found");
        }
        let workspace_id = input
            .workspace_id
            .map(|workspace| authorize_workspace(&state.workspace_roots, workspace))
            .transpose()?;
        if state.context_assignments.iter().any(|assignment| {
            assignment.user_id == input.user_id
                && assignment.workspace_id == workspace_id
                && assignment.assignment_order == input.assignment_order
        }) {
            anyhow::bail!("context pack assignment load order is ambiguous");
        }
        let assignment_source = match (&input.user_id, &workspace_id) {
            (Some(_), Some(_)) => "user_workspace",
            (Some(_), None) => "user",
            (None, Some(_)) => "workspace",
            (None, None) => "global",
        }
        .to_string();
        let assignment = ContextPackAssignmentRecord {
            assignment_id: Uuid::new_v4().to_string(),
            pack_id: input.pack_id,
            user_id: input.user_id,
            workspace_id,
            assignment_source,
            assignment_order: input.assignment_order,
            required_session: input.required_session,
            required_worker: input.required_worker,
        };
        state.context_assignments.push(assignment.clone());
        Ok(assignment)
    }

    async fn list_context_pack_assignments(
        &self,
        _principal: &AuthPrincipal,
    ) -> Result<Vec<ContextPackAssignmentRecord>> {
        let state = self.state.lock().await;
        let mut assignments = state.context_assignments.clone();
        assignments.sort_by(|left, right| {
            left.assignment_order
                .cmp(&right.assignment_order)
                .then_with(|| left.assignment_id.cmp(&right.assignment_id))
        });
        Ok(assignments)
    }

    async fn delete_context_pack_assignment(
        &self,
        _principal: &AuthPrincipal,
        assignment_id: &str,
    ) -> Result<ContextPackAssignmentRecord> {
        let mut state = self.state.lock().await;
        let index = state
            .context_assignments
            .iter()
            .position(|assignment| assignment.assignment_id == assignment_id)
            .context("context pack assignment not found")?;
        Ok(state.context_assignments.remove(index))
    }

    async fn record_context_load(
        &self,
        input: ContextLoadInput,
    ) -> Result<Vec<ContextPackReceiptRecord>> {
        let mut state = self.state.lock().await;
        let required_worker_phase = input.phase == "worker_start";
        let mut assignments = state
            .context_assignments
            .iter()
            .filter(|assignment| {
                assignment_required_for_phase(assignment, required_worker_phase)
                    && assignment_matches(assignment, &input.actor_user_id, &input.workspace_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        assignments.sort_by_key(|assignment| assignment.assignment_order);

        let mut receipts = Vec::new();
        for assignment in assignments {
            let documents = state
                .context_documents
                .iter()
                .filter(|document| document.pack_id == assignment.pack_id)
                .cloned()
                .collect::<Vec<_>>();
            for document in documents {
                receipts.push(ContextPackReceiptRecord {
                    receipt_id: Uuid::new_v4().to_string(),
                    trace_id: input.trace_id.clone(),
                    pack_id: assignment.pack_id.clone(),
                    document_id: document.document_id,
                    content_hash: document.content_hash,
                    load_order: assignment.assignment_order + document.load_order,
                    assignment_source: assignment.assignment_source.clone(),
                    actor_user_id: input.actor_user_id.clone(),
                    workspace_id: input.workspace_id.clone(),
                    session_id: input.session_id.clone(),
                    worker_id: input.worker_id.clone(),
                    phase: input.phase.clone(),
                    created_at: Utc::now(),
                });
            }
        }
        state.context_receipts.extend(receipts.clone());
        Ok(receipts)
    }

    async fn query_evidence(
        &self,
        _principal: &AuthPrincipal,
        query: EvidenceQuery,
    ) -> Result<EvidenceQueryResult> {
        let state = self.state.lock().await;
        let audit_events = state
            .audit_events
            .iter()
            .filter(|event| {
                query
                    .trace_id
                    .as_ref()
                    .is_none_or(|trace| event.trace_id == *trace)
            })
            .cloned()
            .collect();
        let execution_receipts = state
            .execution_receipts
            .iter()
            .filter(|event| {
                query
                    .trace_id
                    .as_ref()
                    .is_none_or(|trace| event.trace_id == *trace)
            })
            .cloned()
            .collect();
        let context_receipts = state
            .context_receipts
            .iter()
            .filter(|event| {
                query
                    .trace_id
                    .as_ref()
                    .is_none_or(|trace| event.trace_id == *trace)
            })
            .cloned()
            .collect();
        Ok(EvidenceQueryResult {
            audit_events,
            execution_receipts,
            context_receipts,
        })
    }

    async fn create_output(&self, input: CreateOutputInput) -> Result<OutputRecord> {
        validate_output_artifact_path(&input.artifact_path)?;
        {
            let state = self.state.lock().await;
            if !state.users.contains_key(&input.owner_user_id) {
                anyhow::bail!("output owner user not found");
            }
        }
        let now = Utc::now();
        let output = OutputRecord {
            output_id: Uuid::new_v4().to_string(),
            owner_user_id: input.owner_user_id,
            workspace_id: input.workspace_id,
            session_id: input.session_id,
            worker_id: input.worker_id,
            category: input.category,
            output_type: input.output_type,
            title: input.title,
            artifact_path: input.artifact_path,
            status: input.status,
            metadata_json: input.metadata_json,
            created_at: now,
            updated_at: now,
        };
        self.state.lock().await.outputs.push(output.clone());
        Ok(output)
    }

    async fn list_outputs(&self, principal: &AuthPrincipal) -> Result<Vec<OutputRecord>> {
        let state = self.state.lock().await;
        let mut outputs = state
            .outputs
            .iter()
            .filter(|output| output.owner_user_id == principal.user_id)
            .cloned()
            .collect::<Vec<_>>();
        outputs.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        Ok(outputs)
    }

    async fn get_output(&self, principal: &AuthPrincipal, output_id: &str) -> Result<OutputRecord> {
        let state = self.state.lock().await;
        state
            .outputs
            .iter()
            .find(|output| {
                output.output_id == output_id && output.owner_user_id == principal.user_id
            })
            .cloned()
            .context("output not found")
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
             VALUES ($1, $2, $3, 'admin')",
        )
        .bind(owner_user_id)
        .bind(&input.owner_email)
        .bind(&input.owner_password_hash)
        .execute(&mut *tx)
        .await
        .context("insert owner user")?;

        sqlx::query(
            "INSERT INTO enterprise_api_tokens (token_id, user_id, label, token_hash)
             VALUES ($1, $2, 'admin-bootstrap', $3)",
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
             WHERE t.token_hash = $1 AND t.revoked_at IS NULL AND u.status = 'active'",
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
             WHERE email = $1 AND status = 'active'",
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

    async fn create_user(
        &self,
        _principal: &AuthPrincipal,
        input: CreateUserInput,
    ) -> Result<UserRecord> {
        let row = sqlx::query(
            "INSERT INTO enterprise_users (user_id, email, password_hash, role, status)
             VALUES ($1, $2, $3, $4, 'active')
             RETURNING user_id::text, email, role, status, created_at, updated_at",
        )
        .bind(Uuid::new_v4())
        .bind(&input.email)
        .bind(&input.password_hash)
        .bind(input.role.as_str())
        .fetch_one(&self.pool)
        .await
        .context("insert enterprise user")?;
        let user = user_from_row(row)?;
        for workspace_root in input.workspace_roots {
            self.assign_workspace_to_user(_principal, &user.user_id, workspace_root)
                .await?;
        }
        Ok(user)
    }

    async fn list_users(&self, _principal: &AuthPrincipal) -> Result<Vec<UserRecord>> {
        let rows = sqlx::query(
            "SELECT user_id::text, email, role, status, created_at, updated_at
             FROM enterprise_users
             ORDER BY email",
        )
        .fetch_all(&self.pool)
        .await
        .context("list enterprise users")?;
        rows.into_iter().map(user_from_row).collect()
    }

    async fn assign_user_role(
        &self,
        _principal: &AuthPrincipal,
        user_id: &str,
        role: EnterpriseRole,
    ) -> Result<UserRecord> {
        self.ensure_admin_role_change_allowed(_principal, user_id, role)
            .await?;
        let row = sqlx::query(
            "UPDATE enterprise_users
             SET role = $1, updated_at = now()
             WHERE user_id = $2
             RETURNING user_id::text, email, role, status, created_at, updated_at",
        )
        .bind(role.as_str())
        .bind(Uuid::parse_str(user_id).context("parse user id")?)
        .fetch_optional(&self.pool)
        .await
        .context("assign user role")?
        .context("user not found")?;
        user_from_row(row)
    }

    async fn set_user_status(
        &self,
        _principal: &AuthPrincipal,
        user_id: &str,
        status: UserStatus,
    ) -> Result<UserRecord> {
        self.ensure_admin_status_change_allowed(_principal, user_id, status)
            .await?;
        let row = sqlx::query(
            "UPDATE enterprise_users
             SET status = $1, updated_at = now()
             WHERE user_id = $2
             RETURNING user_id::text, email, role, status, created_at, updated_at",
        )
        .bind(status.as_str())
        .bind(Uuid::parse_str(user_id).context("parse user id")?)
        .fetch_optional(&self.pool)
        .await
        .context("set user status")?
        .context("user not found")?;
        user_from_row(row)
    }

    async fn register_workspace_root(
        &self,
        principal: &AuthPrincipal,
        root_path: String,
    ) -> Result<WorkspaceRootRecord> {
        let mut roots = canonicalize_workspace_roots(vec![root_path])?;
        let root_path = roots.pop().context("canonical workspace root")?;
        let row = sqlx::query(
            "INSERT INTO enterprise_workspaces (workspace_id, root_path, created_by)
             VALUES ($1, $2, $3)
             RETURNING workspace_id::text, root_path, created_by::text, created_at",
        )
        .bind(Uuid::new_v4())
        .bind(&root_path)
        .bind(Uuid::parse_str(&principal.user_id).context("parse principal user id")?)
        .fetch_one(&self.pool)
        .await
        .context("register workspace root")?;
        workspace_from_row(row)
    }

    async fn validate_workspace_path(
        &self,
        _principal: &AuthPrincipal,
        workspace_path: String,
    ) -> Result<String> {
        self.authorize_workspace_path(_principal, workspace_path)
            .await
    }

    async fn create_session(
        &self,
        principal: &AuthPrincipal,
        session_id: Option<String>,
        workspace_path: String,
        title: Option<String>,
    ) -> Result<SessionRecord> {
        let session_id = session_id.unwrap_or_else(|| Uuid::new_v4().to_string());
        let resolved_workspace_path = self
            .authorize_workspace_path(principal, workspace_path)
            .await?;
        let row = sqlx::query(
            "INSERT INTO enterprise_sessions
             (session_id, owner_user_id, workspace_id, workspace_path, title)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING session_id, owner_user_id::text, workspace_id, workspace_path, title, last_worker_id::text, deleted_at, created_at, updated_at",
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
            "SELECT session_id, owner_user_id::text, workspace_id, workspace_path, title, last_worker_id::text, deleted_at, created_at, updated_at
             FROM enterprise_sessions
             WHERE owner_user_id = $1 AND deleted_at IS NULL
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
            "SELECT session_id, owner_user_id::text, workspace_id, workspace_path, title, last_worker_id::text, deleted_at, created_at, updated_at
             FROM enterprise_sessions
             WHERE session_id = $1 AND owner_user_id = $2 AND deleted_at IS NULL",
        )
        .bind(session_id)
        .bind(Uuid::parse_str(&principal.user_id).context("parse principal user id")?)
        .fetch_optional(&self.pool)
        .await
        .context("get session record")?
        .context("session not found")?;

        session_from_row(row)
    }

    async fn update_session_title(
        &self,
        principal: &AuthPrincipal,
        session_id: &str,
        input: UpdateSessionTitleInput,
    ) -> Result<SessionRecord> {
        let row = sqlx::query(
            "UPDATE enterprise_sessions
             SET title = $3, updated_at = now()
             WHERE session_id = $1 AND owner_user_id = $2 AND deleted_at IS NULL
             RETURNING session_id, owner_user_id::text, workspace_id, workspace_path, project_id::text, repository_id::text, title, last_worker_id::text, deleted_at, created_at, updated_at",
        )
        .bind(session_id)
        .bind(Uuid::parse_str(&principal.user_id).context("parse principal user id")?)
        .bind(input.title)
        .fetch_optional(&self.pool)
        .await
        .context("update session title")?
        .context("session not found")?;

        session_from_row(row)
    }

    async fn delete_session(
        &self,
        principal: &AuthPrincipal,
        session_id: &str,
    ) -> Result<SessionRecord> {
        let row = sqlx::query(
            "UPDATE enterprise_sessions
             SET deleted_at = now(), updated_at = now()
             WHERE session_id = $1 AND owner_user_id = $2 AND deleted_at IS NULL
             RETURNING session_id, owner_user_id::text, workspace_id, workspace_path, project_id::text, repository_id::text, title, last_worker_id::text, deleted_at, created_at, updated_at",
        )
        .bind(session_id)
        .bind(Uuid::parse_str(&principal.user_id).context("parse principal user id")?)
        .fetch_optional(&self.pool)
        .await
        .context("delete session")?
        .context("session not found")?;

        session_from_row(row)
    }

    async fn create_session_message(
        &self,
        principal: &AuthPrincipal,
        session_id: &str,
        input: CreateSessionMessageInput,
    ) -> Result<SessionMessageRecord> {
        let principal_user_id =
            Uuid::parse_str(&principal.user_id).context("parse principal user id")?;
        let row = sqlx::query(
            "WITH owned_session AS (
                 SELECT session_id, owner_user_id
                 FROM enterprise_sessions
                 WHERE session_id = $1 AND owner_user_id = $2 AND deleted_at IS NULL
             ),
             next_sequence AS (
                 SELECT COALESCE(MAX(sequence), 0) + 1 AS sequence
                 FROM enterprise_session_messages
                 WHERE session_id = $1
             ),
             inserted AS (
                 INSERT INTO enterprise_session_messages
                 (message_id, session_id, owner_user_id, kind, label, text, sequence)
                 SELECT $3, owned_session.session_id, owned_session.owner_user_id, $4, $5, $6, next_sequence.sequence
                 FROM owned_session, next_sequence
                 RETURNING message_id::text, session_id, owner_user_id::text, kind, label, text, sequence, created_at
             )
             UPDATE enterprise_sessions
             SET updated_at = now()
             WHERE session_id = $1 AND owner_user_id = $2 AND deleted_at IS NULL
             RETURNING (
                 SELECT message_id FROM inserted
             ) AS message_id,
             (
                 SELECT session_id FROM inserted
             ) AS session_id,
             (
                 SELECT owner_user_id FROM inserted
             ) AS owner_user_id,
             (
                 SELECT kind FROM inserted
             ) AS kind,
             (
                 SELECT label FROM inserted
             ) AS label,
             (
                 SELECT text FROM inserted
             ) AS text,
             (
                 SELECT sequence FROM inserted
             ) AS sequence,
             (
                 SELECT created_at FROM inserted
             ) AS created_at",
        )
        .bind(session_id)
        .bind(principal_user_id)
        .bind(Uuid::new_v4())
        .bind(input.kind)
        .bind(input.label)
        .bind(input.text)
        .fetch_optional(&self.pool)
        .await
        .context("insert session message")?
        .context("session not found")?;
        session_message_from_row(row)
    }

    async fn list_session_messages(
        &self,
        principal: &AuthPrincipal,
        session_id: &str,
    ) -> Result<Vec<SessionMessageRecord>> {
        self.get_session(principal, session_id).await?;
        let rows = sqlx::query(
            "SELECT messages.message_id::text,
                    messages.session_id,
                    messages.owner_user_id::text,
                    messages.kind,
                    messages.label,
                    messages.text,
                    messages.sequence,
                    feedback.rating AS feedback_rating,
                    messages.created_at
             FROM enterprise_session_messages messages
             LEFT JOIN enterprise_response_feedback feedback
               ON feedback.owner_user_id = messages.owner_user_id
              AND feedback.message_id = messages.message_id
             WHERE messages.session_id = $1 AND messages.owner_user_id = $2
             ORDER BY messages.sequence ASC",
        )
        .bind(session_id)
        .bind(Uuid::parse_str(&principal.user_id).context("parse principal user id")?)
        .fetch_all(&self.pool)
        .await
        .context("list session messages")?;

        rows.into_iter().map(session_message_from_row).collect()
    }

    async fn upsert_response_feedback(
        &self,
        principal: &AuthPrincipal,
        session_id: &str,
        message_id: &str,
        input: UpsertResponseFeedbackInput,
    ) -> Result<(ResponseFeedbackRecord, UserResponsePreferencesRecord)> {
        let principal_user_id =
            Uuid::parse_str(&principal.user_id).context("parse feedback owner user id")?;
        let message_uuid = Uuid::parse_str(message_id).context("parse feedback message id")?;
        let message_row = sqlx::query(
            "SELECT kind
             FROM enterprise_session_messages
             WHERE session_id = $1 AND message_id = $2 AND owner_user_id = $3
               AND EXISTS (
                   SELECT 1
                   FROM enterprise_sessions
                   WHERE session_id = $1 AND owner_user_id = $3 AND deleted_at IS NULL
               )",
        )
        .bind(session_id)
        .bind(message_uuid)
        .bind(principal_user_id)
        .fetch_optional(&self.pool)
        .await
        .context("find feedback message")?
        .context("feedback message not found")?;
        let kind: String = message_row.try_get("kind")?;
        if kind != "assistant" {
            anyhow::bail!("feedback target must be an assistant message");
        }

        let reason_tags_json =
            serde_json::to_value(&input.reason_tags).context("serialize feedback reason tags")?;
        let feedback_row = sqlx::query(
            "INSERT INTO enterprise_response_feedback
             (feedback_id, owner_user_id, session_id, message_id, rating, reason_tags, comment)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (owner_user_id, message_id)
             DO UPDATE SET
                rating = EXCLUDED.rating,
                reason_tags = EXCLUDED.reason_tags,
                comment = EXCLUDED.comment,
                updated_at = now()
             RETURNING feedback_id::text, owner_user_id::text, session_id, message_id::text, rating, reason_tags, comment, created_at, updated_at",
        )
        .bind(Uuid::new_v4())
        .bind(principal_user_id)
        .bind(session_id)
        .bind(message_uuid)
        .bind(input.rating)
        .bind(reason_tags_json)
        .bind(input.comment)
        .fetch_one(&self.pool)
        .await
        .context("upsert response feedback")?;
        let feedback = response_feedback_from_row(feedback_row)?;

        let rows = sqlx::query(
            "SELECT feedback_id::text, owner_user_id::text, session_id, message_id::text, rating, reason_tags, comment, created_at, updated_at
             FROM enterprise_response_feedback
             WHERE owner_user_id = $1",
        )
        .bind(principal_user_id)
        .fetch_all(&self.pool)
        .await
        .context("list response feedback for preferences")?;
        let feedback_records = rows
            .into_iter()
            .map(response_feedback_from_row)
            .collect::<Result<Vec<_>>>()?;
        let preferences =
            build_response_preferences_from_feedback(&principal.user_id, feedback_records.iter());
        let positive_tags_json =
            serde_json::to_value(&preferences.positive_tags).context("serialize positive tags")?;
        let negative_tags_json =
            serde_json::to_value(&preferences.negative_tags).context("serialize negative tags")?;
        let preference_row = sqlx::query(
            "INSERT INTO enterprise_user_response_preferences
             (owner_user_id, profile_summary, positive_tags, negative_tags, sample_count)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (owner_user_id)
             DO UPDATE SET
                profile_summary = EXCLUDED.profile_summary,
                positive_tags = EXCLUDED.positive_tags,
                negative_tags = EXCLUDED.negative_tags,
                sample_count = EXCLUDED.sample_count,
                updated_at = now()
             RETURNING owner_user_id::text, profile_summary, positive_tags, negative_tags, sample_count, updated_at",
        )
        .bind(principal_user_id)
        .bind(&preferences.profile_summary)
        .bind(positive_tags_json)
        .bind(negative_tags_json)
        .bind(preferences.sample_count)
        .fetch_one(&self.pool)
        .await
        .context("upsert response preferences")?;
        Ok((feedback, response_preferences_from_row(preference_row)?))
    }

    async fn get_response_preferences(
        &self,
        principal: &AuthPrincipal,
    ) -> Result<UserResponsePreferencesRecord> {
        let row = sqlx::query(
            "SELECT owner_user_id::text, profile_summary, positive_tags, negative_tags, sample_count, updated_at
             FROM enterprise_user_response_preferences
             WHERE owner_user_id = $1",
        )
        .bind(Uuid::parse_str(&principal.user_id).context("parse response preference user id")?)
        .fetch_optional(&self.pool)
        .await
        .context("get response preferences")?;
        row.map(response_preferences_from_row)
            .transpose()
            .map(|preferences| {
                preferences.unwrap_or_else(|| empty_response_preferences(&principal.user_id))
            })
    }

    async fn reset_response_preferences(&self, principal: &AuthPrincipal) -> Result<()> {
        sqlx::query("DELETE FROM enterprise_user_response_preferences WHERE owner_user_id = $1")
            .bind(Uuid::parse_str(&principal.user_id).context("parse response preference user id")?)
            .execute(&self.pool)
            .await
            .context("reset response preferences")?;
        Ok(())
    }

    async fn start_worker(
        &self,
        principal: &AuthPrincipal,
        workspace_path: String,
        session_id: String,
    ) -> Result<WorkerRecord> {
        let resolved_workspace_path = self
            .authorize_workspace_path(principal, workspace_path)
            .await?;
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

    async fn list_workspace_roots(&self, principal: &AuthPrincipal) -> Result<Vec<String>> {
        if matches!(
            principal.role,
            EnterpriseRole::Admin | EnterpriseRole::Owner
        ) {
            let rows =
                sqlx::query("SELECT root_path FROM enterprise_workspaces ORDER BY created_at")
                    .fetch_all(&self.pool)
                    .await
                    .context("list workspace roots")?;
            return rows
                .into_iter()
                .map(|row| row.try_get("root_path"))
                .collect::<Result<Vec<String>, sqlx::Error>>()
                .context("read workspace roots");
        }
        let rows = sqlx::query(
            "SELECT workspace_root FROM enterprise_workspace_assignments
             WHERE user_id = $1
             ORDER BY created_at",
        )
        .bind(Uuid::parse_str(&principal.user_id).context("parse workspace principal user id")?)
        .fetch_all(&self.pool)
        .await
        .context("list assigned workspace roots")?;
        rows.into_iter()
            .map(|row| row.try_get("workspace_root"))
            .collect::<Result<Vec<String>, sqlx::Error>>()
            .context("read workspace roots")
    }

    async fn list_user_workspaces(
        &self,
        principal: &AuthPrincipal,
    ) -> Result<Vec<UserWorkspaceRecord>> {
        let rows = if matches!(
            principal.role,
            EnterpriseRole::Admin | EnterpriseRole::Owner | EnterpriseRole::Manager
        ) {
            sqlx::query(
                "SELECT user_workspace_id::text, owner_user_id::text, workspace_root_id::text, path, created_at
                 FROM enterprise_user_workspaces
                 ORDER BY path",
            )
            .fetch_all(&self.pool)
            .await
            .context("list user workspaces")?
        } else {
            sqlx::query(
                "SELECT user_workspace_id::text, owner_user_id::text, workspace_root_id::text, path, created_at
                 FROM enterprise_user_workspaces
                 WHERE owner_user_id = $1
                 ORDER BY path",
            )
            .bind(Uuid::parse_str(&principal.user_id).context("parse principal user id")?)
            .fetch_all(&self.pool)
            .await
            .context("list user workspaces")?
        };
        rows.into_iter().map(user_workspace_from_row).collect()
    }

    async fn assign_workspace_to_user(
        &self,
        principal: &AuthPrincipal,
        user_id: &str,
        workspace_root: String,
    ) -> Result<WorkspaceAssignmentRecord> {
        let grantable_roots = if matches!(
            principal.role,
            EnterpriseRole::Admin | EnterpriseRole::Owner | EnterpriseRole::Manager
        ) {
            self.admin_workspace_roots().await?
        } else {
            self.list_workspace_roots(principal).await?
        };
        let workspace_root = authorize_workspace(&grantable_roots, workspace_root)?;
        let user_id = Uuid::parse_str(user_id).context("parse workspace assignment user id")?;
        let workspace_id = sqlx::query(
            "SELECT workspace_id FROM enterprise_workspaces WHERE $1 LIKE root_path || '%'
             ORDER BY length(root_path) DESC LIMIT 1",
        )
        .bind(&workspace_root)
        .fetch_optional(&self.pool)
        .await
        .context("find assignment workspace root")?
        .map(|row| row.try_get::<Uuid, _>("workspace_id"))
        .transpose()
        .context("read assignment workspace id")?;
        self.ensure_user_workspace_record(user_id, workspace_id, &workspace_root)
            .await?;
        let row = sqlx::query(
            "INSERT INTO enterprise_workspace_assignments
             (assignment_id, user_id, workspace_id, workspace_root, assigned_by)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING assignment_id::text, user_id::text, workspace_id::text, workspace_root, assigned_by::text, created_at",
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(workspace_id)
        .bind(&workspace_root)
        .bind(Uuid::parse_str(&principal.user_id).context("parse workspace assignment actor id")?)
        .fetch_one(&self.pool)
        .await
        .context("insert workspace assignment")?;
        workspace_assignment_from_row(row)
    }

    async fn list_projects(
        &self,
        principal: &AuthPrincipal,
        include_deleted: bool,
    ) -> Result<Vec<ProjectRecord>> {
        let rows = if matches!(
            principal.role,
            EnterpriseRole::Admin | EnterpriseRole::Owner | EnterpriseRole::Manager
        ) {
            sqlx::query(
                "SELECT project_id::text, owner_user_id::text, user_workspace_id::text, name, slug, project_path, created_at, updated_at, deleted_at
                 FROM enterprise_projects
                 WHERE $1 OR deleted_at IS NULL
                 ORDER BY name",
            )
            .bind(include_deleted)
            .fetch_all(&self.pool)
            .await
            .context("list projects")?
        } else {
            sqlx::query(
                "SELECT project_id::text, owner_user_id::text, user_workspace_id::text, name, slug, project_path, created_at, updated_at, deleted_at
                 FROM enterprise_projects
                 WHERE owner_user_id = $1 AND ($2 OR deleted_at IS NULL)
                 ORDER BY name",
            )
            .bind(Uuid::parse_str(&principal.user_id).context("parse principal user id")?)
            .bind(include_deleted)
            .fetch_all(&self.pool)
            .await
            .context("list projects")?
        };
        let mut projects = Vec::new();
        for row in rows {
            let mut project = project_from_row(row)?;
            project.repositories = self
                .repositories_for_project(&project.project_id)
                .await
                .context("load project repositories")?;
            project.threads = self
                .threads_for_project(principal, &project.project_id)
                .await
                .context("load project threads")?;
            projects.push(project);
        }
        Ok(projects)
    }

    async fn create_project(
        &self,
        principal: &AuthPrincipal,
        input: CreateProjectInput,
    ) -> Result<ProjectRecord> {
        let user_workspace = self
            .select_user_workspace(principal, input.user_workspace_id)
            .await?;
        let slug = slugify_name(&input.name)?;
        let project_path = PathBuf::from(&user_workspace.path)
            .join("projects")
            .join(&slug);
        tokio::fs::create_dir_all(project_path.join("repos"))
            .await
            .with_context(|| format!("create project path {}", project_path.display()))?;
        tokio::fs::create_dir_all(project_path.join("outputs"))
            .await
            .with_context(|| format!("create project outputs {}", project_path.display()))?;
        let project_path = project_path
            .canonicalize()
            .with_context(|| format!("canonicalize project path {}", project_path.display()))?
            .to_string_lossy()
            .to_string();
        let row = sqlx::query(
            "INSERT INTO enterprise_projects
             (project_id, owner_user_id, user_workspace_id, name, slug, project_path)
             VALUES ($1, $2, $3, $4, $5, $6)
             RETURNING project_id::text, owner_user_id::text, user_workspace_id::text, name, slug, project_path, created_at, updated_at, deleted_at",
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::parse_str(&user_workspace.owner_user_id).context("parse project owner user id")?)
        .bind(Uuid::parse_str(&user_workspace.user_workspace_id).context("parse user workspace id")?)
        .bind(input.name)
        .bind(slug)
        .bind(project_path)
        .fetch_one(&self.pool)
        .await
        .context("insert project")?;
        project_from_row(row)
    }

    async fn update_project(
        &self,
        principal: &AuthPrincipal,
        project_id: &str,
        input: UpdateProjectInput,
    ) -> Result<ProjectRecord> {
        slugify_name(&input.name)?;
        let existing = self.get_project_for_user(principal, project_id).await?;
        let row = sqlx::query(
            "UPDATE enterprise_projects
             SET name = $2, updated_at = now()
             WHERE project_id = $1
             RETURNING project_id::text, owner_user_id::text, user_workspace_id::text, name, slug, project_path, created_at, updated_at, deleted_at",
        )
        .bind(Uuid::parse_str(&existing.project_id).context("parse project id")?)
        .bind(input.name)
        .fetch_one(&self.pool)
        .await
        .context("update project")?;
        let mut project = project_from_row(row)?;
        project.repositories = self.repositories_for_project(&project.project_id).await?;
        project.threads = self
            .threads_for_project(principal, &project.project_id)
            .await?;
        Ok(project)
    }

    async fn delete_project(
        &self,
        principal: &AuthPrincipal,
        project_id: &str,
    ) -> Result<ProjectRecord> {
        let existing = self.get_project_for_user(principal, project_id).await?;
        let row = sqlx::query(
            "UPDATE enterprise_projects
             SET deleted_at = now(), updated_at = now()
             WHERE project_id = $1
             RETURNING project_id::text, owner_user_id::text, user_workspace_id::text, name, slug, project_path, created_at, updated_at, deleted_at",
        )
        .bind(Uuid::parse_str(&existing.project_id).context("parse project id")?)
        .fetch_one(&self.pool)
        .await
        .context("soft delete project")?;
        let mut project = project_from_row(row)?;
        project.repositories = self.repositories_for_project(&project.project_id).await?;
        project.threads = self
            .threads_for_project(principal, &project.project_id)
            .await?;
        Ok(project)
    }

    async fn restore_project(
        &self,
        principal: &AuthPrincipal,
        project_id: &str,
    ) -> Result<ProjectRecord> {
        if !matches!(
            principal.role,
            EnterpriseRole::Admin | EnterpriseRole::Owner | EnterpriseRole::Manager
        ) {
            anyhow::bail!("project not found");
        }
        let row = sqlx::query(
            "UPDATE enterprise_projects
             SET deleted_at = NULL, updated_at = now()
             WHERE project_id = $1
             RETURNING project_id::text, owner_user_id::text, user_workspace_id::text, name, slug, project_path, created_at, updated_at, deleted_at",
        )
        .bind(Uuid::parse_str(project_id).context("parse project id")?)
        .fetch_optional(&self.pool)
        .await
        .context("restore project")?
        .context("project not found")?;
        let mut project = project_from_row(row)?;
        project.repositories = self.repositories_for_project(&project.project_id).await?;
        project.threads = self
            .threads_for_project(principal, &project.project_id)
            .await?;
        Ok(project)
    }

    async fn create_project_thread(
        &self,
        principal: &AuthPrincipal,
        input: CreateProjectThreadInput,
    ) -> Result<SessionRecord> {
        let project = self
            .get_project_for_user(principal, &input.project_id)
            .await?;
        let workspace_path = match input.repository_id.as_ref() {
            Some(repository_id) => {
                self.get_repository_for_project(&project.project_id, repository_id)
                    .await?
                    .repository_path
            }
            None => project.project_path.clone(),
        };
        let session_id = input
            .session_id
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let resolved_workspace_path = self
            .authorize_workspace_path(principal, workspace_path)
            .await?;
        let row = sqlx::query(
            "INSERT INTO enterprise_sessions
             (session_id, owner_user_id, workspace_id, workspace_path, project_id, repository_id, title)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             RETURNING session_id, owner_user_id::text, workspace_id, workspace_path, project_id::text, repository_id::text, title, last_worker_id::text, deleted_at, created_at, updated_at",
        )
        .bind(&session_id)
        .bind(Uuid::parse_str(&principal.user_id).context("parse principal user id")?)
        .bind(&resolved_workspace_path)
        .bind(&resolved_workspace_path)
        .bind(Uuid::parse_str(&project.project_id).context("parse project id")?)
        .bind(input.repository_id.as_deref().map(Uuid::parse_str).transpose().context("parse repository id")?)
        .bind(input.title)
        .fetch_one(&self.pool)
        .await
        .context("insert project thread")?;
        session_from_row(row)
    }

    async fn clone_project_repository(
        &self,
        principal: &AuthPrincipal,
        input: CloneProjectRepositoryInput,
    ) -> Result<RepositoryRecord> {
        let project = self
            .get_project_for_user(principal, &input.project_id)
            .await?;
        let repos_dir = PathBuf::from(&project.project_path).join("repos");
        tokio::fs::create_dir_all(&repos_dir)
            .await
            .with_context(|| format!("create project repositories path {}", repos_dir.display()))?;
        let plan =
            crate::repo_clone::plan_clone(&input.repo_url, &repos_dir, &input.destination_name)?;
        crate::repo_clone::clone_repo(&plan).await?;
        let row = sqlx::query(
            "INSERT INTO enterprise_repositories
             (repository_id, project_id, repo_url, name, repository_path)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING repository_id::text, project_id::text, repo_url, name, repository_path, created_at",
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::parse_str(&project.project_id).context("parse project id")?)
        .bind(crate::repo_clone::redact_repo_url_for_storage(&input.repo_url))
        .bind(input.destination_name)
        .bind(plan.destination_path)
        .fetch_one(&self.pool)
        .await
        .context("insert repository")?;
        repository_from_row(row)
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

    async fn create_context_pack(
        &self,
        _principal: &AuthPrincipal,
        input: CreateContextPackInput,
    ) -> Result<(ContextPackRecord, Vec<ContextDocumentRecord>)> {
        let validated = context_packs::validate_documents(&input.documents)?;
        let pack_id = Uuid::new_v4();
        let mut tx = self.pool.begin().await.context("begin context pack tx")?;
        let row = sqlx::query(
            "INSERT INTO enterprise_context_packs (pack_id, name, status)
             VALUES ($1, $2, 'active')
             RETURNING pack_id::text, name, status, created_at",
        )
        .bind(pack_id)
        .bind(&input.name)
        .fetch_one(&mut *tx)
        .await
        .context("insert context pack")?;
        let pack = context_pack_from_row(row)?;
        let mut documents = Vec::new();
        for document in validated.documents {
            let row = sqlx::query(
                "INSERT INTO enterprise_context_documents
                 (document_id, pack_id, filename, content_hash, load_order, required)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 RETURNING document_id::text, pack_id::text, filename, content_hash, load_order, required",
            )
            .bind(Uuid::new_v4())
            .bind(pack_id)
            .bind(document.filename)
            .bind(document.content_hash)
            .bind(document.load_order)
            .bind(document.required)
            .fetch_one(&mut *tx)
            .await
            .context("insert context document")?;
            documents.push(context_document_from_row(row)?);
        }
        tx.commit().await.context("commit context pack tx")?;
        Ok((pack, documents))
    }

    async fn list_context_packs(
        &self,
        _principal: &AuthPrincipal,
    ) -> Result<Vec<ContextPackRecord>> {
        let rows = sqlx::query(
            "SELECT pack_id::text, name, status, created_at
             FROM enterprise_context_packs
             ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await
        .context("list context packs")?;
        rows.into_iter().map(context_pack_from_row).collect()
    }

    async fn assign_context_pack(
        &self,
        principal: &AuthPrincipal,
        input: AssignContextPackInput,
    ) -> Result<ContextPackAssignmentRecord> {
        let workspace_id = match input.workspace_id {
            Some(workspace) => Some(self.authorize_workspace_path(principal, workspace).await?),
            None => None,
        };
        let pack_id = Uuid::parse_str(&input.pack_id).context("parse context pack id")?;
        let user_id = input
            .user_id
            .as_deref()
            .map(|value| Uuid::parse_str(value).context("parse context assignment user id"))
            .transpose()?;
        let exists = sqlx::query(
            "SELECT 1 FROM enterprise_context_assignments
             WHERE (($1::uuid IS NULL AND user_id IS NULL) OR user_id = $1)
               AND (($2::text IS NULL AND workspace_id IS NULL) OR workspace_id = $2)
               AND assignment_order = $3",
        )
        .bind(user_id)
        .bind(&workspace_id)
        .bind(input.assignment_order)
        .fetch_optional(&self.pool)
        .await
        .context("check context assignment ambiguity")?;
        if exists.is_some() {
            anyhow::bail!("context pack assignment load order is ambiguous");
        }
        let assignment_source = match (&input.user_id, &workspace_id) {
            (Some(_), Some(_)) => "user_workspace",
            (Some(_), None) => "user",
            (None, Some(_)) => "workspace",
            (None, None) => "global",
        };
        let row = sqlx::query(
            "INSERT INTO enterprise_context_assignments
             (assignment_id, pack_id, user_id, workspace_id, assignment_source, assignment_order, required_session, required_worker)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             RETURNING assignment_id::text, pack_id::text, user_id::text, workspace_id, assignment_source, assignment_order, required_session, required_worker",
        )
        .bind(Uuid::new_v4())
        .bind(pack_id)
        .bind(user_id)
        .bind(&workspace_id)
        .bind(assignment_source)
        .bind(input.assignment_order)
        .bind(input.required_session)
        .bind(input.required_worker)
        .fetch_one(&self.pool)
        .await
            .context("insert context assignment")?;
        context_assignment_from_row(row)
    }

    async fn list_context_pack_assignments(
        &self,
        _principal: &AuthPrincipal,
    ) -> Result<Vec<ContextPackAssignmentRecord>> {
        let rows = sqlx::query(
            "SELECT assignment_id::text, pack_id::text, user_id::text, workspace_id, assignment_source, assignment_order, required_session, required_worker
             FROM enterprise_context_assignments
             ORDER BY assignment_order, created_at",
        )
        .fetch_all(&self.pool)
        .await
        .context("list context assignments")?;
        rows.into_iter().map(context_assignment_from_row).collect()
    }

    async fn delete_context_pack_assignment(
        &self,
        _principal: &AuthPrincipal,
        assignment_id: &str,
    ) -> Result<ContextPackAssignmentRecord> {
        let row = sqlx::query(
            "DELETE FROM enterprise_context_assignments
             WHERE assignment_id = $1
             RETURNING assignment_id::text, pack_id::text, user_id::text, workspace_id, assignment_source, assignment_order, required_session, required_worker",
        )
        .bind(Uuid::parse_str(assignment_id).context("parse context assignment id")?)
        .fetch_optional(&self.pool)
        .await
        .context("delete context assignment")?
        .context("context pack assignment not found")?;
        context_assignment_from_row(row)
    }

    async fn record_context_load(
        &self,
        input: ContextLoadInput,
    ) -> Result<Vec<ContextPackReceiptRecord>> {
        let sql = if input.phase == "worker_start" {
            "SELECT a.pack_id::text, a.assignment_source, a.assignment_order,
                    d.document_id::text, d.content_hash, d.load_order
             FROM enterprise_context_assignments a
             JOIN enterprise_context_documents d ON d.pack_id = a.pack_id
             WHERE a.required_worker = true
               AND (a.user_id IS NULL OR a.user_id = $1)
               AND (a.workspace_id IS NULL OR a.workspace_id = $2)
             ORDER BY a.assignment_order, d.load_order"
        } else {
            "SELECT a.pack_id::text, a.assignment_source, a.assignment_order,
                    d.document_id::text, d.content_hash, d.load_order
             FROM enterprise_context_assignments a
             JOIN enterprise_context_documents d ON d.pack_id = a.pack_id
             WHERE a.required_session = true
               AND (a.user_id IS NULL OR a.user_id = $1)
               AND (a.workspace_id IS NULL OR a.workspace_id = $2)
             ORDER BY a.assignment_order, d.load_order"
        };
        let rows = sqlx::query(sql)
            .bind(Uuid::parse_str(&input.actor_user_id).context("parse context actor user id")?)
            .bind(&input.workspace_id)
            .fetch_all(&self.pool)
            .await
            .context("load context assignments")?;

        let mut receipts = Vec::new();
        for row in rows {
            let worker_id = input
                .worker_id
                .as_deref()
                .map(|value| Uuid::parse_str(value).context("parse context worker id"))
                .transpose()?;
            let receipt_id = Uuid::new_v4();
            let pack_id: String = row.try_get("pack_id")?;
            let document_id: String = row.try_get("document_id")?;
            let content_hash: String = row.try_get("content_hash")?;
            let assignment_order: i32 = row.try_get("assignment_order")?;
            let document_order: i32 = row.try_get("load_order")?;
            let assignment_source: String = row.try_get("assignment_source")?;
            let row = sqlx::query(
                "INSERT INTO enterprise_context_receipts
                 (receipt_id, trace_id, pack_id, document_id, content_hash, load_order, assignment_source, actor_user_id, workspace_id, session_id, worker_id, phase)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                 RETURNING receipt_id::text, trace_id::text, pack_id::text, document_id::text, content_hash, load_order, assignment_source, actor_user_id::text, workspace_id, session_id, worker_id::text, phase, created_at",
            )
            .bind(receipt_id)
            .bind(Uuid::parse_str(&input.trace_id).context("parse context trace id")?)
            .bind(Uuid::parse_str(&pack_id).context("parse context pack id")?)
            .bind(Uuid::parse_str(&document_id).context("parse context document id")?)
            .bind(content_hash)
            .bind(assignment_order + document_order)
            .bind(assignment_source)
            .bind(Uuid::parse_str(&input.actor_user_id).context("parse context actor user id")?)
            .bind(&input.workspace_id)
            .bind(&input.session_id)
            .bind(worker_id)
            .bind(&input.phase)
            .fetch_one(&self.pool)
            .await
            .context("insert context receipt")?;
            receipts.push(context_receipt_from_row(row)?);
        }
        Ok(receipts)
    }

    async fn query_evidence(
        &self,
        _principal: &AuthPrincipal,
        query: EvidenceQuery,
    ) -> Result<EvidenceQueryResult> {
        let trace_id = query
            .trace_id
            .as_deref()
            .map(|value| Uuid::parse_str(value).context("parse evidence trace id"))
            .transpose()?;
        let audit_rows = sqlx::query(
            "SELECT trace_id::text, actor_user_id::text, workspace_id, session_id, worker_id::text, event_type, result, metadata_json, created_at
             FROM enterprise_audit_events
             WHERE $1::uuid IS NULL OR trace_id = $1
             ORDER BY created_at DESC",
        )
        .bind(trace_id)
        .fetch_all(&self.pool)
        .await
        .context("query audit events")?;
        let receipt_rows = sqlx::query(
            "SELECT receipt_id::text, execution_id::text, trace_id::text, actor_user_id::text, workspace_id, session_id, worker_id::text, event_type, result, metadata_json, created_at
             FROM enterprise_execution_receipts
             WHERE $1::uuid IS NULL OR trace_id = $1
             ORDER BY created_at DESC",
        )
        .bind(trace_id)
        .fetch_all(&self.pool)
        .await
        .context("query execution receipts")?;
        let context_rows = sqlx::query(
            "SELECT receipt_id::text, trace_id::text, pack_id::text, document_id::text, content_hash, load_order, assignment_source, actor_user_id::text, workspace_id, session_id, worker_id::text, phase, created_at
             FROM enterprise_context_receipts
             WHERE $1::uuid IS NULL OR trace_id = $1
             ORDER BY created_at DESC",
        )
        .bind(trace_id)
        .fetch_all(&self.pool)
        .await
        .context("query context receipts")?;
        Ok(EvidenceQueryResult {
            audit_events: audit_rows
                .into_iter()
                .map(audit_event_from_row)
                .collect::<Result<Vec<_>>>()?,
            execution_receipts: receipt_rows
                .into_iter()
                .map(execution_receipt_from_row)
                .collect::<Result<Vec<_>>>()?,
            context_receipts: context_rows
                .into_iter()
                .map(context_receipt_from_row)
                .collect::<Result<Vec<_>>>()?,
        })
    }

    async fn create_output(&self, input: CreateOutputInput) -> Result<OutputRecord> {
        validate_output_artifact_path(&input.artifact_path)?;
        let owner_user_id =
            Uuid::parse_str(&input.owner_user_id).context("parse output owner user id")?;
        let worker_id = input
            .worker_id
            .as_deref()
            .map(|value| Uuid::parse_str(value).context("parse output worker id"))
            .transpose()?;
        let row = sqlx::query(
            "INSERT INTO enterprise_outputs
             (output_id, owner_user_id, workspace_id, session_id, worker_id, category, output_type, title, artifact_path, status, metadata_json)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
             RETURNING output_id::text, owner_user_id::text, workspace_id, session_id, worker_id::text, category, output_type, title, artifact_path, status, metadata_json, created_at, updated_at",
        )
        .bind(Uuid::new_v4())
        .bind(owner_user_id)
        .bind(input.workspace_id)
        .bind(input.session_id)
        .bind(worker_id)
        .bind(input.category.as_str())
        .bind(input.output_type)
        .bind(input.title)
        .bind(input.artifact_path)
        .bind(input.status)
        .bind(input.metadata_json)
        .fetch_one(&self.pool)
        .await
        .context("insert output metadata")?;
        output_from_row(row)
    }

    async fn list_outputs(&self, principal: &AuthPrincipal) -> Result<Vec<OutputRecord>> {
        let rows = sqlx::query(
            "SELECT output_id::text, owner_user_id::text, workspace_id, session_id, worker_id::text, category, output_type, title, artifact_path, status, metadata_json, created_at, updated_at
             FROM enterprise_outputs
             WHERE owner_user_id = $1
             ORDER BY updated_at DESC",
        )
        .bind(Uuid::parse_str(&principal.user_id).context("parse output owner user id")?)
        .fetch_all(&self.pool)
        .await
        .context("list user output metadata")?;
        rows.into_iter().map(output_from_row).collect()
    }

    async fn get_output(&self, principal: &AuthPrincipal, output_id: &str) -> Result<OutputRecord> {
        let row = sqlx::query(
            "SELECT output_id::text, owner_user_id::text, workspace_id, session_id, worker_id::text, category, output_type, title, artifact_path, status, metadata_json, created_at, updated_at
             FROM enterprise_outputs
             WHERE output_id = $1 AND owner_user_id = $2",
        )
        .bind(Uuid::parse_str(output_id).context("parse output id")?)
        .bind(Uuid::parse_str(&principal.user_id).context("parse output owner user id")?)
        .fetch_optional(&self.pool)
        .await
        .context("get user output metadata")?
        .context("output not found")?;
        output_from_row(row)
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
    async fn admin_workspace_roots(&self) -> Result<Vec<String>> {
        let rows = sqlx::query("SELECT root_path FROM enterprise_workspaces ORDER BY created_at")
            .fetch_all(&self.pool)
            .await
            .context("load workspace allowlist")?;
        rows.into_iter()
            .map(|row| row.try_get("root_path"))
            .collect::<Result<Vec<String>, sqlx::Error>>()
            .context("read workspace allowlist")
    }

    async fn authorize_workspace_path(
        &self,
        principal: &AuthPrincipal,
        requested_path: String,
    ) -> Result<String> {
        let roots = self.list_workspace_roots(principal).await?;
        authorize_workspace(&roots, requested_path)
    }

    async fn ensure_user_workspace_record(
        &self,
        user_id: Uuid,
        workspace_root_id: Option<Uuid>,
        path: &str,
    ) -> Result<String> {
        let row = sqlx::query(
            "INSERT INTO enterprise_user_workspaces
             (user_workspace_id, owner_user_id, workspace_root_id, path)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (owner_user_id, path) DO UPDATE
             SET path = EXCLUDED.path
             RETURNING user_workspace_id::text",
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(workspace_root_id)
        .bind(path)
        .fetch_one(&self.pool)
        .await
        .context("ensure user workspace record")?;
        row.try_get("user_workspace_id")
            .context("read user workspace id")
    }

    async fn select_user_workspace(
        &self,
        principal: &AuthPrincipal,
        requested_id: Option<String>,
    ) -> Result<UserWorkspaceRecord> {
        let principal_user_id =
            Uuid::parse_str(&principal.user_id).context("parse principal user id")?;
        let row = match requested_id {
            Some(requested_id) => {
                let requested_id =
                    Uuid::parse_str(&requested_id).context("parse user workspace id")?;
                let sql = if matches!(
                    principal.role,
                    EnterpriseRole::Admin | EnterpriseRole::Owner | EnterpriseRole::Manager
                ) {
                    "SELECT user_workspace_id::text, owner_user_id::text, workspace_root_id::text, path, created_at
                     FROM enterprise_user_workspaces
                     WHERE user_workspace_id = $1"
                } else {
                    "SELECT user_workspace_id::text, owner_user_id::text, workspace_root_id::text, path, created_at
                     FROM enterprise_user_workspaces
                     WHERE user_workspace_id = $1 AND owner_user_id = $2"
                };
                if matches!(
                    principal.role,
                    EnterpriseRole::Admin | EnterpriseRole::Owner | EnterpriseRole::Manager
                ) {
                    sqlx::query(sql)
                        .bind(requested_id)
                        .fetch_optional(&self.pool)
                        .await
                        .context("select user workspace")?
                } else {
                    sqlx::query(sql)
                        .bind(requested_id)
                        .bind(principal_user_id)
                        .fetch_optional(&self.pool)
                        .await
                        .context("select user workspace")?
                }
            }
            None => {
                sqlx::query(
                    "SELECT user_workspace_id::text, owner_user_id::text, workspace_root_id::text, path, created_at
                     FROM enterprise_user_workspaces
                     WHERE owner_user_id = $1
                     ORDER BY path
                     LIMIT 1",
                )
                .bind(principal_user_id)
                .fetch_optional(&self.pool)
                .await
                .context("select default user workspace")?
            }
        }
        .context("user workspace not found")?;
        user_workspace_from_row(row)
    }

    async fn get_project_for_user(
        &self,
        principal: &AuthPrincipal,
        project_id: &str,
    ) -> Result<ProjectRecord> {
        let project_uuid = Uuid::parse_str(project_id).context("parse project id")?;
        let principal_user_id =
            Uuid::parse_str(&principal.user_id).context("parse principal user id")?;
        let row = if matches!(
            principal.role,
            EnterpriseRole::Admin | EnterpriseRole::Owner | EnterpriseRole::Manager
        ) {
            sqlx::query(
                "SELECT project_id::text, owner_user_id::text, user_workspace_id::text, name, slug, project_path, created_at, updated_at, deleted_at
                 FROM enterprise_projects
                 WHERE project_id = $1 AND deleted_at IS NULL",
            )
            .bind(project_uuid)
            .fetch_optional(&self.pool)
            .await
            .context("get project")?
        } else {
            sqlx::query(
                "SELECT project_id::text, owner_user_id::text, user_workspace_id::text, name, slug, project_path, created_at, updated_at, deleted_at
                 FROM enterprise_projects
                 WHERE project_id = $1 AND owner_user_id = $2 AND deleted_at IS NULL",
            )
            .bind(project_uuid)
            .bind(principal_user_id)
            .fetch_optional(&self.pool)
            .await
            .context("get project")?
        }
        .context("project not found")?;
        project_from_row(row)
    }

    async fn repositories_for_project(&self, project_id: &str) -> Result<Vec<RepositoryRecord>> {
        let rows = sqlx::query(
            "SELECT repository_id::text, project_id::text, repo_url, name, repository_path, created_at
             FROM enterprise_repositories
             WHERE project_id = $1
             ORDER BY name",
        )
        .bind(Uuid::parse_str(project_id).context("parse project id")?)
        .fetch_all(&self.pool)
        .await
        .context("list project repositories")?;
        rows.into_iter().map(repository_from_row).collect()
    }

    async fn get_repository_for_project(
        &self,
        project_id: &str,
        repository_id: &str,
    ) -> Result<RepositoryRecord> {
        let row = sqlx::query(
            "SELECT repository_id::text, project_id::text, repo_url, name, repository_path, created_at
             FROM enterprise_repositories
             WHERE project_id = $1 AND repository_id = $2",
        )
        .bind(Uuid::parse_str(project_id).context("parse project id")?)
        .bind(Uuid::parse_str(repository_id).context("parse repository id")?)
        .fetch_optional(&self.pool)
        .await
        .context("get repository")?
        .context("repository not found")?;
        repository_from_row(row)
    }

    async fn threads_for_project(
        &self,
        principal: &AuthPrincipal,
        project_id: &str,
    ) -> Result<Vec<SessionRecord>> {
        let rows = sqlx::query(
            "SELECT session_id, owner_user_id::text, workspace_id, workspace_path, project_id::text, repository_id::text, title, last_worker_id::text, deleted_at, created_at, updated_at
             FROM enterprise_sessions
             WHERE project_id = $1 AND owner_user_id = $2 AND deleted_at IS NULL
             ORDER BY updated_at DESC",
        )
        .bind(Uuid::parse_str(project_id).context("parse project id")?)
        .bind(Uuid::parse_str(&principal.user_id).context("parse principal user id")?)
        .fetch_all(&self.pool)
        .await
        .context("list project threads")?;
        rows.into_iter().map(session_from_row).collect()
    }

    async fn ensure_admin_role_change_allowed(
        &self,
        principal: &AuthPrincipal,
        user_id: &str,
        next_role: EnterpriseRole,
    ) -> Result<()> {
        if principal.user_id == user_id && next_role != EnterpriseRole::Admin {
            anyhow::bail!("admins cannot remove their own admin role");
        }
        let row = sqlx::query("SELECT role FROM enterprise_users WHERE user_id = $1")
            .bind(Uuid::parse_str(user_id).context("parse user id")?)
            .fetch_optional(&self.pool)
            .await
            .context("load user role for admin guard")?
            .context("user not found")?;
        let role: String = row.try_get("role")?;
        let stored_role = EnterpriseRole::from_storage(&role).context("unknown stored role")?;
        if stored_role != EnterpriseRole::Admin || next_role == EnterpriseRole::Admin {
            return Ok(());
        }
        self.ensure_multiple_active_admins().await
    }

    async fn ensure_admin_status_change_allowed(
        &self,
        principal: &AuthPrincipal,
        user_id: &str,
        next_status: UserStatus,
    ) -> Result<()> {
        if principal.user_id == user_id && next_status != UserStatus::Active {
            anyhow::bail!("admins cannot deactivate themselves");
        }
        let row = sqlx::query("SELECT role FROM enterprise_users WHERE user_id = $1")
            .bind(Uuid::parse_str(user_id).context("parse user id")?)
            .fetch_optional(&self.pool)
            .await
            .context("load user role for admin status guard")?
            .context("user not found")?;
        let role: String = row.try_get("role")?;
        let stored_role = EnterpriseRole::from_storage(&role).context("unknown stored role")?;
        if stored_role != EnterpriseRole::Admin || next_status == UserStatus::Active {
            return Ok(());
        }
        self.ensure_multiple_active_admins().await
    }

    async fn ensure_multiple_active_admins(&self) -> Result<()> {
        let (active_admins,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM enterprise_users WHERE role IN ('admin', 'owner') AND status = 'active'",
        )
        .fetch_one(&self.pool)
        .await
        .context("count active admins")?;
        if active_admins <= 1 {
            anyhow::bail!("cannot remove final active admin");
        }
        Ok(())
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
                .with_context(|| {
                    format!(
                        "workspace root is not accessible from this Enterprise server runtime: {root}"
                    )
                })
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

fn authorized_roots_for_user(state: &MemoryState, principal: &AuthPrincipal) -> Vec<String> {
    match principal.role {
        EnterpriseRole::Admin | EnterpriseRole::Owner => state.workspace_roots.clone(),
        EnterpriseRole::Manager | EnterpriseRole::Developer | EnterpriseRole::Viewer => state
            .workspace_assignments
            .iter()
            .filter(|assignment| assignment.user_id == principal.user_id)
            .map(|assignment| assignment.workspace_root.clone())
            .collect(),
    }
}

fn grantable_roots_for_user(state: &MemoryState, principal: &AuthPrincipal) -> Vec<String> {
    match principal.role {
        EnterpriseRole::Admin | EnterpriseRole::Owner | EnterpriseRole::Manager => {
            state.workspace_roots.clone()
        }
        EnterpriseRole::Developer | EnterpriseRole::Viewer => {
            authorized_roots_for_user(state, principal)
        }
    }
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
        project_id: None,
        repository_id: None,
        title,
        last_worker_id: None,
        deleted_at: None,
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

fn user_from_row(row: sqlx::postgres::PgRow) -> Result<UserRecord> {
    let role: String = row.try_get("role")?;
    let status: String = row.try_get("status")?;
    let role = EnterpriseRole::from_storage(&role)
        .context("unknown stored user role")?
        .as_str()
        .to_string();
    Ok(UserRecord {
        user_id: row.try_get("user_id")?,
        email: row.try_get("email")?,
        role,
        status: UserStatus::from_storage(&status).context("unknown stored user status")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn workspace_from_row(row: sqlx::postgres::PgRow) -> Result<WorkspaceRootRecord> {
    Ok(WorkspaceRootRecord {
        workspace_id: row.try_get("workspace_id")?,
        root_path: row.try_get("root_path")?,
        created_by: row.try_get("created_by")?,
        created_at: row.try_get("created_at")?,
    })
}

fn workspace_assignment_from_row(row: sqlx::postgres::PgRow) -> Result<WorkspaceAssignmentRecord> {
    Ok(WorkspaceAssignmentRecord {
        assignment_id: row.try_get("assignment_id")?,
        user_id: row.try_get("user_id")?,
        workspace_id: row.try_get("workspace_id")?,
        workspace_root: row.try_get("workspace_root")?,
        assigned_by: row.try_get("assigned_by")?,
        created_at: row.try_get("created_at")?,
    })
}

fn user_workspace_from_row(row: sqlx::postgres::PgRow) -> Result<UserWorkspaceRecord> {
    Ok(UserWorkspaceRecord {
        user_workspace_id: row.try_get("user_workspace_id")?,
        owner_user_id: row.try_get("owner_user_id")?,
        workspace_root_id: row.try_get("workspace_root_id")?,
        path: row.try_get("path")?,
        created_at: row.try_get("created_at")?,
    })
}

fn repository_from_row(row: sqlx::postgres::PgRow) -> Result<RepositoryRecord> {
    Ok(RepositoryRecord {
        repository_id: row.try_get("repository_id")?,
        project_id: row.try_get("project_id")?,
        repo_url: row.try_get("repo_url")?,
        name: row.try_get("name")?,
        repository_path: row.try_get("repository_path")?,
        created_at: row.try_get("created_at")?,
    })
}

fn project_from_row(row: sqlx::postgres::PgRow) -> Result<ProjectRecord> {
    Ok(ProjectRecord {
        project_id: row.try_get("project_id")?,
        owner_user_id: row.try_get("owner_user_id")?,
        user_workspace_id: row.try_get("user_workspace_id")?,
        name: row.try_get("name")?,
        slug: row.try_get("slug")?,
        project_path: row.try_get("project_path")?,
        repositories: Vec::new(),
        threads: Vec::new(),
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        deleted_at: row.try_get("deleted_at")?,
    })
}

fn context_pack_from_row(row: sqlx::postgres::PgRow) -> Result<ContextPackRecord> {
    Ok(ContextPackRecord {
        pack_id: row.try_get("pack_id")?,
        name: row.try_get("name")?,
        status: row.try_get("status")?,
        created_at: row.try_get("created_at")?,
    })
}

fn context_document_from_row(row: sqlx::postgres::PgRow) -> Result<ContextDocumentRecord> {
    Ok(ContextDocumentRecord {
        document_id: row.try_get("document_id")?,
        pack_id: row.try_get("pack_id")?,
        filename: row.try_get("filename")?,
        content_hash: row.try_get("content_hash")?,
        load_order: row.try_get("load_order")?,
        required: row.try_get("required")?,
    })
}

fn context_assignment_from_row(row: sqlx::postgres::PgRow) -> Result<ContextPackAssignmentRecord> {
    Ok(ContextPackAssignmentRecord {
        assignment_id: row.try_get("assignment_id")?,
        pack_id: row.try_get("pack_id")?,
        user_id: row.try_get("user_id")?,
        workspace_id: row.try_get("workspace_id")?,
        assignment_source: row.try_get("assignment_source")?,
        assignment_order: row.try_get("assignment_order")?,
        required_session: row.try_get("required_session")?,
        required_worker: row.try_get("required_worker")?,
    })
}

fn context_receipt_from_row(row: sqlx::postgres::PgRow) -> Result<ContextPackReceiptRecord> {
    Ok(ContextPackReceiptRecord {
        receipt_id: row.try_get("receipt_id")?,
        trace_id: row.try_get("trace_id")?,
        pack_id: row.try_get("pack_id")?,
        document_id: row.try_get("document_id")?,
        content_hash: row.try_get("content_hash")?,
        load_order: row.try_get("load_order")?,
        assignment_source: row.try_get("assignment_source")?,
        actor_user_id: row.try_get("actor_user_id")?,
        workspace_id: row.try_get("workspace_id")?,
        session_id: row.try_get("session_id")?,
        worker_id: row.try_get("worker_id")?,
        phase: row.try_get("phase")?,
        created_at: row.try_get("created_at")?,
    })
}

fn audit_event_from_row(row: sqlx::postgres::PgRow) -> Result<AuditEventRecord> {
    let result: String = row.try_get("result")?;
    Ok(AuditEventRecord {
        trace_id: row.try_get("trace_id")?,
        actor_user_id: row.try_get("actor_user_id")?,
        workspace_id: row.try_get("workspace_id")?,
        session_id: row.try_get("session_id")?,
        worker_id: row.try_get("worker_id")?,
        event_type: row.try_get("event_type")?,
        result: TraceResult::from_storage(&result).context("unknown audit result")?,
        metadata_json: row.try_get("metadata_json")?,
        created_at: row.try_get("created_at")?,
    })
}

fn execution_receipt_from_row(row: sqlx::postgres::PgRow) -> Result<ExecutionReceiptRecord> {
    let result: String = row.try_get("result")?;
    Ok(ExecutionReceiptRecord {
        receipt_id: row.try_get("receipt_id")?,
        execution_id: row.try_get("execution_id")?,
        trace_id: row.try_get("trace_id")?,
        actor_user_id: row.try_get("actor_user_id")?,
        workspace_id: row.try_get("workspace_id")?,
        session_id: row.try_get("session_id")?,
        worker_id: row.try_get("worker_id")?,
        event_type: row.try_get("event_type")?,
        result: TraceResult::from_storage(&result).context("unknown receipt result")?,
        metadata_json: row.try_get("metadata_json")?,
        created_at: row.try_get("created_at")?,
    })
}

fn output_from_row(row: sqlx::postgres::PgRow) -> Result<OutputRecord> {
    let category: String = row.try_get("category")?;
    Ok(OutputRecord {
        output_id: row.try_get("output_id")?,
        owner_user_id: row.try_get("owner_user_id")?,
        workspace_id: row.try_get("workspace_id")?,
        session_id: row.try_get("session_id")?,
        worker_id: row.try_get("worker_id")?,
        category: OutputCategory::from_storage(&category).context("unknown output category")?,
        output_type: row.try_get("output_type")?,
        title: row.try_get("title")?,
        artifact_path: row.try_get("artifact_path")?,
        status: row.try_get("status")?,
        metadata_json: row.try_get("metadata_json")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn session_from_row(row: sqlx::postgres::PgRow) -> Result<SessionRecord> {
    Ok(SessionRecord {
        session_id: row.try_get("session_id")?,
        owner_user_id: row.try_get("owner_user_id")?,
        workspace_id: row.try_get("workspace_id")?,
        workspace_path: row.try_get("workspace_path")?,
        project_id: row.try_get("project_id").ok(),
        repository_id: row.try_get("repository_id").ok(),
        title: row.try_get("title")?,
        last_worker_id: row.try_get("last_worker_id")?,
        deleted_at: row.try_get("deleted_at").ok(),
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn session_message_from_row(row: sqlx::postgres::PgRow) -> Result<SessionMessageRecord> {
    Ok(SessionMessageRecord {
        message_id: row.try_get("message_id")?,
        session_id: row.try_get("session_id")?,
        owner_user_id: row.try_get("owner_user_id")?,
        kind: row.try_get("kind")?,
        label: row.try_get("label")?,
        text: row.try_get("text")?,
        sequence: row.try_get("sequence")?,
        feedback_rating: row.try_get("feedback_rating").ok(),
        created_at: row.try_get("created_at")?,
    })
}

fn response_feedback_from_row(row: sqlx::postgres::PgRow) -> Result<ResponseFeedbackRecord> {
    Ok(ResponseFeedbackRecord {
        feedback_id: row.try_get("feedback_id")?,
        owner_user_id: row.try_get("owner_user_id")?,
        session_id: row.try_get("session_id")?,
        message_id: row.try_get("message_id")?,
        rating: row.try_get("rating")?,
        reason_tags: feedback_reason_tags_from_json(row.try_get("reason_tags")?)?,
        comment: row.try_get("comment")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn response_preferences_from_row(
    row: sqlx::postgres::PgRow,
) -> Result<UserResponsePreferencesRecord> {
    Ok(UserResponsePreferencesRecord {
        owner_user_id: row.try_get("owner_user_id")?,
        profile_summary: row.try_get("profile_summary")?,
        positive_tags: feedback_reason_tags_from_json(row.try_get("positive_tags")?)?,
        negative_tags: feedback_reason_tags_from_json(row.try_get("negative_tags")?)?,
        sample_count: row.try_get("sample_count")?,
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

fn user_record_from_memory(state: &MemoryState, user: &AuthPrincipal) -> Result<UserRecord> {
    Ok(UserRecord {
        user_id: user.user_id.clone(),
        email: user.email.clone(),
        role: user.role.as_str().to_string(),
        status: *state
            .user_statuses
            .get(&user.user_id)
            .context("user status not found")?,
        created_at: *state
            .user_created_at
            .get(&user.user_id)
            .context("user created timestamp not found")?,
        updated_at: *state
            .user_updated_at
            .get(&user.user_id)
            .context("user updated timestamp not found")?,
    })
}

fn ensure_admin_role_change_allowed(
    state: &MemoryState,
    principal: &AuthPrincipal,
    user_id: &str,
    next_role: EnterpriseRole,
) -> Result<()> {
    if principal.user_id == user_id && next_role != EnterpriseRole::Admin {
        anyhow::bail!("admins cannot remove their own admin role");
    }
    let Some(user) = state.users.get(user_id) else {
        anyhow::bail!("user not found");
    };
    if user.role != EnterpriseRole::Admin || next_role == EnterpriseRole::Admin {
        return Ok(());
    }
    ensure_multiple_active_admins(state)
}

fn ensure_admin_status_change_allowed(
    state: &MemoryState,
    principal: &AuthPrincipal,
    user_id: &str,
    next_status: UserStatus,
) -> Result<()> {
    if principal.user_id == user_id && next_status != UserStatus::Active {
        anyhow::bail!("admins cannot deactivate themselves");
    }
    let Some(user) = state.users.get(user_id) else {
        anyhow::bail!("user not found");
    };
    if user.role != EnterpriseRole::Admin || next_status == UserStatus::Active {
        return Ok(());
    }
    ensure_multiple_active_admins(state)
}

fn ensure_multiple_active_admins(state: &MemoryState) -> Result<()> {
    let active_admins = state
        .users
        .values()
        .filter(|candidate| {
            candidate.role == EnterpriseRole::Admin
                && state.user_statuses.get(&candidate.user_id) == Some(&UserStatus::Active)
        })
        .count();
    if active_admins <= 1 {
        anyhow::bail!("cannot remove final active admin");
    }
    Ok(())
}

fn assignment_required_for_phase(
    assignment: &ContextPackAssignmentRecord,
    worker_phase: bool,
) -> bool {
    if worker_phase {
        assignment.required_worker
    } else {
        assignment.required_session
    }
}

fn assignment_matches(
    assignment: &ContextPackAssignmentRecord,
    user_id: &str,
    workspace_id: &str,
) -> bool {
    assignment
        .user_id
        .as_ref()
        .is_none_or(|assigned_user| assigned_user == user_id)
        && assignment
            .workspace_id
            .as_ref()
            .is_none_or(|assigned_workspace| assigned_workspace == workspace_id)
}

fn ensure_user_workspace_record(
    state: &mut MemoryState,
    user_id: &str,
    path: &str,
    now: DateTime<Utc>,
) -> String {
    if let Some(existing) = state
        .user_workspaces
        .iter()
        .find(|workspace| workspace.owner_user_id == user_id && workspace.path == path)
    {
        return existing.user_workspace_id.clone();
    }
    let workspace_root_id = state
        .workspaces
        .iter()
        .filter(|workspace| path.starts_with(&workspace.root_path))
        .max_by_key(|workspace| workspace.root_path.len())
        .map(|workspace| workspace.workspace_id.clone());
    let record = UserWorkspaceRecord {
        user_workspace_id: Uuid::new_v4().to_string(),
        owner_user_id: user_id.to_string(),
        workspace_root_id,
        path: path.to_string(),
        created_at: now,
    };
    let id = record.user_workspace_id.clone();
    state.user_workspaces.push(record);
    id
}

fn select_user_workspace(
    state: &MemoryState,
    principal: &AuthPrincipal,
    requested_id: Option<String>,
) -> Result<UserWorkspaceRecord> {
    let mut workspaces = state
        .user_workspaces
        .iter()
        .filter(|workspace| {
            workspace.owner_user_id == principal.user_id
                || matches!(
                    principal.role,
                    EnterpriseRole::Admin | EnterpriseRole::Owner | EnterpriseRole::Manager
                )
        })
        .cloned()
        .collect::<Vec<_>>();
    workspaces.sort_by(|left, right| left.path.cmp(&right.path));
    if let Some(requested_id) = requested_id {
        return workspaces
            .into_iter()
            .find(|workspace| workspace.user_workspace_id == requested_id)
            .context("user workspace not found");
    }
    workspaces
        .into_iter()
        .next()
        .context("no user workspace assigned")
}

fn hydrate_project_record(state: &MemoryState, project: &ProjectRecord) -> ProjectRecord {
    let mut project = project.clone();
    project.repositories = state
        .repositories
        .iter()
        .filter(|repository| repository.project_id == project.project_id)
        .cloned()
        .collect();
    project.threads = state
        .sessions
        .values()
        .filter(|session| {
            session.project_id.as_deref() == Some(project.project_id.as_str())
                && session.deleted_at.is_none()
        })
        .cloned()
        .collect();
    project
        .threads
        .sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    project
}

fn user_can_access_project(
    _state: &MemoryState,
    principal: &AuthPrincipal,
    project: &ProjectRecord,
) -> bool {
    matches!(
        principal.role,
        EnterpriseRole::Admin | EnterpriseRole::Owner | EnterpriseRole::Manager
    ) || project.owner_user_id == principal.user_id
}

fn slugify_name(name: &str) -> Result<String> {
    let slug = name
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else if matches!(ch, '-' | '_') || ch.is_whitespace() {
                '-'
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        anyhow::bail!("project name must contain at least one letter or number");
    }
    Ok(slug)
}

fn validate_output_artifact_path(path: &str) -> Result<()> {
    let trimmed = path.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('/')
        || trimmed.starts_with('~')
        || trimmed.contains('\\')
        || trimmed
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".." || part.starts_with('.'))
    {
        anyhow::bail!("output artifact path must be a relative user-scoped metadata path");
    }
    Ok(())
}
