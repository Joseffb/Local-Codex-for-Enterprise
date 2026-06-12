use crate::auth;
use crate::config::EnterpriseConfig;
use crate::rbac;
use crate::rbac::EnterpriseAction;
use crate::rbac::EnterpriseRole;
use crate::repo_clone;
use crate::storage::AssignContextPackInput;
use crate::storage::BootstrapInput;
use crate::storage::CloneProjectRepositoryInput;
use crate::storage::ContextDocumentRecord;
use crate::storage::ContextLoadInput;
use crate::storage::ContextPackAssignmentRecord;
use crate::storage::ContextPackReceiptRecord;
use crate::storage::ContextPackRecord;
use crate::storage::CreateContextPackFileInput;
use crate::storage::CreateContextPackInput;
use crate::storage::CreateOutputInput;
use crate::storage::CreateProjectInput;
use crate::storage::CreateProjectThreadInput;
use crate::storage::CreateSessionMessageInput;
use crate::storage::CreateThreadReferenceInput;
use crate::storage::CreateUserInput;
use crate::storage::EnterpriseStore;
use crate::storage::EvidenceQuery;
use crate::storage::EvidenceQueryResult;
use crate::storage::EvidenceRecordInput;
use crate::storage::InMemoryEnterpriseStore;
use crate::storage::OutputCategory;
use crate::storage::OutputRecord;
use crate::storage::ProjectRecord;
use crate::storage::RepositoryRecord;
use crate::storage::ResponseFeedbackRecord;
use crate::storage::SessionMessageRecord;
use crate::storage::SessionRecord;
use crate::storage::ThreadReferenceRecord;
use crate::storage::UpdateContextPackFileInput;
use crate::storage::UpdateProjectInput;
use crate::storage::UpdateSessionTitleInput;
use crate::storage::UpdateThreadReferenceInput;
use crate::storage::UpsertResponseFeedbackInput;
use crate::storage::UserRecord;
use crate::storage::UserResponsePreferencesRecord;
use crate::storage::UserStatus;
use crate::storage::UserWorkspaceRecord;
use crate::storage::WorkspaceAssignmentRecord;
use crate::storage::WorkspaceRootRecord;
use crate::trace;
use crate::trace::EvidenceRecordContext;
use crate::trace::TraceContext;
use crate::trace::TraceResult;
use crate::worker::WorkerRecord;
use crate::worker::WorkerRuntimeSupervisor;
use crate::worker::WorkerState;
use anyhow::Context;
use axum::Json;
use axum::Router;
use axum::extract::Extension;
use axum::extract::Path;
use axum::extract::Query;
use axum::extract::State;
use axum::extract::ws::Message as AxumWsMessage;
use axum::extract::ws::WebSocket;
use axum::extract::ws::WebSocketUpgrade;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::http::Uri;
use axum::middleware::from_fn;
use axum::response::Html;
use axum::response::IntoResponse;
use axum::response::Redirect;
use axum::response::Response;
use axum::routing::get;
use axum::routing::patch;
use axum::routing::post;
use axum::routing::put;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use chrono::DateTime;
use chrono::Utc;
use codex_uds::UnixStream;
use futures::SinkExt;
use futures::StreamExt;
use serde::Deserialize;
use serde::Serialize;
use std::path::Path as FsPath;
use std::time::Duration;
use tokio_tungstenite::client_async;
use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;
use utoipa::OpenApi;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    pub product: &'static str,
    pub status: &'static str,
}

#[derive(Debug, Clone)]
pub struct AppState<S> {
    store: S,
    config: EnterpriseConfig,
    worker_runtime: WorkerRuntimeSupervisor,
}

impl<S> AppState<S> {
    pub fn new(
        store: S,
        config: EnterpriseConfig,
        worker_runtime: WorkerRuntimeSupervisor,
    ) -> Self {
        Self {
            store,
            config,
            worker_runtime,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ConfigResponse {
    pub product: &'static str,
    pub mode: &'static str,
    pub default_model_provider: String,
    pub default_model: String,
    pub turn_guidance: TurnGuidanceResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TurnGuidanceResponse {
    pub repository_tool_rule: String,
    pub tool_output_rule: String,
    pub planning_sequence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct EnterpriseSetupRequest {
    pub owner_email: String,
    pub owner_password: String,
    pub workspace_roots: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct EnterpriseSetupResponse {
    pub owner_user_id: String,
    pub owner_email: String,
    pub api_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct LoginResponse {
    pub user_id: String,
    pub email: String,
    pub role: String,
    pub api_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct BrowserLoginResponse {
    pub user_id: String,
    pub email: String,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CreateUserRequest {
    pub email: String,
    pub password: String,
    pub role: String,
    pub workspace_roots: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AssignUserRoleRequest {
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UserResponse {
    pub user: UserRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UsersResponse {
    pub users: Vec<UserRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RegisterWorkspaceRequest {
    pub root_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ValidateWorkspaceRequest {
    pub workspace_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct WorkspaceResponse {
    pub workspace: WorkspaceRootRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct WorkspacesResponse {
    pub workspaces: Vec<WorkspaceRootRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UserWorkspacesResponse {
    pub user_workspaces: Vec<UserWorkspaceRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CreateProjectRequest {
    pub name: String,
    pub user_workspace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UpdateProjectRequest {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProjectResponse {
    pub project: ProjectRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProjectsResponse {
    pub projects: Vec<ProjectRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProjectQuery {
    pub user_id: Option<String>,
    pub include_deleted: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CreateProjectThreadRequest {
    pub session_id: Option<String>,
    pub repository_id: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UpdateThreadRequest {
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CloneProjectRepositoryRequest {
    pub repo_url: String,
    pub destination_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RepositoryResponse {
    pub repository: RepositoryRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AssignWorkspaceRequest {
    pub user_id: String,
    pub workspace_root: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct WorkspaceAssignmentResponse {
    pub assignment: WorkspaceAssignmentRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ValidateWorkspaceResponse {
    pub workspace_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CreateContextPackRequest {
    pub name: String,
    pub documents: Vec<crate::context_packs::ContextPackDocumentInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ContextPackResponse {
    pub pack: ContextPackRecord,
    pub documents: Vec<ContextDocumentRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ContextPacksResponse {
    pub packs: Vec<ContextPackRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CreateContextPackFileRequest {
    pub relative_path: String,
    pub content_base64: String,
    pub content_type: Option<String>,
    pub load_order: Option<i32>,
    pub required: Option<bool>,
    pub file_kind: Option<String>,
    pub loadable: Option<bool>,
    pub source_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UpdateContextPackFileRequest {
    pub relative_path: Option<String>,
    pub content_type: Option<String>,
    pub load_order: Option<i32>,
    pub required: Option<bool>,
    pub file_kind: Option<String>,
    pub loadable: Option<bool>,
    pub source_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ContextPackFileResponse {
    pub file: ContextDocumentRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ContextPackFilesResponse {
    pub files: Vec<ContextDocumentRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AssignContextPackRequest {
    pub pack_id: String,
    pub user_id: Option<String>,
    pub workspace_id: Option<String>,
    pub assignment_order: i32,
    pub required_session: bool,
    pub required_worker: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ContextPackAssignmentResponse {
    pub assignment: ContextPackAssignmentRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ContextPackAssignmentsResponse {
    pub assignments: Vec<ContextPackAssignmentRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct BulkAssignContextPacksRequest {
    pub pack_ids: Vec<String>,
    pub user_ids: Vec<String>,
    pub workspace_ids: Vec<String>,
    pub assignment_order: i32,
    pub required_session: bool,
    pub required_worker: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct BulkContextPackAssignmentsResponse {
    pub assignments: Vec<ContextPackAssignmentRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AuditResponse {
    pub audit_events: Vec<crate::storage::AuditEventRecord>,
    pub execution_receipts: Vec<crate::storage::ExecutionReceiptRecord>,
    pub context_receipts: Vec<ContextPackReceiptRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AuditQuery {
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DemoDataStatusResponse {
    pub installed: bool,
    pub demo_user_count: usize,
    pub demo_context_pack_count: usize,
    pub demo_assignment_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DemoUserSeedRecord {
    pub user_id: String,
    pub email: String,
    pub role: String,
    pub created: bool,
    pub password: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DemoDataSeedResponse {
    pub installed: bool,
    pub users: Vec<DemoUserSeedRecord>,
    pub context_pack: ContextPackRecord,
    pub documents: Vec<ContextDocumentRecord>,
    pub document_filenames: Vec<String>,
    pub assignments: Vec<ContextPackAssignmentRecord>,
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct StartWorkerRequest {
    pub workspace_path: String,
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CreateSessionRequest {
    pub session_id: Option<String>,
    pub workspace_path: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SessionResponse {
    pub session: SessionRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SessionsResponse {
    pub sessions: Vec<SessionRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CreateSessionMessageRequest {
    pub kind: String,
    pub label: String,
    pub text: String,
    pub retry_of_message_id: Option<String>,
    pub supersedes_message_id: Option<String>,
    pub context_cutoff_message_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SessionMessageResponse {
    pub message: SessionMessageRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SessionMessagesResponse {
    pub messages: Vec<SessionMessageRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ResponseFeedbackRequest {
    pub rating: String,
    #[serde(default)]
    pub reason_tags: Vec<String>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ResponseFeedbackResponse {
    pub feedback: ResponseFeedbackRecord,
    pub preferences: UserResponsePreferencesRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ResponsePreferencesResponse {
    pub preferences: UserResponsePreferencesRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct WorkerResponse {
    pub worker: WorkerRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct WorkersResponse {
    pub workers: Vec<WorkerRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct WorkerHandoffResponse {
    pub handoff_token: String,
    pub jti: String,
    pub owner_user_id: String,
    pub worker_id: String,
    pub workspace_id: String,
    pub session_id: String,
    pub socket_path: String,
    #[schema(value_type = String)]
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ConsumeWorkerHandoffRequest {
    pub handoff_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ConsumeWorkerHandoffResponse {
    pub jti: String,
    pub owner_user_id: String,
    pub worker_id: String,
    pub workspace_id: String,
    pub session_id: String,
    pub socket_path: String,
    #[schema(value_type = String)]
    pub consumed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CreateOutputRequest {
    pub owner_user_id: Option<String>,
    pub workspace_id: Option<String>,
    pub session_id: Option<String>,
    pub worker_id: Option<String>,
    pub category: String,
    pub output_type: String,
    pub title: String,
    pub artifact_path: String,
    pub status: String,
    pub metadata_json: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct OutputResponse {
    pub output: OutputRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct OutputsResponse {
    pub outputs: Vec<OutputRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SaveMessageOutputRequest {
    pub title: String,
    pub category: Option<String>,
    pub output_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ThreadReferenceResponse {
    pub reference: ThreadReferenceRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ThreadReferencesResponse {
    pub references: Vec<ThreadReferenceRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ThreadReferenceOutputResponse {
    pub reference: ThreadReferenceRecord,
    pub output: OutputRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, ToSchema)]
pub struct CreateThreadReferenceRequest {
    pub source_thread_id: String,
    pub reference_type: String,
    pub max_chars: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, ToSchema)]
pub struct UpdateThreadReferenceRequest {
    pub status: String,
    pub output_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, ToSchema)]
pub struct CreateThreadArtifactRequest {
    pub target_thread_id: Option<String>,
    pub title: Option<String>,
    pub max_chars: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, ToSchema)]
pub struct ImportThreadArtifactRequest {
    pub source_output_id: String,
    pub max_chars: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ThreadSummaryPromptResponse {
    pub reference: ThreadReferenceRecord,
    pub summary_prompt: String,
    pub source_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ArtifactImportResponse {
    pub reference: ThreadReferenceRecord,
    pub excerpt: String,
    pub excerpt_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WorkerRpcQuery {
    pub handoff_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn internal(error: anyhow::Error) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
    }

    fn storage(error: anyhow::Error) -> Self {
        let message = error.to_string();
        if message.contains("workspace path is not allowlisted") {
            return Self::new(StatusCode::FORBIDDEN, message);
        }
        if message.contains("workspace root is not allowlisted")
            || message.contains("repo URL host is not allowed")
        {
            return Self::new(StatusCode::FORBIDDEN, message);
        }
        if message.contains("canonicalize workspace")
            || message
                .contains("workspace root is not accessible from this Enterprise server runtime")
        {
            return Self::new(StatusCode::BAD_REQUEST, message);
        }
        if message.contains("repo URL")
            || message.contains("repo clone destination")
            || message.contains("parse repo")
            || message.contains("canonicalize workspace root")
        {
            return Self::new(StatusCode::BAD_REQUEST, message);
        }
        if message.contains("worker not found") {
            return Self::new(StatusCode::NOT_FOUND, message);
        }
        if message.contains("project not found") || message.contains("repository not found") {
            return Self::new(StatusCode::NOT_FOUND, message);
        }
        if message.contains("session not found") {
            return Self::new(StatusCode::NOT_FOUND, message);
        }
        if message.contains("feedback message not found") {
            return Self::new(StatusCode::NOT_FOUND, message);
        }
        if message.contains("feedback target must be an assistant message") {
            return Self::new(StatusCode::BAD_REQUEST, message);
        }
        if message.contains("context pack assignment not found") {
            return Self::new(StatusCode::NOT_FOUND, message);
        }
        if message.contains("session already exists")
            || message.contains("session workspace does not match worker workspace")
            || message.contains("workspace root already registered")
            || message.contains("user email already exists")
            || message.contains("context pack assignment load order is ambiguous")
            || message.contains("cannot remove final active admin")
            || message.contains("admins cannot remove their own admin role")
            || message.contains("admins cannot deactivate themselves")
            || message.contains("workspace assignment already exists")
        {
            return Self::new(StatusCode::CONFLICT, message);
        }
        if message.contains("workspace assignment user not found") {
            return Self::new(StatusCode::NOT_FOUND, message);
        }
        if message.contains("worker is not running")
            || message.contains("worker socket is not available")
            || message.contains("worker handoff already consumed")
        {
            return Self::new(StatusCode::CONFLICT, message);
        }
        if message.contains("worker handoff expired")
            || message.contains("worker handoff not found")
            || message.contains("worker handoff claims do not match record")
        {
            return Self::new(StatusCode::UNAUTHORIZED, message);
        }
        if message.contains("output artifact path") {
            return Self::new(StatusCode::BAD_REQUEST, message);
        }
        if message.contains("output owner user not found") || message.contains("output not found") {
            return Self::new(StatusCode::NOT_FOUND, message);
        }
        if message.contains("context pack file not found")
            || message.contains("context pack not found")
        {
            return Self::new(StatusCode::NOT_FOUND, message);
        }
        if message.contains("context pack file path already exists") {
            return Self::new(StatusCode::CONFLICT, message);
        }
        if message.contains("context pack file exceeds 10 MB limit") {
            return Self::new(StatusCode::PAYLOAD_TOO_LARGE, message);
        }
        if message.contains("context pack file path is not allowed")
            || message.contains("context pack file kind is not allowed")
            || message.contains("context pack file source type is not allowed")
        {
            return Self::new(StatusCode::BAD_REQUEST, message);
        }
        Self::internal(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(ErrorResponse {
                error: self.message,
            }),
        )
            .into_response()
    }
}

pub fn health_response() -> HealthResponse {
    HealthResponse {
        product: "Local Codex for Enterprise",
        status: "ok",
    }
}

fn turn_guidance_response() -> TurnGuidanceResponse {
    TurnGuidanceResponse {
        repository_tool_rule: "Before using repository tools, decide whether the user request is actually about the current codebase. If the request is business planning, architecture advice, writing, strategy, or general analysis, answer directly and do not inspect the repository unless the user explicitly asks.".to_string(),
        tool_output_rule: "When using web or documentation tools, summarize findings. Do not print raw HTML, JSON, or full tool output unless explicitly requested.".to_string(),
        planning_sequence: vec![
            "business goal".to_string(),
            "users/stakeholders".to_string(),
            "decisions the system must support".to_string(),
            "data sources".to_string(),
            "architecture".to_string(),
            "implementation path".to_string(),
        ],
    }
}

#[utoipa::path(
    get,
    path = "/healthz",
    responses((status = 200, description = "Enterprise server is healthy", body = HealthResponse))
)]
async fn healthz() -> Json<HealthResponse> {
    Json(health_response())
}

#[derive(OpenApi)]
#[openapi(
    paths(healthz, get_config, setup_enterprise::<InMemoryEnterpriseStore>, login::<InMemoryEnterpriseStore>, list_sessions::<InMemoryEnterpriseStore>, get_session::<InMemoryEnterpriseStore>, list_workers::<InMemoryEnterpriseStore>, start_worker::<InMemoryEnterpriseStore>, stop_worker::<InMemoryEnterpriseStore>, issue_worker_handoff::<InMemoryEnterpriseStore>, consume_worker_handoff::<InMemoryEnterpriseStore>, worker_rpc::<InMemoryEnterpriseStore>),
    components(schemas(
        CloneProjectRepositoryRequest,
        ConsumeWorkerHandoffRequest,
        ConsumeWorkerHandoffResponse,
        ConfigResponse,
        ContextPackFileResponse,
        ContextPackFilesResponse,
        CreateContextPackFileRequest,
        CreateProjectThreadRequest,
        CreateSessionMessageRequest,
        EnterpriseSetupRequest,
        EnterpriseSetupResponse,
        ErrorResponse,
        HealthResponse,
        LoginRequest,
        LoginResponse,
        ResponseFeedbackRequest,
        ResponseFeedbackResponse,
        ResponsePreferencesResponse,
        SessionResponse,
        SessionMessageResponse,
        SessionMessagesResponse,
        SessionsResponse,
        StartWorkerRequest,
        UpdateContextPackFileRequest,
        UpdateThreadRequest,
        WorkerHandoffResponse,
        WorkerResponse,
        WorkersResponse
    ))
)]
struct EnterpriseApi;

pub fn openapi_document() -> utoipa::openapi::OpenApi {
    EnterpriseApi::openapi()
}

pub fn build_router() -> Router {
    build_test_router()
}

pub fn build_test_router() -> Router {
    build_router_with_store(
        InMemoryEnterpriseStore::default(),
        EnterpriseConfig::default(),
    )
}

pub fn build_router_with_store<S>(store: S, config: EnterpriseConfig) -> Router
where
    S: EnterpriseStore,
{
    Router::new()
        .route("/", get(page::<S>))
        .route("/setup", get(page::<S>))
        .route("/login", get(page::<S>))
        .route("/admin", get(page::<S>))
        .route("/admin/users", get(page::<S>))
        .route("/admin/users/new", get(page::<S>))
        .route("/admin/users/status", get(page::<S>))
        .route("/admin/users/projects", get(page::<S>))
        .route("/admin/users/context-packs", get(page::<S>))
        .route("/admin/rbac", get(page::<S>))
        .route("/admin/workspaces", get(page::<S>))
        .route("/admin/workspaces/register", get(page::<S>))
        .route("/admin/workspaces/validate", get(page::<S>))
        .route("/admin/context-packs", get(page::<S>))
        .route("/admin/context-packs/new", get(page::<S>))
        .route("/admin/context-packs/import", get(page::<S>))
        .route("/admin/context-packs/files", get(page::<S>))
        .route("/admin/context-packs/assignments", get(page::<S>))
        .route("/admin/outputs", get(page::<S>))
        .route("/chat", get(page::<S>))
        .route("/app", get(page::<S>))
        .route("/app/terminal", get(page::<S>))
        .route("/app/outputs", get(page::<S>))
        .route("/app/sessions/{session_id}", get(page::<S>))
        .route("/admin/audit", get(page::<S>))
        .route("/healthz", get(healthz))
        .route("/v1/config", get(get_config::<S>))
        .route("/v1/setup/enterprise", post(setup_enterprise::<S>))
        .route("/v1/auth/login", post(login::<S>))
        .route("/v1/auth/browser-login", post(browser_login::<S>))
        .route("/v1/auth/browser-logout", post(browser_logout))
        .route("/v1/auth/me", get(current_user::<S>))
        .route("/v1/user-workspaces", get(list_user_workspaces::<S>))
        .route(
            "/v1/user-workspaces/{user_workspace_id}/projects",
            get(list_user_workspace_projects::<S>).post(create_user_workspace_project::<S>),
        )
        .route(
            "/v1/projects",
            get(list_projects::<S>).post(create_project::<S>),
        )
        .route(
            "/v1/projects/{project_id}",
            get(get_project::<S>)
                .patch(update_project::<S>)
                .delete(delete_project::<S>),
        )
        .route(
            "/v1/projects/{project_id}/restorations",
            post(restore_project::<S>),
        )
        .route(
            "/v1/projects/{project_id}/threads",
            get(list_project_threads::<S>).post(create_project_thread::<S>),
        )
        .route(
            "/v1/threads/{thread_id}",
            get(get_session::<S>)
                .patch(update_thread::<S>)
                .delete(delete_thread::<S>),
        )
        .route(
            "/v1/threads/{thread_id}/messages",
            get(list_session_messages::<S>).post(create_session_message::<S>),
        )
        .route(
            "/v1/threads/{thread_id}/messages/{message_id}/feedback",
            put(upsert_response_feedback::<S>),
        )
        .route(
            "/v1/threads/{thread_id}/messages/{message_id}/outputs",
            post(save_message_output::<S>),
        )
        .route(
            "/v1/threads/{thread_id}/references",
            get(list_thread_references::<S>).post(create_thread_reference::<S>),
        )
        .route(
            "/v1/thread-references/{reference_id}",
            patch(update_thread_reference::<S>),
        )
        .route(
            "/v1/threads/{thread_id}/exports",
            post(export_thread_transcript::<S>),
        )
        .route(
            "/v1/threads/{thread_id}/handoffs",
            post(create_thread_handoff::<S>),
        )
        .route(
            "/v1/threads/{thread_id}/artifact-imports",
            post(import_thread_artifact::<S>),
        )
        .route(
            "/v1/me/response-preferences",
            get(get_response_preferences::<S>).delete(reset_response_preferences::<S>),
        )
        .route(
            "/v1/projects/{project_id}/repositories/clone",
            post(clone_project_repository::<S>),
        )
        .route(
            "/v1/demo-data",
            get(get_demo_data_status::<S>).post(seed_demo_data::<S>),
        )
        .route("/v1/users", get(list_users::<S>).post(create_user::<S>))
        .route(
            "/v1/users/{user_id}/deactivations",
            post(deactivate_user::<S>),
        )
        .route(
            "/v1/users/{user_id}/reactivations",
            post(reactivate_user::<S>),
        )
        .route("/v1/users/{user_id}/role", put(assign_user_role::<S>))
        .route(
            "/v1/workspace-roots",
            get(list_workspaces::<S>).post(register_workspace::<S>),
        )
        .route(
            "/v1/workspace-path-validations",
            post(validate_workspace::<S>),
        )
        .route(
            "/v1/user-workspace-access-grants",
            post(assign_workspace::<S>),
        )
        .route(
            "/v1/context-packs",
            get(list_context_packs::<S>).post(create_context_pack::<S>),
        )
        .route(
            "/v1/context-packs/{pack_id}/files",
            get(list_context_pack_files::<S>).post(create_context_pack_file::<S>),
        )
        .route(
            "/v1/context-packs/{pack_id}/files/{document_id}",
            axum::routing::patch(update_context_pack_file::<S>)
                .delete(delete_context_pack_file::<S>),
        )
        .route(
            "/v1/context-packs/{pack_id}/files/{document_id}/download",
            get(download_context_pack_file::<S>),
        )
        .route(
            "/v1/context-pack-assignments",
            get(list_context_pack_assignments::<S>).post(assign_context_pack::<S>),
        )
        .route(
            "/v1/context-pack-assignment-batches",
            post(bulk_assign_context_packs::<S>),
        )
        .route(
            "/v1/context-pack-assignments/{assignment_id}",
            axum::routing::delete(delete_context_pack_assignment::<S>),
        )
        .route("/v1/evidence-records", get(query_audit::<S>))
        .route(
            "/v1/outputs",
            get(list_outputs::<S>).post(create_output::<S>),
        )
        .route(
            "/v1/outputs/{output_id}/download",
            get(download_output::<S>),
        )
        .route(
            "/v1/threads",
            get(list_sessions::<S>).post(create_session::<S>),
        )
        .route(
            "/v1/workers",
            get(list_workers::<S>).post(start_worker::<S>),
        )
        .route(
            "/v1/workers/{worker_id}",
            axum::routing::delete(stop_worker::<S>),
        )
        .route(
            "/v1/workers/{worker_id}/handoffs",
            post(issue_worker_handoff::<S>),
        )
        .route(
            "/v1/threads/{thread_id}/workers",
            post(start_thread_worker::<S>),
        )
        .route("/v1/workers/{worker_id}/rpc", get(worker_rpc::<S>))
        .route(
            "/v1/worker-handoffs/{jti}/consumptions",
            post(consume_worker_handoff::<S>),
        )
        .layer(from_fn(trace::trace_middleware))
        .with_state(AppState::new(
            store,
            config,
            WorkerRuntimeSupervisor::default(),
        ))
}

#[utoipa::path(
    get,
    path = "/v1/config",
    responses((status = 200, description = "Enterprise server runtime config", body = ConfigResponse))
)]
async fn get_config<S>(State(state): State<AppState<S>>) -> Json<ConfigResponse>
where
    S: EnterpriseStore,
{
    Json(ConfigResponse {
        product: "Local Codex for Enterprise",
        mode: "enterprise",
        default_model_provider: state.config.default_model_provider,
        default_model: state.config.default_model,
        turn_guidance: turn_guidance_response(),
    })
}

#[utoipa::path(
    post,
    path = "/v1/setup/enterprise",
    request_body = EnterpriseSetupRequest,
    responses(
        (status = 201, description = "Enterprise server bootstrapped", body = EnterpriseSetupResponse),
        (status = 409, description = "Enterprise server is already bootstrapped", body = ErrorResponse)
    )
)]
async fn setup_enterprise<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    Json(request): Json<EnterpriseSetupRequest>,
) -> Result<(StatusCode, Json<EnterpriseSetupResponse>), ApiError>
where
    S: EnterpriseStore,
{
    if state
        .store
        .is_bootstrapped()
        .await
        .map_err(ApiError::internal)?
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "enterprise server is already bootstrapped",
        ));
    }

    let password_hash = auth::hash_password(&request.owner_password).map_err(ApiError::internal)?;
    let issued_token = auth::issue_api_token("admin").map_err(ApiError::internal)?;
    let owner_email = request.owner_email.clone();
    let owner_workspace_base = request
        .workspace_roots
        .first()
        .cloned()
        .or_else(|| state.config.default_workspace_root.clone());
    let outcome = state
        .store
        .bootstrap_enterprise(BootstrapInput {
            owner_email: request.owner_email,
            owner_password_hash: password_hash,
            workspace_roots: request.workspace_roots,
            issued_token_hash: issued_token.token_hash,
        })
        .await
        .map_err(ApiError::storage)?;
    if let Some(owner_workspace_base) = owner_workspace_base {
        let owner_workspace =
            default_user_workspace_root_at_base(&owner_workspace_base, &owner_email)
                .await
                .map_err(ApiError::storage)?;
        let owner_principal = crate::storage::AuthPrincipal {
            user_id: outcome.owner_user_id.clone(),
            email: outcome.owner_email.clone(),
            role: EnterpriseRole::Admin,
        };
        state
            .store
            .assign_workspace_to_user(&owner_principal, &outcome.owner_user_id, owner_workspace)
            .await
            .map_err(ApiError::storage)?;
    }
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace).actor(outcome.owner_user_id.clone()),
        "enterprise.bootstrap",
        TraceResult::Completed,
        serde_json::json!({ "owner_email": outcome.owner_email.clone() }),
    )
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(EnterpriseSetupResponse {
            owner_user_id: outcome.owner_user_id,
            owner_email: outcome.owner_email,
            api_token: issued_token.plaintext,
        }),
    ))
}

#[utoipa::path(
    post,
    path = "/v1/auth/login",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "User authenticated and API token issued", body = LoginResponse),
        (status = 401, description = "Invalid email or password", body = ErrorResponse)
    )
)]
async fn login<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = state
        .store
        .authenticate_password(&request.email, &request.password)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "invalid email or password"));
    let principal = match principal {
        Ok(principal) => principal,
        Err(error) => {
            record_audit(
                &state,
                EvidenceRecordContext::new(&trace),
                "auth.login.failure",
                TraceResult::Denied,
                serde_json::json!({ "email": request.email }),
            )
            .await?;
            return Err(error);
        }
    };
    let issued_token = auth::issue_api_token("login").map_err(ApiError::internal)?;
    state
        .store
        .create_api_token(&principal, "login", issued_token.token_hash)
        .await
        .map_err(ApiError::internal)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace).actor(principal.user_id.clone()),
        "auth.login.success",
        TraceResult::Completed,
        serde_json::json!({ "email": principal.email.clone() }),
    )
    .await?;

    Ok(Json(LoginResponse {
        user_id: principal.user_id,
        email: principal.email,
        role: principal.role.as_str().to_string(),
        api_token: issued_token.plaintext,
    }))
}

async fn browser_login<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    Json(request): Json<LoginRequest>,
) -> Result<impl IntoResponse, ApiError>
where
    S: EnterpriseStore,
{
    let principal = state
        .store
        .authenticate_password(&request.email, &request.password)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "invalid email or password"));
    let principal = match principal {
        Ok(principal) => principal,
        Err(error) => {
            record_audit(
                &state,
                EvidenceRecordContext::new(&trace),
                "auth.browser_login.failure",
                TraceResult::Denied,
                serde_json::json!({ "email": request.email }),
            )
            .await?;
            return Err(error);
        }
    };
    let issued_token = auth::issue_api_token("browser").map_err(ApiError::internal)?;
    state
        .store
        .create_api_token(&principal, "browser", issued_token.token_hash)
        .await
        .map_err(ApiError::internal)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace).actor(principal.user_id.clone()),
        "auth.browser_login.success",
        TraceResult::Completed,
        serde_json::json!({ "email": principal.email.clone() }),
    )
    .await?;
    let cookie = format!(
        "lce_api_token={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=28800",
        issued_token.plaintext
    );
    Ok((
        [(axum::http::header::SET_COOKIE, cookie)],
        Json(BrowserLoginResponse {
            user_id: principal.user_id,
            email: principal.email,
            role: principal.role.as_str().to_string(),
        }),
    ))
}

async fn browser_logout() -> impl IntoResponse {
    (
        [(
            axum::http::header::SET_COOKIE,
            "lce_api_token=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0".to_string(),
        )],
        Json(serde_json::json!({ "status": "logged_out" })),
    )
}

async fn current_user<S>(
    State(state): State<AppState<S>>,
    headers: HeaderMap,
) -> Result<Json<BrowserLoginResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    Ok(Json(BrowserLoginResponse {
        user_id: principal.user_id,
        email: principal.email,
        role: principal.role.as_str().to_string(),
    }))
}

async fn get_demo_data_status<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
) -> Result<Json<DemoDataStatusResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::AdministerUsers,
    )
    .await?;
    Ok(Json(demo_data_status(&state, &principal).await?))
}

async fn seed_demo_data<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<DemoDataSeedResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::AdministerUsers,
    )
    .await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::ManageContextPacks,
    )
    .await?;

    let before = demo_data_status(&state, &principal).await?;
    let mut seeded_users = Vec::new();
    for (email, role) in [
        ("demo.developer@example.test", EnterpriseRole::Developer),
        ("demo.viewer@example.test", EnterpriseRole::Viewer),
    ] {
        seeded_users.push(ensure_demo_user(&state, &principal, email, role).await?);
    }

    let (context_pack, documents) = ensure_demo_context_pack(&state, &principal).await?;
    let workspace_id = state
        .store
        .list_workspace_roots(&principal)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .next();
    let assignments = if let Some(workspace_id) = workspace_id.clone() {
        ensure_demo_assignments(
            &state,
            &principal,
            &context_pack.pack_id,
            &seeded_users,
            &workspace_id,
        )
        .await?
    } else {
        Vec::new()
    };

    record_audit(
        &state,
        EvidenceRecordContext::new(&trace).actor(principal.user_id.clone()),
        "demo_data.seed",
        TraceResult::Completed,
        serde_json::json!({
            "created_user_count": seeded_users.iter().filter(|user| user.created).count(),
            "context_pack_id": context_pack.pack_id,
            "document_count": documents.len(),
            "assignment_count": assignments.len(),
            "workspace_id": workspace_id,
        }),
    )
    .await?;

    let status = if before.installed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((
        status,
        Json(DemoDataSeedResponse {
            installed: true,
            users: seeded_users,
            context_pack,
            documents,
            document_filenames: demo_context_pack_documents()
                .into_iter()
                .map(|document| document.filename)
                .collect(),
            assignments,
            workspace_id,
        }),
    ))
}

async fn create_user<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Json(request): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<UserResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::AdministerUsers,
    )
    .await?;
    let role = parse_role(&request.role)?;
    let password_hash = auth::hash_password(&request.password).map_err(ApiError::internal)?;
    let workspace_roots = user_workspace_roots_from_request(&state.config, &request)
        .await
        .map_err(ApiError::storage)?;
    let user = state
        .store
        .create_user(
            &principal,
            CreateUserInput {
                email: request.email,
                password_hash,
                role,
                workspace_roots,
            },
        )
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace).actor(principal.user_id.clone()),
        "user.create",
        TraceResult::Completed,
        serde_json::json!({
            "user_id": user.user_id.clone(),
            "email": user.email.clone(),
            "role": user.role.clone(),
            "status": user.status,
        }),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(UserResponse { user })))
}

async fn list_users<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
) -> Result<Json<UsersResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::AdministerUsers,
    )
    .await?;
    let users = state
        .store
        .list_users(&principal)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(UsersResponse { users }))
}

async fn assign_user_role<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Json(request): Json<AssignUserRoleRequest>,
) -> Result<Json<UserResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::AssignRoles).await?;
    let role = parse_role(&request.role)?;
    let user = state
        .store
        .assign_user_role(&principal, &user_id, role)
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace).actor(principal.user_id.clone()),
        "rbac.role.assign",
        TraceResult::Completed,
        serde_json::json!({ "user_id": user.user_id.clone(), "role": user.role.clone() }),
    )
    .await?;
    Ok(Json(UserResponse { user }))
}

async fn deactivate_user<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> Result<Json<UserResponse>, ApiError>
where
    S: EnterpriseStore,
{
    set_user_status(&state, &trace, headers, user_id, UserStatus::Inactive).await
}

async fn reactivate_user<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> Result<Json<UserResponse>, ApiError>
where
    S: EnterpriseStore,
{
    set_user_status(&state, &trace, headers, user_id, UserStatus::Active).await
}

async fn set_user_status<S>(
    state: &AppState<S>,
    trace: &TraceContext,
    headers: HeaderMap,
    user_id: String,
    status: UserStatus,
) -> Result<Json<UserResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(state, &headers).await?;
    authorize(state, trace, &principal, EnterpriseAction::AdministerUsers).await?;
    let user = state
        .store
        .set_user_status(&principal, &user_id, status)
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        state,
        EvidenceRecordContext::new(trace).actor(principal.user_id.clone()),
        "user.status",
        TraceResult::Completed,
        serde_json::json!({ "user_id": user.user_id.clone(), "status": user.status }),
    )
    .await?;
    Ok(Json(UserResponse { user }))
}

async fn register_workspace<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Json(request): Json<RegisterWorkspaceRequest>,
) -> Result<(StatusCode, Json<WorkspaceResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::ManageWorkspaces,
    )
    .await?;
    let workspace = state
        .store
        .register_workspace_root(&principal, request.root_path)
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(principal.user_id.clone())
            .workspace(workspace.workspace_id.clone()),
        "workspace.register",
        TraceResult::Completed,
        serde_json::json!({
            "workspace_id": workspace.workspace_id.clone(),
            "root_path": workspace.root_path.clone(),
        }),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(WorkspaceResponse { workspace })))
}

async fn list_workspaces<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
) -> Result<Json<WorkspacesResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize_any(
        &state,
        &trace,
        &principal,
        &[
            EnterpriseAction::ManageWorkspaces,
            EnterpriseAction::StartWorker,
            EnterpriseAction::ReadThreads,
        ],
    )
    .await?;
    let roots = state
        .store
        .list_workspace_roots(&principal)
        .await
        .map_err(ApiError::internal)?;
    let workspaces = roots
        .into_iter()
        .map(|root| WorkspaceRootRecord {
            workspace_id: root.clone(),
            root_path: root,
            created_by: principal.user_id.clone(),
            created_at: Utc::now(),
        })
        .collect();
    Ok(Json(WorkspacesResponse { workspaces }))
}

async fn list_user_workspaces<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
) -> Result<Json<UserWorkspacesResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize_any(
        &state,
        &trace,
        &principal,
        &[
            EnterpriseAction::ManageWorkspaces,
            EnterpriseAction::StartWorker,
            EnterpriseAction::ReadThreads,
        ],
    )
    .await?;
    let user_workspaces = state
        .store
        .list_user_workspaces(&principal)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(UserWorkspacesResponse { user_workspaces }))
}

async fn list_projects<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Query(query): Query<ProjectQuery>,
) -> Result<Json<ProjectsResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize_any(
        &state,
        &trace,
        &principal,
        &[
            EnterpriseAction::ManageWorkspaces,
            EnterpriseAction::StartWorker,
            EnterpriseAction::ReadThreads,
        ],
    )
    .await?;
    let mut projects = state
        .store
        .list_projects(&principal, query.include_deleted.unwrap_or(false))
        .await
        .map_err(ApiError::internal)?;
    if let Some(user_id) = query.user_id {
        projects.retain(|project| project.owner_user_id == user_id);
    }
    Ok(Json(ProjectsResponse { projects }))
}

async fn list_user_workspace_projects<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(user_workspace_id): Path<String>,
) -> Result<Json<ProjectsResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize_any(
        &state,
        &trace,
        &principal,
        &[
            EnterpriseAction::ManageWorkspaces,
            EnterpriseAction::StartWorker,
            EnterpriseAction::ReadThreads,
        ],
    )
    .await?;
    let projects = state
        .store
        .list_projects(&principal, false)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .filter(|project| project.user_workspace_id == user_workspace_id)
        .collect();
    Ok(Json(ProjectsResponse { projects }))
}

async fn create_project<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Json(request): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<ProjectResponse>), ApiError>
where
    S: EnterpriseStore,
{
    create_project_with_user_workspace(&state, &trace, headers, request).await
}

async fn create_user_workspace_project<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(user_workspace_id): Path<String>,
    Json(mut request): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<ProjectResponse>), ApiError>
where
    S: EnterpriseStore,
{
    request.user_workspace_id = Some(user_workspace_id);
    create_project_with_user_workspace(&state, &trace, headers, request).await
}

async fn create_project_with_user_workspace<S>(
    state: &AppState<S>,
    trace: &TraceContext,
    headers: HeaderMap,
    request: CreateProjectRequest,
) -> Result<(StatusCode, Json<ProjectResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(state, &headers).await?;
    authorize_any(
        state,
        trace,
        &principal,
        &[
            EnterpriseAction::ManageWorkspaces,
            EnterpriseAction::StartWorker,
            EnterpriseAction::ReadThreads,
        ],
    )
    .await?;
    let project = state
        .store
        .create_project(
            &principal,
            CreateProjectInput {
                name: request.name,
                user_workspace_id: request.user_workspace_id,
            },
        )
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        state,
        EvidenceRecordContext::new(trace).actor(principal.user_id.clone()),
        "project.create",
        TraceResult::Completed,
        serde_json::json!({
            "project_id": project.project_id.clone(),
            "owner_user_id": project.owner_user_id.clone(),
            "user_workspace_id": project.user_workspace_id.clone(),
        }),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(ProjectResponse { project })))
}

async fn get_project<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<ProjectResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize_any(
        &state,
        &trace,
        &principal,
        &[
            EnterpriseAction::ManageWorkspaces,
            EnterpriseAction::StartWorker,
            EnterpriseAction::ReadThreads,
        ],
    )
    .await?;
    let project = state
        .store
        .list_projects(&principal, false)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .find(|project| project.project_id == project_id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "project not found"))?;
    Ok(Json(ProjectResponse { project }))
}

async fn update_project<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(request): Json<UpdateProjectRequest>,
) -> Result<Json<ProjectResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::ManageWorkspaces,
    )
    .await?;
    let project = state
        .store
        .update_project(
            &principal,
            &project_id,
            UpdateProjectInput { name: request.name },
        )
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace).actor(principal.user_id.clone()),
        "project.update",
        TraceResult::Completed,
        serde_json::json!({
            "project_id": project.project_id.clone(),
            "owner_user_id": project.owner_user_id.clone(),
            "user_workspace_id": project.user_workspace_id.clone(),
        }),
    )
    .await?;
    Ok(Json(ProjectResponse { project }))
}

async fn delete_project<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<ProjectResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::ManageWorkspaces,
    )
    .await?;
    let project = state
        .store
        .delete_project(&principal, &project_id)
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace).actor(principal.user_id.clone()),
        "project.delete",
        TraceResult::Completed,
        serde_json::json!({
            "project_id": project.project_id.clone(),
            "owner_user_id": project.owner_user_id.clone(),
            "user_workspace_id": project.user_workspace_id.clone(),
        }),
    )
    .await?;
    Ok(Json(ProjectResponse { project }))
}

async fn restore_project<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<ProjectResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::ManageWorkspaces,
    )
    .await?;
    let project = state
        .store
        .restore_project(&principal, &project_id)
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace).actor(principal.user_id.clone()),
        "project.restore",
        TraceResult::Completed,
        serde_json::json!({
            "project_id": project.project_id.clone(),
            "owner_user_id": project.owner_user_id.clone(),
            "user_workspace_id": project.user_workspace_id.clone(),
        }),
    )
    .await?;
    Ok(Json(ProjectResponse { project }))
}

async fn list_project_threads<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<SessionsResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let project = get_project(State(state), Extension(trace), headers, Path(project_id)).await?;
    Ok(Json(SessionsResponse {
        sessions: project.project.threads.clone(),
    }))
}

async fn create_project_thread<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(request): Json<CreateProjectThreadRequest>,
) -> Result<(StatusCode, Json<SessionResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::StartWorker).await?;
    let session = state
        .store
        .create_project_thread(
            &principal,
            CreateProjectThreadInput {
                project_id,
                repository_id: request.repository_id,
                session_id: request.session_id,
                title: request.title,
            },
        )
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(principal.user_id.clone())
            .workspace(session.workspace_id.clone())
            .session(session.session_id.clone()),
        "thread.create",
        TraceResult::Completed,
        serde_json::json!({
            "thread_id": session.session_id.clone(),
            "project_id": session.project_id.clone(),
            "repository_id": session.repository_id.clone(),
        }),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(SessionResponse { session })))
}

async fn clone_project_repository<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(request): Json<CloneProjectRepositoryRequest>,
) -> Result<(StatusCode, Json<RepositoryResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::StartWorker).await?;
    if let Err(error) =
        repo_clone::validate_clone_request(&request.repo_url, &request.destination_name)
    {
        record_audit(
            &state,
            EvidenceRecordContext::new(&trace).actor(principal.user_id.clone()),
            "repository.clone",
            TraceResult::Denied,
            serde_json::json!({
                "project_id": project_id.clone(),
                "repo_url": redacted_repo_url(&request.repo_url),
                "destination_name": request.destination_name.clone(),
            }),
        )
        .await?;
        return Err(ApiError::storage(error));
    }
    let repository = state
        .store
        .clone_project_repository(
            &principal,
            CloneProjectRepositoryInput {
                project_id,
                repo_url: request.repo_url.clone(),
                destination_name: request.destination_name,
            },
        )
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace).actor(principal.user_id.clone()),
        "repository.clone",
        TraceResult::Completed,
        serde_json::json!({
            "repository_id": repository.repository_id.clone(),
            "project_id": repository.project_id.clone(),
            "repo_url": redacted_repo_url(&request.repo_url),
        }),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(RepositoryResponse { repository })))
}

async fn validate_workspace<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Json(request): Json<ValidateWorkspaceRequest>,
) -> Result<Json<ValidateWorkspaceResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::GrantWorkspaceAccess,
    )
    .await?;
    let workspace_path = state
        .store
        .validate_workspace_path(&principal, request.workspace_path)
        .await
        .map_err(ApiError::storage)?;
    Ok(Json(ValidateWorkspaceResponse { workspace_path }))
}

async fn assign_workspace<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Json(request): Json<AssignWorkspaceRequest>,
) -> Result<(StatusCode, Json<WorkspaceAssignmentResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::GrantWorkspaceAccess,
    )
    .await?;
    let assignment = state
        .store
        .assign_workspace_to_user(&principal, &request.user_id, request.workspace_root)
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(principal.user_id.clone())
            .workspace(assignment.workspace_root.clone()),
        "workspace.assign",
        TraceResult::Completed,
        serde_json::json!({
            "assignment_id": assignment.assignment_id.clone(),
            "user_id": assignment.user_id.clone(),
            "workspace_root": assignment.workspace_root.clone(),
        }),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(WorkspaceAssignmentResponse { assignment }),
    ))
}

async fn create_context_pack<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Json(request): Json<CreateContextPackRequest>,
) -> Result<(StatusCode, Json<ContextPackResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::ManageContextPacks,
    )
    .await?;
    let (pack, documents) = state
        .store
        .create_context_pack(
            &principal,
            CreateContextPackInput {
                name: request.name,
                documents: request.documents,
            },
        )
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace).actor(principal.user_id.clone()),
        "context_pack.create",
        TraceResult::Completed,
        serde_json::json!({
            "pack_id": pack.pack_id.clone(),
            "name": pack.name.clone(),
            "document_count": documents.len(),
        }),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(ContextPackResponse { pack, documents }),
    ))
}

async fn list_context_packs<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
) -> Result<Json<ContextPacksResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::ManageContextPacks,
    )
    .await?;
    let packs = state
        .store
        .list_context_packs(&principal)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(ContextPacksResponse { packs }))
}

async fn list_context_pack_files<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(pack_id): Path<String>,
) -> Result<Json<ContextPackFilesResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::ManageContextPacks,
    )
    .await?;
    let files = state
        .store
        .list_context_pack_files(&principal, &pack_id)
        .await
        .map_err(ApiError::storage)?;
    Ok(Json(ContextPackFilesResponse { files }))
}

async fn create_context_pack_file<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(pack_id): Path<String>,
    Json(request): Json<CreateContextPackFileRequest>,
) -> Result<(StatusCode, Json<ContextPackFileResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::ManageContextPacks,
    )
    .await?;
    let content_bytes = BASE64_STANDARD
        .decode(request.content_base64.as_bytes())
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "content_base64 is invalid"))?;
    let file = state
        .store
        .create_context_pack_file(
            &principal,
            CreateContextPackFileInput {
                pack_id,
                relative_path: request.relative_path,
                content_bytes,
                content_type: request.content_type,
                load_order: request.load_order,
                required: request.required,
                file_kind: request.file_kind,
                loadable: request.loadable,
                source_type: request.source_type,
            },
        )
        .await
        .map_err(ApiError::storage)?;
    record_context_pack_file_audit(&state, &trace, &principal, "context_pack.file.add", &file)
        .await?;
    Ok((StatusCode::CREATED, Json(ContextPackFileResponse { file })))
}

async fn update_context_pack_file<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path((pack_id, document_id)): Path<(String, String)>,
    Json(request): Json<UpdateContextPackFileRequest>,
) -> Result<Json<ContextPackFileResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::ManageContextPacks,
    )
    .await?;
    let file = state
        .store
        .update_context_pack_file(
            &principal,
            UpdateContextPackFileInput {
                pack_id,
                document_id,
                relative_path: request.relative_path,
                content_type: request.content_type,
                load_order: request.load_order,
                required: request.required,
                file_kind: request.file_kind,
                loadable: request.loadable,
                source_type: request.source_type,
            },
        )
        .await
        .map_err(ApiError::storage)?;
    record_context_pack_file_audit(
        &state,
        &trace,
        &principal,
        "context_pack.file.update",
        &file,
    )
    .await?;
    Ok(Json(ContextPackFileResponse { file }))
}

async fn delete_context_pack_file<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path((pack_id, document_id)): Path<(String, String)>,
) -> Result<Json<ContextPackFileResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::ManageContextPacks,
    )
    .await?;
    let file = state
        .store
        .delete_context_pack_file(&principal, &pack_id, &document_id)
        .await
        .map_err(ApiError::storage)?;
    record_context_pack_file_audit(
        &state,
        &trace,
        &principal,
        "context_pack.file.remove",
        &file,
    )
    .await?;
    Ok(Json(ContextPackFileResponse { file }))
}

async fn download_context_pack_file<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path((pack_id, document_id)): Path<(String, String)>,
) -> Result<Response, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::ManageContextPacks,
    )
    .await?;
    let file = state
        .store
        .get_context_pack_file(&principal, &pack_id, &document_id)
        .await
        .map_err(ApiError::storage)?;
    record_context_pack_file_audit(
        &state,
        &trace,
        &principal,
        "context_pack.file.download",
        &file,
    )
    .await?;
    let disposition = format!(
        "attachment; filename=\"{}\"",
        file.filename.replace('"', "")
    );
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, file.content_type.clone()),
            (axum::http::header::CONTENT_DISPOSITION, disposition),
        ],
        file.content_bytes,
    )
        .into_response())
}

async fn assign_context_pack<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Json(request): Json<AssignContextPackRequest>,
) -> Result<(StatusCode, Json<ContextPackAssignmentResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::ManageContextPacks,
    )
    .await?;
    let assignment = state
        .store
        .assign_context_pack(
            &principal,
            AssignContextPackInput {
                pack_id: request.pack_id,
                user_id: request.user_id,
                workspace_id: request.workspace_id,
                assignment_order: request.assignment_order,
                required_session: request.required_session,
                required_worker: request.required_worker,
            },
        )
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace).actor(principal.user_id.clone()),
        "context_pack.assign",
        TraceResult::Completed,
        serde_json::json!({
            "assignment_id": assignment.assignment_id.clone(),
            "pack_id": assignment.pack_id.clone(),
            "assignment_source": assignment.assignment_source.clone(),
            "assignment_order": assignment.assignment_order,
        }),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(ContextPackAssignmentResponse { assignment }),
    ))
}

async fn list_context_pack_assignments<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
) -> Result<Json<ContextPackAssignmentsResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::ManageContextPacks,
    )
    .await?;
    let assignments = state
        .store
        .list_context_pack_assignments(&principal)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(ContextPackAssignmentsResponse { assignments }))
}

async fn bulk_assign_context_packs<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Json(request): Json<BulkAssignContextPacksRequest>,
) -> Result<(StatusCode, Json<BulkContextPackAssignmentsResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::ManageContextPacks,
    )
    .await?;

    if request.pack_ids.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "select at least one context pack",
        ));
    }
    let user_ids = nullable_selection(request.user_ids);
    let workspace_ids = nullable_selection(request.workspace_ids);

    let mut assignments = Vec::new();
    let mut order_offset = 0;
    for pack_id in request.pack_ids {
        for user_id in &user_ids {
            for workspace_id in &workspace_ids {
                let assignment = state
                    .store
                    .assign_context_pack(
                        &principal,
                        AssignContextPackInput {
                            pack_id: pack_id.clone(),
                            user_id: user_id.clone(),
                            workspace_id: workspace_id.clone(),
                            assignment_order: request.assignment_order + order_offset * 10,
                            required_session: request.required_session,
                            required_worker: request.required_worker,
                        },
                    )
                    .await
                    .map_err(ApiError::storage)?;
                assignments.push(assignment);
                order_offset += 1;
            }
        }
    }

    record_audit(
        &state,
        EvidenceRecordContext::new(&trace).actor(principal.user_id.clone()),
        "context_pack.assign.bulk",
        TraceResult::Completed,
        serde_json::json!({
            "assignment_count": assignments.len(),
        }),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(BulkContextPackAssignmentsResponse { assignments }),
    ))
}

async fn delete_context_pack_assignment<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(assignment_id): Path<String>,
) -> Result<Json<ContextPackAssignmentResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(
        &state,
        &trace,
        &principal,
        EnterpriseAction::ManageContextPacks,
    )
    .await?;
    let assignment = state
        .store
        .delete_context_pack_assignment(&principal, &assignment_id)
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace).actor(principal.user_id.clone()),
        "context_pack.assignment.remove",
        TraceResult::Completed,
        serde_json::json!({
            "assignment_id": assignment.assignment_id.clone(),
            "pack_id": assignment.pack_id.clone(),
            "assignment_source": assignment.assignment_source.clone(),
        }),
    )
    .await?;
    Ok(Json(ContextPackAssignmentResponse { assignment }))
}

async fn query_audit<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Query(query): Query<AuditQuery>,
) -> Result<Json<AuditResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::ReadAudit).await?;
    let result: EvidenceQueryResult = state
        .store
        .query_evidence(
            &principal,
            EvidenceQuery {
                trace_id: query.trace_id,
            },
        )
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(AuditResponse {
        audit_events: result.audit_events,
        execution_receipts: result.execution_receipts,
        context_receipts: result.context_receipts,
    }))
}

async fn create_output<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Json(request): Json<CreateOutputRequest>,
) -> Result<(StatusCode, Json<OutputResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::ManageOutputs).await?;
    let owner_user_id = request.owner_user_id.clone().ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "owner_user_id is required when assigning output metadata",
        )
    })?;
    let category = parse_output_category(&request.category)?;
    validate_output_status(&request.status)?;
    let metadata_json = sanitized_output_metadata(request.metadata_json.unwrap_or_default());
    let output = state
        .store
        .create_output(CreateOutputInput {
            owner_user_id,
            workspace_id: request.workspace_id,
            session_id: request.session_id,
            worker_id: request.worker_id,
            category,
            output_type: request.output_type,
            title: request.title,
            artifact_path: request.artifact_path,
            status: request.status,
            metadata_json,
        })
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace).actor(principal.user_id.clone()),
        "output.metadata.create",
        TraceResult::Completed,
        serde_json::json!({
            "output_id": output.output_id.clone(),
            "owner_user_id": output.owner_user_id.clone(),
            "category": output.category,
            "output_type": output.output_type.clone(),
            "artifact_path": output.artifact_path.clone(),
            "status": output.status.clone(),
        }),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(OutputResponse { output })))
}

async fn list_outputs<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
) -> Result<Json<OutputsResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::ReadThreads).await?;
    let outputs = state
        .store
        .list_outputs(&principal)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(OutputsResponse { outputs }))
}

async fn download_output<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(output_id): Path<String>,
) -> Result<Response, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::ReadThreads).await?;
    let output = state
        .store
        .get_output(&principal, &output_id)
        .await
        .map_err(ApiError::storage)?;
    let artifact_path = user_output_artifact_path(&state.config, &principal.user_id, &output)
        .map_err(ApiError::storage)?;
    let bytes = tokio::fs::read(&artifact_path)
        .await
        .map_err(|error| ApiError::new(StatusCode::NOT_FOUND, error.to_string()))?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace).actor(principal.user_id.clone()),
        "output.download",
        TraceResult::Completed,
        serde_json::json!({
            "output_id": output.output_id,
            "category": output.category,
            "output_type": output.output_type,
            "artifact_path": output.artifact_path,
        }),
    )
    .await?;
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "text/markdown"),
            (
                axum::http::header::CONTENT_DISPOSITION,
                "attachment; filename=\"output.md\"",
            ),
        ],
        bytes,
    )
        .into_response())
}

async fn save_message_output<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path((thread_id, message_id)): Path<(String, String)>,
    Json(request): Json<SaveMessageOutputRequest>,
) -> Result<(StatusCode, Json<OutputResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::ReadThreads).await?;
    let title = request.title.trim().to_string();
    if title.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "output title is required",
        ));
    }
    let session = state
        .store
        .get_session(&principal, &thread_id)
        .await
        .map_err(ApiError::storage)?;
    let message = state
        .store
        .get_session_message(&principal, &thread_id, &message_id)
        .await
        .map_err(ApiError::storage)?;
    if message.kind != "assistant" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "only completed assistant messages can be saved as outputs",
        ));
    }
    let category = parse_output_category(request.category.as_deref().unwrap_or("deliverable"))?;
    let output_type = request
        .output_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("markdown_report")
        .to_string();
    let artifact_path = generated_chat_output_artifact_path(&thread_id, &title);
    let output = state
        .store
        .create_output(CreateOutputInput {
            owner_user_id: principal.user_id.clone(),
            workspace_id: Some(session.workspace_id.clone()),
            session_id: Some(session.session_id.clone()),
            worker_id: session.last_worker_id.clone(),
            category,
            output_type,
            title: title.clone(),
            artifact_path,
            status: "completed".to_string(),
            metadata_json: serde_json::json!({
                "source": "chat_assistant_message",
                "thread_id": thread_id,
                "message_id": message_id,
            }),
        })
        .await
        .map_err(ApiError::storage)?;
    let artifact_path = user_output_artifact_path(&state.config, &principal.user_id, &output)
        .map_err(ApiError::storage)?;
    if let Some(parent) = artifact_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    }
    tokio::fs::write(&artifact_path, message.text.as_bytes())
        .await
        .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(principal.user_id.clone())
            .workspace(session.workspace_id.clone())
            .session(session.session_id.clone()),
        "output.chat_message.save",
        TraceResult::Completed,
        serde_json::json!({
            "output_id": output.output_id.clone(),
            "category": output.category,
            "output_type": output.output_type.clone(),
            "artifact_path": output.artifact_path.clone(),
            "message_id": message.message_id.clone(),
        }),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(OutputResponse { output })))
}

async fn list_thread_references<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(thread_id): Path<String>,
) -> Result<Json<ThreadReferencesResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::ReadThreads).await?;
    let references = state
        .store
        .list_thread_references(&principal, &thread_id)
        .await
        .map_err(ApiError::storage)?;
    Ok(Json(ThreadReferencesResponse { references }))
}

async fn create_thread_reference<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(target_thread_id): Path<String>,
    Json(request): Json<CreateThreadReferenceRequest>,
) -> Result<(StatusCode, Json<ThreadSummaryPromptResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::ReadThreads).await?;
    if request.reference_type != "ai_summary" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "POST /v1/threads/{thread_id}/references currently creates ai_summary references",
        ));
    }
    let source_session = state
        .store
        .get_session(&principal, &request.source_thread_id)
        .await
        .map_err(ApiError::storage)?;
    let target_session = state
        .store
        .get_session(&principal, &target_thread_id)
        .await
        .map_err(ApiError::storage)?;
    let messages = state
        .store
        .list_session_messages(&principal, &source_session.session_id)
        .await
        .map_err(ApiError::storage)?;
    let bounded = bounded_thread_markdown(&source_session, &messages, request.max_chars);
    let reference = state
        .store
        .create_thread_reference(
            &principal,
            CreateThreadReferenceInput {
                source_thread_id: source_session.session_id.clone(),
                target_thread_id: target_session.session_id.clone(),
                source_output_id: None,
                output_id: None,
                reference_type: "ai_summary".to_string(),
                knowledge_origin: "ai_generated".to_string(),
                status: "pending".to_string(),
                metadata_json: serde_json::json!({
                    "source_thread_id": source_session.session_id,
                    "target_thread_id": target_session.session_id,
                    "source_truncated": bounded.truncated,
                    "message_count": messages.len(),
                }),
            },
        )
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(principal.user_id.clone())
            .workspace(target_session.workspace_id.clone())
            .session(target_session.session_id.clone()),
        "thread_reference.create",
        TraceResult::Completed,
        serde_json::json!({
            "reference_id": reference.reference_id.clone(),
            "source_thread_id": reference.source_thread_id.clone(),
            "target_thread_id": reference.target_thread_id.clone(),
            "reference_type": reference.reference_type.clone(),
            "knowledge_origin": reference.knowledge_origin.clone(),
            "status": reference.status.clone(),
            "source_truncated": bounded.truncated,
        }),
    )
    .await?;
    let summary_prompt = format!(
        "Summarize the referenced source thread for use in the current thread.\n\nReturn Markdown with exactly these sections:\n\n1. Decisions\n2. Findings\n3. Action Items\n4. Open Questions\n5. Handoff Notes\n\nDo not execute tasks, call tools, inspect repositories, or message another thread. Use only this bounded source-thread content:\n\n{}",
        bounded.content
    );
    Ok((
        StatusCode::CREATED,
        Json(ThreadSummaryPromptResponse {
            reference,
            summary_prompt,
            source_truncated: bounded.truncated,
        }),
    ))
}

async fn update_thread_reference<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(reference_id): Path<String>,
    Json(request): Json<UpdateThreadReferenceRequest>,
) -> Result<Json<ThreadReferenceResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::ReadThreads).await?;
    let reference = state
        .store
        .update_thread_reference(
            &principal,
            &reference_id,
            UpdateThreadReferenceInput {
                output_id: request.output_id,
                status: request.status,
                metadata_json: None,
            },
        )
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(principal.user_id.clone())
            .session(reference.target_thread_id.clone()),
        "thread_reference.update",
        TraceResult::Completed,
        serde_json::json!({
            "reference_id": reference.reference_id.clone(),
            "source_thread_id": reference.source_thread_id.clone(),
            "target_thread_id": reference.target_thread_id.clone(),
            "output_id": reference.output_id.clone(),
            "reference_type": reference.reference_type.clone(),
            "knowledge_origin": reference.knowledge_origin.clone(),
            "status": reference.status.clone(),
        }),
    )
    .await?;
    Ok(Json(ThreadReferenceResponse { reference }))
}

async fn export_thread_transcript<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(source_thread_id): Path<String>,
    Json(request): Json<CreateThreadArtifactRequest>,
) -> Result<(StatusCode, Json<ThreadReferenceOutputResponse>), ApiError>
where
    S: EnterpriseStore,
{
    create_thread_knowledge_output(
        state,
        trace,
        headers,
        source_thread_id,
        request,
        "transcript_export",
        "thread_transcript",
    )
    .await
}

async fn create_thread_handoff<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(source_thread_id): Path<String>,
    Json(request): Json<CreateThreadArtifactRequest>,
) -> Result<(StatusCode, Json<ThreadReferenceOutputResponse>), ApiError>
where
    S: EnterpriseStore,
{
    create_thread_knowledge_output(
        state,
        trace,
        headers,
        source_thread_id,
        request,
        "handoff",
        "thread_handoff",
    )
    .await
}

async fn import_thread_artifact<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(target_thread_id): Path<String>,
    Json(request): Json<ImportThreadArtifactRequest>,
) -> Result<(StatusCode, Json<ArtifactImportResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::ReadThreads).await?;
    let target_session = state
        .store
        .get_session(&principal, &target_thread_id)
        .await
        .map_err(ApiError::storage)?;
    let source_output = state
        .store
        .get_output(&principal, &request.source_output_id)
        .await
        .map_err(ApiError::storage)?;
    let output_path = user_output_artifact_path(&state.config, &principal.user_id, &source_output)
        .map_err(ApiError::storage)?;
    let content = tokio::fs::read_to_string(&output_path)
        .await
        .map_err(|error| ApiError::new(StatusCode::NOT_FOUND, error.to_string()))?;
    let bounded = bounded_text(content, request.max_chars);
    let source_thread_id = source_output
        .session_id
        .clone()
        .unwrap_or_else(|| target_session.session_id.clone());
    if source_output.session_id.is_some() {
        state
            .store
            .get_session(&principal, &source_thread_id)
            .await
            .map_err(ApiError::storage)?;
    }
    let reference = state
        .store
        .create_thread_reference(
            &principal,
            CreateThreadReferenceInput {
                source_thread_id,
                target_thread_id: target_session.session_id.clone(),
                source_output_id: Some(source_output.output_id.clone()),
                output_id: Some(source_output.output_id.clone()),
                reference_type: "artifact_import".to_string(),
                knowledge_origin: "user_generated".to_string(),
                status: "completed".to_string(),
                metadata_json: serde_json::json!({
                    "source_output_id": source_output.output_id,
                    "target_thread_id": target_session.session_id,
                    "excerpt_truncated": bounded.truncated,
                }),
            },
        )
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(principal.user_id.clone())
            .workspace(target_session.workspace_id.clone())
            .session(target_session.session_id.clone()),
        "thread_reference.create",
        TraceResult::Completed,
        serde_json::json!({
            "reference_id": reference.reference_id.clone(),
            "source_thread_id": reference.source_thread_id.clone(),
            "target_thread_id": reference.target_thread_id.clone(),
            "source_output_id": reference.source_output_id.clone(),
            "output_id": reference.output_id.clone(),
            "reference_type": reference.reference_type.clone(),
            "knowledge_origin": reference.knowledge_origin.clone(),
            "status": reference.status.clone(),
            "excerpt_truncated": bounded.truncated,
        }),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(ArtifactImportResponse {
            reference,
            excerpt: bounded.content,
            excerpt_truncated: bounded.truncated,
        }),
    ))
}

#[utoipa::path(
    get,
    path = "/v1/workers",
    responses(
        (status = 200, description = "Workers visible to the authenticated user", body = WorkersResponse),
        (status = 401, description = "Missing or invalid API token", body = ErrorResponse)
    ),
    security(("bearer" = []))
)]
async fn list_workers<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
) -> Result<Json<WorkersResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::ReadThreads).await?;
    let workers = state
        .store
        .list_workers(&principal)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(WorkersResponse { workers }))
}

#[utoipa::path(
    post,
    path = "/v1/threads",
    request_body = CreateSessionRequest,
    responses(
        (status = 201, description = "Coding session created for an allowlisted workspace", body = SessionResponse),
        (status = 401, description = "Missing or invalid API token", body = ErrorResponse),
        (status = 403, description = "Workspace path is not allowlisted", body = ErrorResponse),
        (status = 409, description = "Session already exists", body = ErrorResponse)
    ),
    security(("bearer" = []))
)]
async fn create_session<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Json(request): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<SessionResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::StartWorker).await?;
    let session = state
        .store
        .create_session(
            &principal,
            request.session_id,
            request.workspace_path,
            request.title,
        )
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(principal.user_id.clone())
            .workspace(session.workspace_id.clone())
            .session(session.session_id.clone()),
        "session.create",
        TraceResult::Completed,
        serde_json::json!({
            "session_id": session.session_id.clone(),
            "workspace_id": session.workspace_id.clone(),
            "title": session.title.clone(),
        }),
    )
    .await?;
    record_execution_receipt(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(principal.user_id.clone())
            .workspace(session.workspace_id.clone())
            .session(session.session_id.clone()),
        "session.create",
        TraceResult::Completed,
        serde_json::json!({ "title": session.title.clone() }),
    )
    .await?;
    let context_receipts = state
        .store
        .record_context_load(ContextLoadInput {
            trace_id: trace.trace_id.clone(),
            actor_user_id: principal.user_id.clone(),
            workspace_id: session.workspace_id.clone(),
            session_id: session.session_id.clone(),
            worker_id: None,
            phase: "session_start".to_string(),
        })
        .await
        .map_err(ApiError::storage)?;
    if !context_receipts.is_empty() {
        record_execution_receipt(
            &state,
            EvidenceRecordContext::new(&trace)
                .actor(principal.user_id.clone())
                .workspace(session.workspace_id.clone())
                .session(session.session_id.clone()),
            "context_pack.session_load",
            TraceResult::Completed,
            serde_json::json!({ "receipt_count": context_receipts.len() }),
        )
        .await?;
    }

    Ok((StatusCode::CREATED, Json(SessionResponse { session })))
}

#[utoipa::path(
    get,
    path = "/v1/threads",
    responses(
        (status = 200, description = "Sessions visible to the authenticated user", body = SessionsResponse),
        (status = 401, description = "Missing or invalid API token", body = ErrorResponse)
    ),
    security(("bearer" = []))
)]
async fn list_sessions<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
) -> Result<Json<SessionsResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::ReadThreads).await?;
    let sessions = state
        .store
        .list_sessions(&principal)
        .await
        .map_err(ApiError::internal)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace).actor(principal.user_id.clone()),
        "session.list",
        TraceResult::Completed,
        serde_json::json!({ "count": sessions.len() }),
    )
    .await?;
    Ok(Json(SessionsResponse { sessions }))
}

#[utoipa::path(
    get,
    path = "/v1/threads/{thread_id}",
    responses(
        (status = 200, description = "Session visible to the authenticated user", body = SessionResponse),
        (status = 401, description = "Missing or invalid API token", body = ErrorResponse),
        (status = 404, description = "Session not found", body = ErrorResponse)
    ),
    security(("bearer" = []))
)]
async fn get_session<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<SessionResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::ReadThreads).await?;
    let session = state
        .store
        .get_session(&principal, &session_id)
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(principal.user_id.clone())
            .workspace(session.workspace_id.clone())
            .session(session.session_id.clone()),
        "session.get",
        TraceResult::Completed,
        serde_json::json!({ "session_id": session.session_id.clone() }),
    )
    .await?;
    Ok(Json(SessionResponse { session }))
}

async fn update_thread<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(request): Json<UpdateThreadRequest>,
) -> Result<Json<SessionResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::ReadThreads).await?;
    let title = request.title.trim().to_string();
    if title.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "thread title is required",
        ));
    }
    let session = state
        .store
        .update_session_title(
            &principal,
            &session_id,
            UpdateSessionTitleInput {
                title: title.clone(),
            },
        )
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(principal.user_id.clone())
            .workspace(session.workspace_id.clone())
            .session(session.session_id.clone()),
        "thread.update",
        TraceResult::Completed,
        serde_json::json!({
            "thread_id": session.session_id.clone(),
            "title_length": title.chars().count(),
        }),
    )
    .await?;
    Ok(Json(SessionResponse { session }))
}

async fn delete_thread<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<SessionResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::ReadThreads).await?;
    let session = state
        .store
        .delete_session(&principal, &session_id)
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(principal.user_id.clone())
            .workspace(session.workspace_id.clone())
            .session(session.session_id.clone()),
        "thread.delete",
        TraceResult::Completed,
        serde_json::json!({
            "thread_id": session.session_id.clone(),
            "project_id": session.project_id.clone(),
        }),
    )
    .await?;
    Ok(Json(SessionResponse { session }))
}

async fn create_session_message<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(request): Json<CreateSessionMessageRequest>,
) -> Result<(StatusCode, Json<SessionMessageResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::ReadThreads).await?;
    validate_session_message_request(&request)?;
    let message = state
        .store
        .create_session_message(
            &principal,
            &session_id,
            CreateSessionMessageInput {
                kind: request.kind,
                label: request.label,
                text: request.text,
                retry_of_message_id: request.retry_of_message_id,
                supersedes_message_id: request.supersedes_message_id,
                context_cutoff_message_id: request.context_cutoff_message_id,
            },
        )
        .await
        .map_err(ApiError::storage)?;
    Ok((
        StatusCode::CREATED,
        Json(SessionMessageResponse { message }),
    ))
}

async fn list_session_messages<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<SessionMessagesResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::ReadThreads).await?;
    let messages = state
        .store
        .list_session_messages(&principal, &session_id)
        .await
        .map_err(ApiError::storage)?;
    Ok(Json(SessionMessagesResponse { messages }))
}

async fn upsert_response_feedback<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path((thread_id, message_id)): Path<(String, String)>,
    Json(request): Json<ResponseFeedbackRequest>,
) -> Result<Json<ResponseFeedbackResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::ReadThreads).await?;
    let input = validate_response_feedback_request(request)?;
    let (feedback, preferences) = state
        .store
        .upsert_response_feedback(&principal, &thread_id, &message_id, input)
        .await
        .map_err(ApiError::storage)?;
    Ok(Json(ResponseFeedbackResponse {
        feedback,
        preferences,
    }))
}

async fn get_response_preferences<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
) -> Result<Json<ResponsePreferencesResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::ReadThreads).await?;
    let preferences = state
        .store
        .get_response_preferences(&principal)
        .await
        .map_err(ApiError::storage)?;
    Ok(Json(ResponsePreferencesResponse { preferences }))
}

async fn reset_response_preferences<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::ReadThreads).await?;
    state
        .store
        .reset_response_preferences(&principal)
        .await
        .map_err(ApiError::storage)?;
    Ok(StatusCode::NO_CONTENT)
}

fn validate_session_message_request(request: &CreateSessionMessageRequest) -> Result<(), ApiError> {
    if !matches!(request.kind.as_str(), "system" | "user" | "assistant") {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "session message kind must be system, user, or assistant",
        ));
    }
    if request.label.trim().is_empty() || request.text.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "session message label and text are required",
        ));
    }
    Ok(())
}

fn validate_response_feedback_request(
    request: ResponseFeedbackRequest,
) -> Result<UpsertResponseFeedbackInput, ApiError> {
    if !matches!(request.rating.as_str(), "good" | "bad") {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "feedback rating must be good or bad",
        ));
    }
    let mut reason_tags = Vec::new();
    for tag in request.reason_tags {
        let normalized = tag.trim().to_ascii_lowercase();
        if normalized.is_empty() || reason_tags.contains(&normalized) {
            continue;
        }
        if !matches!(
            normalized.as_str(),
            "too_verbose"
                | "too_generic"
                | "wrong_context"
                | "used_repo_when_not_needed"
                | "poor_formatting"
                | "missed_business_goal"
                | "raw_tool_output"
                | "other"
        ) {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "unknown feedback reason tag",
            ));
        }
        reason_tags.push(normalized);
    }
    Ok(UpsertResponseFeedbackInput {
        rating: request.rating,
        reason_tags,
        comment: request.comment.and_then(|comment| {
            (!comment.trim().is_empty()).then(|| "<redacted-comment>".to_string())
        }),
    })
}

#[utoipa::path(
    post,
    path = "/v1/workers",
    request_body = StartWorkerRequest,
    responses(
        (status = 201, description = "Worker start recorded", body = WorkerResponse),
        (status = 401, description = "Missing or invalid API token", body = ErrorResponse)
    ),
    security(("bearer" = []))
)]
async fn start_worker<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Json(request): Json<StartWorkerRequest>,
) -> Result<(StatusCode, Json<WorkerResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::StartWorker).await?;
    let starting_worker = state
        .store
        .start_worker(&principal, request.workspace_path, request.session_id)
        .await
        .map_err(ApiError::storage)?;
    let runtime = match state
        .worker_runtime
        .launch(&starting_worker, &state.config)
        .await
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = state
                .store
                .update_worker_runtime(
                    &principal,
                    &starting_worker.worker_id,
                    WorkerState::Failed,
                    None,
                )
                .await;
            record_audit(
                &state,
                EvidenceRecordContext::new(&trace)
                    .actor(principal.user_id.clone())
                    .workspace(starting_worker.workspace_id.clone())
                    .session(starting_worker.session_id.clone())
                    .worker(starting_worker.worker_id.clone()),
                "worker.start",
                TraceResult::Failed,
                serde_json::json!({
                    "worker_id": starting_worker.worker_id.clone(),
                    "workspace_id": starting_worker.workspace_id.clone(),
                    "session_id": starting_worker.session_id.clone(),
                }),
            )
            .await?;
            record_execution_receipt(
                &state,
                EvidenceRecordContext::new(&trace)
                    .actor(principal.user_id.clone())
                    .workspace(starting_worker.workspace_id.clone())
                    .session(starting_worker.session_id.clone())
                    .worker(starting_worker.worker_id.clone()),
                "worker.start",
                TraceResult::Failed,
                serde_json::json!({
                    "worker_id": starting_worker.worker_id.clone(),
                    "workspace_id": starting_worker.workspace_id.clone(),
                    "session_id": starting_worker.session_id.clone(),
                }),
            )
            .await?;
            return Err(ApiError::internal(error));
        }
    };
    let worker = state
        .store
        .update_worker_runtime(
            &principal,
            &starting_worker.worker_id,
            WorkerState::Running,
            Some(runtime),
        )
        .await
        .map_err(ApiError::internal)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(principal.user_id.clone())
            .workspace(worker.workspace_id.clone())
            .session(worker.session_id.clone())
            .worker(worker.worker_id.clone()),
        "worker.start",
        TraceResult::Completed,
        serde_json::json!({
            "worker_id": worker.worker_id.clone(),
            "workspace_id": worker.workspace_id.clone(),
            "session_id": worker.session_id.clone(),
            "state": worker.state,
        }),
    )
    .await?;
    record_execution_receipt(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(principal.user_id.clone())
            .workspace(worker.workspace_id.clone())
            .session(worker.session_id.clone())
            .worker(worker.worker_id.clone()),
        "worker.start",
        TraceResult::Completed,
        serde_json::json!({
            "worker_id": worker.worker_id.clone(),
            "workspace_id": worker.workspace_id.clone(),
            "session_id": worker.session_id.clone(),
            "state": worker.state,
        }),
    )
    .await?;
    let context_receipts = state
        .store
        .record_context_load(ContextLoadInput {
            trace_id: trace.trace_id.clone(),
            actor_user_id: principal.user_id.clone(),
            workspace_id: worker.workspace_id.clone(),
            session_id: worker.session_id.clone(),
            worker_id: Some(worker.worker_id.clone()),
            phase: "worker_start".to_string(),
        })
        .await
        .map_err(ApiError::storage)?;
    if !context_receipts.is_empty() {
        record_execution_receipt(
            &state,
            EvidenceRecordContext::new(&trace)
                .actor(principal.user_id.clone())
                .workspace(worker.workspace_id.clone())
                .session(worker.session_id.clone())
                .worker(worker.worker_id.clone()),
            "context_pack.worker_load",
            TraceResult::Completed,
            serde_json::json!({ "receipt_count": context_receipts.len() }),
        )
        .await?;
    }
    Ok((StatusCode::CREATED, Json(WorkerResponse { worker })))
}

async fn start_thread_worker<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(thread_id): Path<String>,
) -> Result<(StatusCode, Json<WorkerResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::StartWorker).await?;
    let session = state
        .store
        .get_session(&principal, &thread_id)
        .await
        .map_err(ApiError::storage)?;
    let starting_worker = state
        .store
        .start_worker(&principal, session.workspace_path, session.session_id)
        .await
        .map_err(ApiError::storage)?;
    let runtime = state
        .worker_runtime
        .launch(&starting_worker, &state.config)
        .await
        .map_err(ApiError::internal)?;
    let worker = state
        .store
        .update_worker_runtime(
            &principal,
            &starting_worker.worker_id,
            WorkerState::Running,
            Some(runtime),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(WorkerResponse { worker })))
}

#[utoipa::path(
    delete,
    path = "/v1/workers/{worker_id}",
    responses(
        (status = 200, description = "Worker stopped", body = WorkerResponse),
        (status = 401, description = "Missing or invalid API token", body = ErrorResponse)
    ),
    security(("bearer" = []))
)]
async fn stop_worker<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(worker_id): Path<String>,
) -> Result<Json<WorkerResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::StartWorker).await?;
    state
        .worker_runtime
        .stop(&worker_id)
        .await
        .map_err(ApiError::internal)?;
    let worker = state
        .store
        .stop_worker(&principal, &worker_id)
        .await
        .map_err(ApiError::internal)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(principal.user_id.clone())
            .workspace(worker.workspace_id.clone())
            .session(worker.session_id.clone())
            .worker(worker.worker_id.clone()),
        "worker.stop",
        TraceResult::Completed,
        serde_json::json!({
            "worker_id": worker.worker_id.clone(),
            "workspace_id": worker.workspace_id.clone(),
            "session_id": worker.session_id.clone(),
        }),
    )
    .await?;
    record_execution_receipt(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(principal.user_id.clone())
            .workspace(worker.workspace_id.clone())
            .session(worker.session_id.clone())
            .worker(worker.worker_id.clone()),
        "worker.stop",
        TraceResult::Completed,
        serde_json::json!({
            "worker_id": worker.worker_id.clone(),
            "workspace_id": worker.workspace_id.clone(),
            "session_id": worker.session_id.clone(),
        }),
    )
    .await?;
    Ok(Json(WorkerResponse { worker }))
}

#[utoipa::path(
    post,
    path = "/v1/workers/{worker_id}/handoffs",
    responses(
        (status = 201, description = "Short-lived worker handoff token issued", body = WorkerHandoffResponse),
        (status = 401, description = "Missing or invalid API token", body = ErrorResponse),
        (status = 404, description = "Worker not found", body = ErrorResponse),
        (status = 409, description = "Worker is not ready for handoff", body = ErrorResponse)
    ),
    security(("bearer" = []))
)]
async fn issue_worker_handoff<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    headers: HeaderMap,
    Path(worker_id): Path<String>,
) -> Result<(StatusCode, Json<WorkerHandoffResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::StartWorker).await?;
    let worker = state
        .store
        .get_worker(&principal, &worker_id)
        .await
        .map_err(ApiError::storage)?;
    let issued_token = auth::issue_worker_handoff_token(
        &principal.user_id,
        &worker.workspace_id,
        &worker.session_id,
        &worker.worker_id,
        Duration::from_secs(state.config.handoff_token_ttl_seconds),
        &state.config.handoff_token_secret,
    )
    .map_err(ApiError::internal)?;
    let claims =
        auth::decode_worker_handoff_token(&issued_token.jwt, &state.config.handoff_token_secret)
            .map_err(ApiError::internal)?;
    let expires_at = DateTime::<Utc>::from_timestamp(claims.exp as i64, 0)
        .ok_or_else(|| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "invalid handoff exp"))?;
    let handoff = state
        .store
        .create_worker_handoff(&principal, &worker.worker_id, issued_token.jti, expires_at)
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(principal.user_id.clone())
            .workspace(handoff.workspace_id.clone())
            .session(handoff.session_id.clone())
            .worker(handoff.worker_id.clone()),
        "worker.handoff.issue",
        TraceResult::Completed,
        serde_json::json!({
            "jti": handoff.jti.clone(),
            "worker_id": handoff.worker_id.clone(),
            "workspace_id": handoff.workspace_id.clone(),
            "session_id": handoff.session_id.clone(),
            "expires_at": handoff.expires_at,
        }),
    )
    .await?;
    record_execution_receipt(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(principal.user_id.clone())
            .workspace(handoff.workspace_id.clone())
            .session(handoff.session_id.clone())
            .worker(handoff.worker_id.clone()),
        "worker.handoff.issue",
        TraceResult::Completed,
        serde_json::json!({
            "jti": handoff.jti.clone(),
            "expires_at": handoff.expires_at,
        }),
    )
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(WorkerHandoffResponse {
            handoff_token: issued_token.jwt,
            jti: handoff.jti,
            owner_user_id: handoff.owner_user_id,
            worker_id: handoff.worker_id,
            workspace_id: handoff.workspace_id,
            session_id: handoff.session_id,
            socket_path: handoff.socket_path,
            expires_at: handoff.expires_at,
        }),
    ))
}

#[utoipa::path(
    post,
    path = "/v1/worker-handoffs/{jti}/consume",
    request_body = ConsumeWorkerHandoffRequest,
    responses(
        (status = 200, description = "Worker handoff token consumed", body = ConsumeWorkerHandoffResponse),
        (status = 401, description = "Invalid, expired, or mismatched worker handoff token", body = ErrorResponse),
        (status = 409, description = "Worker handoff token was already consumed", body = ErrorResponse)
    )
)]
async fn consume_worker_handoff<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    Path(jti): Path<String>,
    Json(request): Json<ConsumeWorkerHandoffRequest>,
) -> Result<Json<ConsumeWorkerHandoffResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let claims = auth::decode_worker_handoff_token(
        &request.handoff_token,
        &state.config.handoff_token_secret,
    )
    .map_err(|error| ApiError::new(StatusCode::UNAUTHORIZED, error.to_string()))?;
    if claims.jti != jti {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "worker handoff jti does not match request path",
        ));
    }
    let handoff = state
        .store
        .consume_worker_handoff(&claims)
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(handoff.owner_user_id.clone())
            .workspace(handoff.workspace_id.clone())
            .session(handoff.session_id.clone())
            .worker(handoff.worker_id.clone()),
        "worker.handoff.consume",
        TraceResult::Completed,
        serde_json::json!({
            "jti": handoff.jti.clone(),
            "worker_id": handoff.worker_id.clone(),
            "workspace_id": handoff.workspace_id.clone(),
            "session_id": handoff.session_id.clone(),
        }),
    )
    .await?;
    record_execution_receipt(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(handoff.owner_user_id.clone())
            .workspace(handoff.workspace_id.clone())
            .session(handoff.session_id.clone())
            .worker(handoff.worker_id.clone()),
        "worker.handoff.consume",
        TraceResult::Completed,
        serde_json::json!({ "jti": handoff.jti.clone() }),
    )
    .await?;
    let consumed_at = handoff.consumed_at.ok_or_else(|| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "worker handoff was not marked consumed",
        )
    })?;

    Ok(Json(ConsumeWorkerHandoffResponse {
        jti: handoff.jti,
        owner_user_id: handoff.owner_user_id,
        worker_id: handoff.worker_id,
        workspace_id: handoff.workspace_id,
        session_id: handoff.session_id,
        socket_path: handoff.socket_path,
        consumed_at,
    }))
}

#[utoipa::path(
    get,
    path = "/v1/workers/{worker_id}/rpc",
    responses(
        (status = 101, description = "Websocket tunnel to the scoped Codex worker"),
        (status = 401, description = "Invalid, expired, mismatched, or replayed worker handoff token", body = ErrorResponse)
    )
)]
async fn worker_rpc<S>(
    State(state): State<AppState<S>>,
    Extension(trace): Extension<TraceContext>,
    Path(worker_id): Path<String>,
    Query(query): Query<WorkerRpcQuery>,
    ws: WebSocketUpgrade,
) -> Result<axum::response::Response, ApiError>
where
    S: EnterpriseStore,
{
    let claims =
        auth::decode_worker_handoff_token(&query.handoff_token, &state.config.handoff_token_secret)
            .map_err(|error| ApiError::new(StatusCode::UNAUTHORIZED, error.to_string()))?;
    if claims.worker_id != worker_id {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "worker handoff token does not match requested worker",
        ));
    }
    let handoff = state
        .store
        .consume_worker_handoff(&claims)
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(handoff.owner_user_id.clone())
            .workspace(handoff.workspace_id.clone())
            .session(handoff.session_id.clone())
            .worker(handoff.worker_id.clone()),
        "worker.rpc.connect",
        TraceResult::Completed,
        serde_json::json!({
            "jti": handoff.jti.clone(),
            "worker_id": handoff.worker_id.clone(),
            "workspace_id": handoff.workspace_id.clone(),
            "session_id": handoff.session_id.clone(),
        }),
    )
    .await?;
    record_execution_receipt(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(handoff.owner_user_id.clone())
            .workspace(handoff.workspace_id.clone())
            .session(handoff.session_id.clone())
            .worker(handoff.worker_id.clone()),
        "worker.rpc.connect",
        TraceResult::Completed,
        serde_json::json!({ "jti": handoff.jti.clone() }),
    )
    .await?;
    let socket_path = handoff.socket_path;

    Ok(ws
        .on_upgrade(move |socket| async move {
            if let Err(error) = proxy_worker_rpc(socket, socket_path).await {
                eprintln!("worker rpc proxy failed: {error:#}");
            }
        })
        .into_response())
}

async fn authenticate<S>(
    state: &AppState<S>,
    headers: &HeaderMap,
) -> Result<crate::storage::AuthPrincipal, ApiError>
where
    S: EnterpriseStore,
{
    let token = auth_token(headers)?;
    state
        .store
        .authenticate_api_token(token)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "missing or invalid API token"))
}

async fn record_audit<S>(
    state: &AppState<S>,
    context: EvidenceRecordContext,
    event_type: &str,
    result: TraceResult,
    metadata_json: serde_json::Value,
) -> Result<(), ApiError>
where
    S: EnterpriseStore,
{
    state
        .store
        .record_audit_event(EvidenceRecordInput {
            context,
            event_type: event_type.to_string(),
            result,
            metadata_json,
        })
        .await
        .map_err(ApiError::internal)
}

async fn record_context_pack_file_audit<S>(
    state: &AppState<S>,
    trace: &TraceContext,
    principal: &crate::storage::AuthPrincipal,
    event_type: &str,
    file: &ContextDocumentRecord,
) -> Result<(), ApiError>
where
    S: EnterpriseStore,
{
    record_audit(
        state,
        EvidenceRecordContext::new(trace).actor(principal.user_id.clone()),
        event_type,
        TraceResult::Completed,
        serde_json::json!({
            "pack_id": file.pack_id,
            "document_id": file.document_id,
            "relative_path": file.relative_path,
            "file_kind": file.file_kind,
            "source_type": file.source_type,
            "loadable": file.loadable,
            "is_system_file": file.is_system_file,
            "file_size_bytes": file.file_size_bytes,
            "content_hash": file.content_hash,
            "deleted": file.deleted_at.is_some(),
        }),
    )
    .await
}

async fn record_execution_receipt<S>(
    state: &AppState<S>,
    context: EvidenceRecordContext,
    event_type: &str,
    result: TraceResult,
    metadata_json: serde_json::Value,
) -> Result<(), ApiError>
where
    S: EnterpriseStore,
{
    state
        .store
        .record_execution_receipt(EvidenceRecordInput {
            context,
            event_type: event_type.to_string(),
            result,
            metadata_json,
        })
        .await
        .map_err(ApiError::internal)
}

async fn proxy_worker_rpc(client_socket: WebSocket, socket_path: String) -> anyhow::Result<()> {
    let worker_stream = UnixStream::connect(FsPath::new(&socket_path)).await?;
    let (worker_socket, _response) = client_async("ws://localhost/rpc", worker_stream).await?;
    let (mut client_writer, mut client_reader) = client_socket.split();
    let (mut worker_writer, mut worker_reader) = worker_socket.split();

    let client_to_worker = async {
        while let Some(message) = client_reader.next().await {
            let message = message?;
            match axum_to_tungstenite(message) {
                Some(message) => worker_writer.send(message).await?,
                None => break,
            }
        }
        Ok::<(), anyhow::Error>(())
    };

    let worker_to_client = async {
        while let Some(message) = worker_reader.next().await {
            let message = message?;
            match tungstenite_to_axum(message) {
                Some(message) => client_writer.send(message).await?,
                None => break,
            }
        }
        Ok::<(), anyhow::Error>(())
    };

    tokio::select! {
        result = client_to_worker => {
            result.context("client to worker websocket proxy closed")?;
        },
        result = worker_to_client => {
            result.context("worker to client websocket proxy closed")?;
        },
    }
    Ok(())
}

fn axum_to_tungstenite(message: AxumWsMessage) -> Option<TungsteniteMessage> {
    match message {
        AxumWsMessage::Text(text) => Some(TungsteniteMessage::Text(text.to_string().into())),
        AxumWsMessage::Binary(binary) => Some(TungsteniteMessage::Binary(binary.to_vec().into())),
        AxumWsMessage::Ping(payload) => Some(TungsteniteMessage::Ping(payload.to_vec().into())),
        AxumWsMessage::Pong(payload) => Some(TungsteniteMessage::Pong(payload.to_vec().into())),
        AxumWsMessage::Close(_) => Some(TungsteniteMessage::Close(None)),
    }
}

fn tungstenite_to_axum(message: TungsteniteMessage) -> Option<AxumWsMessage> {
    match message {
        TungsteniteMessage::Text(text) => Some(AxumWsMessage::Text(text.to_string().into())),
        TungsteniteMessage::Binary(binary) => Some(AxumWsMessage::Binary(binary.to_vec().into())),
        TungsteniteMessage::Ping(payload) => Some(AxumWsMessage::Ping(payload.to_vec().into())),
        TungsteniteMessage::Pong(payload) => Some(AxumWsMessage::Pong(payload.to_vec().into())),
        TungsteniteMessage::Close(_) => Some(AxumWsMessage::Close(None)),
        TungsteniteMessage::Frame(_) => None,
    }
}

async fn authorize<S>(
    state: &AppState<S>,
    trace: &TraceContext,
    principal: &crate::storage::AuthPrincipal,
    action: EnterpriseAction,
) -> Result<(), ApiError>
where
    S: EnterpriseStore,
{
    let allowed = rbac::casbin_role_allows(principal.role, action)
        .await
        .map_err(ApiError::internal)?;
    if !allowed {
        record_audit(
            state,
            EvidenceRecordContext::new(trace).actor(principal.user_id.clone()),
            "rbac.deny",
            TraceResult::Denied,
            serde_json::json!({
                "role": principal.role.as_str(),
                "action": format!("{action:?}"),
            }),
        )
        .await?;
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "principal is not authorized for this enterprise action",
        ));
    }
    Ok(())
}

async fn authorize_any<S>(
    state: &AppState<S>,
    trace: &TraceContext,
    principal: &crate::storage::AuthPrincipal,
    actions: &[EnterpriseAction],
) -> Result<(), ApiError>
where
    S: EnterpriseStore,
{
    for action in actions {
        let allowed = rbac::casbin_role_allows(principal.role, *action)
            .await
            .map_err(ApiError::internal)?;
        if allowed {
            return Ok(());
        }
    }
    record_audit(
        state,
        EvidenceRecordContext::new(trace).actor(principal.user_id.clone()),
        "rbac.deny",
        TraceResult::Denied,
        serde_json::json!({
            "role": principal.role.as_str(),
            "actions": actions.iter().map(|action| format!("{action:?}")).collect::<Vec<_>>(),
        }),
    )
    .await?;
    Err(ApiError::new(
        StatusCode::FORBIDDEN,
        "principal is not authorized for this enterprise action",
    ))
}

async fn page<S>(
    State(state): State<AppState<S>>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, ApiError>
where
    S: EnterpriseStore,
{
    let bootstrapped = state
        .store
        .is_bootstrapped()
        .await
        .map_err(ApiError::internal)?;
    let path = uri.path();
    let principal = if bootstrapped {
        authenticate(&state, &headers).await.ok()
    } else {
        None
    };
    let authenticated = principal.is_some();

    if path == "/" {
        return Ok(Redirect::to(if bootstrapped {
            if authenticated { "/chat" } else { "/login" }
        } else {
            "/setup"
        })
        .into_response());
    }
    if !bootstrapped && path != "/setup" {
        return Ok(Redirect::to("/setup").into_response());
    }
    if bootstrapped && path == "/setup" {
        return Ok(Redirect::to("/login").into_response());
    }
    if bootstrapped && path == "/login" && authenticated {
        return Ok(Redirect::to("/chat").into_response());
    }
    if bootstrapped && is_protected_page(path) && !authenticated {
        return Ok(Redirect::to("/login").into_response());
    }
    if path == "/admin" || path.starts_with("/admin/") {
        let role = principal
            .as_ref()
            .map(|principal| principal.role)
            .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "not signed in"))?;
        if !admin_page_allowed(role, path) {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "principal is not authorized for this admin page",
            ));
        }
    }

    let (title, content) = match path {
        "/setup" => ("Bootstrap Admin", setup_page(&state.config)),
        "/login" => ("Sign In", login_page().to_string()),
        "/admin" => (
            "Admin",
            admin_overview_page(principal.as_ref().map(|principal| principal.role)),
        ),
        "/admin/users" => ("Users", users_page()),
        "/admin/users/new" => ("Create User", user_create_page()),
        "/admin/users/status" => ("User Status", user_status_page()),
        "/admin/users/projects" => ("User Projects", user_projects_page()),
        "/admin/users/context-packs" => ("User Context Packs", user_context_packs_page()),
        "/admin/rbac" => ("RBAC", rbac_page()),
        "/admin/workspaces" => ("Workspaces", workspaces_page()),
        "/admin/workspaces/register" => {
            ("Register Workspace", workspace_register_page(&state.config))
        }
        "/admin/workspaces/validate" => ("Validate Workspace", workspace_validate_page()),
        "/admin/context-packs" => ("Context Packs", context_packs_page()),
        "/admin/context-packs/new" => ("Create Context Pack", context_pack_create_page()),
        "/admin/context-packs/import" => ("Import Context Pack", context_pack_import_page()),
        "/admin/context-packs/files" => ("Context Pack Files", context_pack_files_page()),
        "/admin/context-packs/assignments" => {
            ("Context Pack Assignments", context_pack_assignments_page())
        }
        "/admin/outputs" => (
            "Reports And Outputs",
            admin_console_page_for(
                principal.as_ref().map(|principal| principal.role),
                outputs_admin_page(),
            ),
        ),
        "/chat" => ("Chat", chat_page(&state.config, principal.as_ref())),
        "/app" => ("Developer App", app_page(&state.config)),
        "/app/terminal" => ("Terminal", terminal_page(&state.config)),
        path if path.starts_with("/app/sessions/") => ("Session", session_page().to_string()),
        "/app/outputs" => ("My Outputs", outputs_page().to_string()),
        "/admin/audit" => (
            "Audit",
            admin_console_page_for(
                principal.as_ref().map(|principal| principal.role),
                audit_page(),
            ),
        ),
        _ => (
            "Local Codex for Enterprise",
            admin_overview_page(principal.as_ref().map(|principal| principal.role)),
        ),
    };

    Ok(Html(render_shell(
        title,
        bootstrapped,
        principal.as_ref(),
        &content,
    ))
    .into_response())
}

fn is_protected_page(path: &str) -> bool {
    path == "/chat"
        || path == "/admin"
        || path.starts_with("/admin/")
        || path == "/app"
        || path.starts_with("/app/")
}

fn admin_page_allowed(role: EnterpriseRole, path: &str) -> bool {
    if matches!(role, EnterpriseRole::Admin | EnterpriseRole::Owner) {
        return true;
    }
    matches!(
        (role, path),
        (EnterpriseRole::Manager, "/admin")
            | (EnterpriseRole::Manager, "/admin/outputs")
            | (EnterpriseRole::Manager, "/admin/audit")
    )
}

fn role_has_admin_console(role: EnterpriseRole) -> bool {
    matches!(
        role,
        EnterpriseRole::Admin | EnterpriseRole::Owner | EnterpriseRole::Manager
    )
}

fn render_shell(
    title: &str,
    bootstrapped: bool,
    principal: Option<&crate::storage::AuthPrincipal>,
    content: &str,
) -> String {
    let nav = if bootstrapped {
        if let Some(principal) = principal {
            let admin_link = if role_has_admin_console(principal.role) {
                let label = if matches!(principal.role, EnterpriseRole::Manager) {
                    "Manager Console"
                } else {
                    "Admin"
                };
                format!(r#"<a href="/admin">{label}</a>"#)
            } else {
                String::new()
            };
            format!(
                r#"
      <a href="/chat">Chat</a>
      <a href="/app/outputs">Outputs</a>
      <a href="/app/terminal">Terminal</a>
      {admin_link}
      <details class="account-menu">
        <summary>{} ({})</summary>
        <a href="/chat">Chat</a>
        <a href="/app/outputs">Outputs</a>
        <a href="/app/terminal">Terminal instructions</a>
        {}
      </details>"#,
                html_escape(&principal.email),
                principal.role.as_str(),
                admin_link
            )
        } else {
            r#"
      <a href="/login">Login</a>"#
                .to_string()
        }
    } else if bootstrapped {
        r#"
      <a href="/login">Login</a>"#
            .to_string()
    } else {
        r#"
      <a href="/setup">Setup</a>"#
            .to_string()
    };
    let intro = if content.contains("chat-shell-fullscreen") {
        ""
    } else {
        r#"
    <section id="page-intro">
      <h2>__TITLE__</h2>
      <p>Self-hosted control plane for governed local Codex sessions, workspace access, context loading, worker lifecycle, and trace receipts.</p>
    </section>"#
    };
    page_template()
        .replace("__TITLE__", title)
        .replace("__NAV__", &nav)
        .replace("__INTRO_SECTION__", intro)
        .replace("__CONTENT__", content)
        .replace("__VERSION__", env!("CARGO_PKG_VERSION"))
}

fn page_template() -> &'static str {
    r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>__TITLE__ - Local Codex for Enterprise</title>
  <style>
    body { margin: 0; font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; background: #f7f8fa; color: #17202a; }
    header { background: #102033; color: white; padding: 16px 24px; }
    nav { display: flex; flex-wrap: wrap; gap: 10px; margin-top: 12px; }
    nav a { color: #d7ecff; text-decoration: none; font-size: 14px; }
    .account-menu { margin-left: auto; position: relative; }
    .account-menu summary { cursor: pointer; color: #d7ecff; font-size: 14px; list-style: none; }
    .account-menu summary::-webkit-details-marker { display: none; }
    .account-menu[open] { z-index: 5; }
    .account-menu[open] summary { color: white; }
    .account-menu a { display: block; color: #102033; padding: 8px 10px; border-radius: 6px; }
    .account-menu a:hover { background: #edf3ff; }
    .account-menu[open]::before { content: ""; position: absolute; right: 0; top: 24px; width: 220px; min-height: 120px; background: white; border: 1px solid #dce3ea; border-radius: 8px; box-shadow: 0 16px 40px rgba(0,0,0,.18); }
    .account-menu[open] a { position: relative; width: 200px; margin-left: auto; }
    main { max-width: 1100px; margin: 0 auto; padding: 24px; }
    section { background: white; border: 1px solid #dce3ea; border-radius: 8px; padding: 18px; margin-bottom: 16px; }
    h1 { margin: 0; font-size: 24px; letter-spacing: 0; }
    h2 { margin: 0 0 12px; font-size: 18px; }
    label { display: block; font-weight: 600; margin-top: 12px; }
    input, textarea, select { width: 100%; box-sizing: border-box; margin-top: 4px; padding: 9px; border: 1px solid #b8c4cf; border-radius: 6px; font: inherit; }
    button { margin-top: 12px; padding: 9px 12px; border: 0; border-radius: 6px; background: #1463ff; color: white; font-weight: 700; cursor: pointer; }
    button.secondary { background: #26384c; }
    button.danger { background: #b42318; }
    .actions { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; }
    .hint { color: #526070; font-size: 13px; line-height: 1.4; }
    pre { overflow: auto; background: #101820; color: #e8f1f2; padding: 12px; border-radius: 6px; }
    .grid { display: grid; gap: 16px; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); }
    .console-layout { display: grid; gap: 18px; grid-template-columns: 240px minmax(0, 1fr); align-items: start; }
    .tree-nav { position: sticky; top: 16px; }
    .tree-nav h3 { margin: 0 0 10px; font-size: 13px; text-transform: uppercase; letter-spacing: 0; color: #526070; }
    .tree-group { border-top: 1px solid #dce3ea; padding-top: 10px; margin-top: 10px; }
    .tree-group strong { display: block; margin-bottom: 6px; }
    .tree-nav a { display: block; padding: 6px 8px; border-radius: 6px; color: #102033; text-decoration: none; font-size: 14px; }
    .tree-nav a:hover { background: #edf3ff; }
    .resource-table { width: 100%; border-collapse: collapse; font-size: 14px; }
    .resource-table th, .resource-table td { border-bottom: 1px solid #e4e9ef; padding: 8px; text-align: left; vertical-align: top; }
    .resource-table th { color: #526070; font-size: 12px; text-transform: uppercase; letter-spacing: 0; }
    .toolbar { display: flex; flex-wrap: wrap; gap: 10px; margin-bottom: 12px; }
    .toolbar a { background: #1463ff; color: white; border-radius: 6px; padding: 9px 12px; text-decoration: none; font-weight: 700; font-size: 14px; }
    .toolbar a.secondary { background: #26384c; }
    .muted { color: #526070; }
    .empty-state { background: #fff8e6; border: 1px solid #f4d27a; border-left: 4px solid #c78600; border-radius: 6px; padding: 12px; margin: 12px 0; }
    .empty-state a { color: #0f55cc; font-weight: 700; }
    .empty-state[hidden] { display: none; }
    button:disabled, select:disabled { opacity: 0.55; cursor: not-allowed; }
    select[multiple] { min-height: 142px; }
    .workbench-shell { display: grid; grid-template-columns: 260px minmax(0, 1fr); gap: 0; min-height: 680px; background: #0c1017; border: 1px solid #1f2937; border-radius: 8px; overflow: hidden; color: #e5edf7; }
    .workbench-sidebar { background: #111827; border-right: 1px solid #263244; padding: 16px; display: flex; flex-direction: column; gap: 16px; }
    .workbench-brand { font-size: 13px; font-weight: 800; color: #98f5b0; text-transform: uppercase; letter-spacing: 0; }
    .workbench-sidebar label, .workbench-composer label { color: #dce8f7; }
    .workbench-sidebar input, .workbench-sidebar select, .workbench-composer textarea { background: #0b1220; color: #f8fbff; border-color: #314155; }
    .workbench-main { display: grid; grid-template-rows: auto minmax(0, 1fr) auto; min-width: 0; }
    .workbench-topbar { display: flex; justify-content: space-between; align-items: center; gap: 16px; padding: 14px 18px; border-bottom: 1px solid #263244; background: #0f1724; }
    .workbench-title { margin: 0; font-size: 16px; }
    .workbench-status { display: inline-flex; align-items: center; gap: 8px; color: #a7b4c6; font-size: 13px; }
    .workbench-dot { width: 9px; height: 9px; border-radius: 999px; background: #f2b84b; box-shadow: 0 0 0 3px rgba(242,184,75,.14); }
    .workbench-dot.connected { background: #39d98a; box-shadow: 0 0 0 3px rgba(57,217,138,.16); }
    .workbench-chat { padding: 22px; overflow: auto; display: flex; flex-direction: column; gap: 14px; }
    .thread-list { display: flex; flex-direction: column; gap: 8px; }
    .thread-row { width: 100%; text-align: left; background: #172235; border: 1px solid #2a3950; color: #dce8f7; padding: 9px 10px; border-radius: 6px; }
    .thread-row.active { background: #17345e; border-color: #3c77c4; }
    .message { max-width: 820px; border: 1px solid #263244; border-radius: 8px; padding: 14px 16px; line-height: 1.5; }
    .message.system { background: #101928; color: #dce8f7; }
    .message.user { align-self: flex-end; background: #17345e; border-color: #23508e; color: #f8fbff; }
    .message.assistant { background: #0f1f19; border-color: #1d5a3e; color: #eafff2; }
    .message small { display: block; color: #91a1b6; font-weight: 800; margin-bottom: 6px; text-transform: uppercase; font-size: 11px; }
    .workbench-composer { border-top: 1px solid #263244; background: #0f1724; padding: 16px 18px; }
    .composer-row { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 12px; align-items: end; }
    .composer-input { min-height: 88px; resize: vertical; }
    .terminal-instructions { display: grid; grid-template-columns: minmax(0, 1fr) minmax(280px, .7fr); gap: 16px; }
    footer { max-width: 1100px; margin: 0 auto; padding: 0 24px 24px; color: #526070; font-size: 13px; }
    .footer-inner { border-top: 1px solid #dce3ea; display: flex; justify-content: space-between; gap: 16px; padding-top: 14px; }
    @media (max-width: 900px) { .workbench-shell, .terminal-instructions { grid-template-columns: 1fr; } .workbench-sidebar { border-right: 0; border-bottom: 1px solid #263244; } }
    @media (max-width: 820px) { .console-layout { grid-template-columns: 1fr; } .tree-nav { position: static; } }
    @media (max-width: 640px) { .footer-inner { flex-direction: column; } }
  </style>
</head>
<body>
  <header>
    <h1>Local Codex for Enterprise</h1>
    <nav>
__NAV__
    </nav>
  </header>
  <main>
__INTRO_SECTION__
__CONTENT__
    <section id="result-panel">
      <h2>Result</h2>
      <pre id="result">Ready.</pre>
    </section>
  </main>
  <footer>
    <div class="footer-inner">
      <span>Local Codex for Enterprise v__VERSION__</span>
      <span>Made with Codex</span>
    </div>
  </footer>
  <script>
    function v(id) { return document.getElementById(id).value; }
    function lines(id) { return v(id).split('\n').map((line) => line.trim()).filter(Boolean); }
    async function postJson(url, body, redirectTo) {
      const headers = {'content-type':'application/json'};
      const response = await fetch(url, {method:'POST', headers, credentials:'same-origin', body: JSON.stringify(body)});
      const text = await response.text();
      try {
        const json = JSON.parse(text);
        document.getElementById('result').textContent = JSON.stringify(json, null, 2);
      } catch (_) {
        document.getElementById('result').textContent = text;
      }
      if (response.ok && redirectTo) {
        window.location.assign(redirectTo);
      }
    }
    async function getJson(url) {
      const response = await fetch(url, {credentials:'same-origin'});
      const text = await response.text();
      try {
        document.getElementById('result').textContent = JSON.stringify(JSON.parse(text), null, 2);
      } catch (_) {
        document.getElementById('result').textContent = text;
      }
    }
    async function fetchJson(url) {
      const response = await fetch(url, {credentials:'same-origin'});
      if (!response.ok) {
        throw new Error(await response.text());
      }
      return await response.json();
    }
    async function deleteJson(url) {
      const response = await fetch(url, {method:'DELETE', credentials:'same-origin'});
      const text = await response.text();
      document.getElementById('result').textContent = text;
      await refreshAdminChoices();
    }
    async function patchJson(url, body) {
      const response = await fetch(url, {method:'PATCH', headers:{'content-type':'application/json'}, credentials:'same-origin', body: JSON.stringify(body)});
      const text = await response.text();
      document.getElementById('result').textContent = text;
      await refreshAdminChoices();
    }
    function selectedValues(id) {
      const element = document.getElementById(id);
      if (!element) return [];
      return Array.from(element.selectedOptions).map((option) => option.value).filter(Boolean);
    }
    function fillSelect(id, items, label, value, emptyLabel) {
      const element = document.getElementById(id);
      if (!element) return;
      element.innerHTML = '';
      element.disabled = items.length === 0;
      if (!items.length && emptyLabel) {
        const option = document.createElement('option');
        option.textContent = emptyLabel;
        option.disabled = true;
        option.selected = true;
        element.appendChild(option);
        return;
      }
      for (const item of items) {
        const option = document.createElement('option');
        option.value = value(item);
        option.textContent = label(item);
        element.appendChild(option);
      }
    }
    function emptyStateHtml(message, href, label) {
      const escape = (value) => String(value ?? '').replace(/[&<>"']/g, (char) => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[char]));
      const action = href && label ? ' <a href="'+escape(href)+'">'+escape(label)+'</a>' : '';
      return '<div class="empty-state"><strong>Nothing here yet.</strong><p>'+escape(message)+action+'</p></div>';
    }
    function setEmptyState(id, isEmpty, message, href, label) {
      const target = document.getElementById(id);
      if (!target) return;
      target.hidden = !isEmpty;
      target.innerHTML = isEmpty ? emptyStateHtml(message, href, label) : '';
    }
    function setButtonDisabled(id, disabled) {
      const button = document.getElementById(id);
      if (button) button.disabled = disabled;
    }
    function renderTable(id, headers, rows, emptyState) {
      const target = document.getElementById(id);
      if (!target) return;
      if (!rows.length) {
        target.innerHTML = emptyState ? emptyStateHtml(emptyState.message, emptyState.href, emptyState.label) : '<p class="hint">No records yet.</p>';
        return;
      }
      const escape = (value) => String(value ?? '').replace(/[&<>"']/g, (char) => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[char]));
      const head = headers.map((header) => '<th>'+escape(header.label)+'</th>').join('');
      const body = rows.map((row) => '<tr>'+headers.map((header) => '<td>'+escape(header.value(row))+'</td>').join('')+'</tr>').join('');
      target.innerHTML = '<table class="resource-table"><thead><tr>'+head+'</tr></thead><tbody>'+body+'</tbody></table>';
    }
    let adminUsers = [];
    let adminUserWorkspaces = [];
    let adminProjects = [];
    function renderUserProjectControls() {
      const userSelect = document.getElementById('project-user-select');
      if (!userSelect) return;
      const userId = userSelect.value || (adminUsers[0] ? adminUsers[0].user_id : '');
      const userWorkspaces = adminUserWorkspaces.filter((workspace) => !userId || workspace.owner_user_id === userId);
      const projects = adminProjects.filter((project) => !userId || project.owner_user_id === userId);
      const activeProjects = projects.filter((project) => !project.deleted_at);
      const archivedProjects = projects.filter((project) => project.deleted_at);
      fillSelect('project-user-workspace-select', userWorkspaces, (workspace) => workspace.path, (workspace) => workspace.user_workspace_id, 'No user workspaces available');
      fillSelect('project-edit-select', activeProjects, (project) => project.name, (project) => project.project_id, 'No active projects available');
      fillSelect('project-restore-select', archivedProjects, (project) => project.name, (project) => project.project_id, 'No archived projects available');
      renderTable('project-index', [
        {label:'Name', value:(project) => project.name},
        {label:'Owner', value:(project) => (adminUsers.find((user) => user.user_id === project.owner_user_id)?.email || project.owner_user_id)},
        {label:'Path', value:(project) => project.project_path},
        {label:'Repositories', value:(project) => (project.repositories || []).length},
        {label:'Threads', value:(project) => (project.threads || []).length}
      ], activeProjects, {message:'No active projects exist for the selected user yet. Choose a user workspace and create a project before starting project threads.', href:null, label:null});
      renderTable('project-archive-index', [
        {label:'Name', value:(project) => project.name},
        {label:'Owner', value:(project) => (adminUsers.find((user) => user.user_id === project.owner_user_id)?.email || project.owner_user_id)},
        {label:'Path', value:(project) => project.project_path},
        {label:'Archived', value:(project) => project.deleted_at || ''}
      ], archivedProjects, {message:'No archived projects exist for the selected user.', href:null, label:null});
      setEmptyState('project-workspace-empty-state', userWorkspaces.length === 0, 'This user has no workspace yet. Create or assign a user workspace before creating projects.', '/admin/users/new', 'Create user workspace');
      setEmptyState('project-empty-state', activeProjects.length === 0, 'No active projects exist for the selected user yet. Create one from this page.', null, null);
      setButtonDisabled('project-create-submit', userWorkspaces.length === 0);
      setButtonDisabled('project-rename-submit', activeProjects.length === 0);
      setButtonDisabled('project-delete-submit', activeProjects.length === 0);
      setButtonDisabled('project-restore-submit', archivedProjects.length === 0);
    }
    async function refreshProjectsForSelectedUser() {
      const userId = v('project-user-select');
      const payload = await fetchJson('/v1/projects?user_id='+encodeURIComponent(userId)+'&include_deleted=true').catch(() => ({projects: []}));
      adminProjects = payload.projects || [];
      renderUserProjectControls();
    }
    function renderAssignments(id, assignments, packs, users) {
      const target = document.getElementById(id);
      if (!target) return;
      const packName = new Map(packs.map((pack) => [pack.pack_id, pack.name]));
      const userName = new Map(users.map((user) => [user.user_id, user.email]));
      if (!assignments.length) {
        target.innerHTML = '<p class="hint">No context pack assignments yet.</p>';
        return;
      }
      const escape = (value) => String(value ?? '').replace(/[&<>"']/g, (char) => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[char]));
      target.innerHTML = '<table class="resource-table"><thead><tr><th>Pack</th><th>User</th><th>Workspace</th><th>Order</th><th>Source</th><th>Action</th></tr></thead><tbody>'+
        assignments.map((assignment) => '<tr><td>'+escape(packName.get(assignment.pack_id) || assignment.pack_id)+'</td><td>'+escape(userName.get(assignment.user_id) || 'All users')+'</td><td>'+escape(assignment.workspace_id || 'All workspaces')+'</td><td>'+escape(assignment.assignment_order)+'</td><td>'+escape(assignment.assignment_source)+'</td><td><button class="danger" onclick="deleteJson(&quot;/v1/context-pack-assignments/'+escape(assignment.assignment_id)+'&quot;)">Remove Assignment</button></td></tr>').join('')+
        '</tbody></table>';
    }
    async function base64FromFile(file) {
      const bytes = new Uint8Array(await file.arrayBuffer());
      let binary = '';
      const chunkSize = 8192;
      for (let index = 0; index < bytes.length; index += chunkSize) {
        binary += String.fromCharCode(...bytes.subarray(index, index + chunkSize));
      }
      return btoa(binary);
    }
    async function refreshContextPackFiles() {
      const packId = v('pack-file-pack-select');
      const target = document.getElementById('context-pack-file-index');
      if (!target) return;
      if (!packId) {
        target.innerHTML = emptyStateHtml('No context pack is selected. Create a Context Pack before registering package files.', '/admin/context-packs/new', 'Create Context Pack');
        return;
      }
      const payload = await fetchJson('/v1/context-packs/'+encodeURIComponent(packId)+'/files').catch((error) => ({files: [], error: error.message}));
      const files = payload.files || [];
      if (!files.length) {
        target.innerHTML = emptyStateHtml('No files are registered for this Context Pack yet. Upload package files or a bundle folder to make the pack portable.', null, null);
        return;
      }
      const escape = (value) => String(value ?? '').replace(/[&<>"']/g, (char) => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[char]));
      target.innerHTML = '<table class="resource-table"><thead><tr><th>Path</th><th>Kind</th><th>Source</th><th>Loadable</th><th>System</th><th>Size</th><th>Hash</th><th>Actions</th></tr></thead><tbody>'+
        files.map((file) => '<tr><td>'+escape(file.relative_path)+'</td><td>'+escape(file.file_kind)+'</td><td>'+escape(file.source_type)+'</td><td>'+escape(file.loadable)+'</td><td>'+escape(file.is_system_file ? 'system' : 'custom')+'</td><td>'+escape(file.file_size_bytes)+'</td><td><code>'+escape(file.content_hash)+'</code></td><td><a href="/v1/context-packs/'+escape(packId)+'/files/'+escape(file.document_id)+'/download">Download</a> <button onclick="renameContextPackFile(&quot;'+escape(packId)+'&quot;,&quot;'+escape(file.document_id)+'&quot;,&quot;'+escape(file.relative_path)+'&quot;)">Rename</button> <button onclick="toggleContextPackFileLoadable(&quot;'+escape(packId)+'&quot;,&quot;'+escape(file.document_id)+'&quot;,'+(!file.loadable)+')">'+(file.loadable ? 'Disable load' : 'Enable load')+'</button> <button class="danger" onclick="removeContextPackFile(&quot;'+escape(packId)+'&quot;,&quot;'+escape(file.document_id)+'&quot;)">Remove</button></td></tr>').join('')+
        '</tbody></table>';
    }
    async function uploadContextPackFiles() {
      const packId = v('pack-file-pack-select');
      const input = document.getElementById('pack-file-input');
      if (!packId || !input?.files?.length) {
        document.getElementById('result').textContent = 'Choose a context pack and at least one file.';
        return;
      }
      const kind = v('pack-file-kind');
      const source = v('pack-file-source') || 'upload';
      const forceLoadable = document.getElementById('pack-file-loadable')?.checked;
      let order = Number(v('pack-file-load-order') || 100);
      for (const file of input.files) {
        const relativePath = file.webkitRelativePath || file.name;
        const payload = {
          relative_path: relativePath,
          content_base64: await base64FromFile(file),
          content_type: file.type || undefined,
          source_type: source,
          load_order: order
        };
        if (kind) payload.file_kind = kind;
        if (forceLoadable) payload.loadable = true;
        await postJsonPayload('/v1/context-packs/'+encodeURIComponent(packId)+'/files', payload);
        order += 1;
      }
      await refreshContextPackFiles();
    }
    async function renameContextPackFile(packId, documentId, currentPath) {
      const nextPath = prompt('New relative path', currentPath);
      if (!nextPath || nextPath === currentPath) return;
      await patchJsonPayload('/v1/context-packs/'+encodeURIComponent(packId)+'/files/'+encodeURIComponent(documentId), {relative_path: nextPath});
      await refreshContextPackFiles();
    }
    async function toggleContextPackFileLoadable(packId, documentId, loadable) {
      await patchJsonPayload('/v1/context-packs/'+encodeURIComponent(packId)+'/files/'+encodeURIComponent(documentId), {loadable});
      await refreshContextPackFiles();
    }
    async function removeContextPackFile(packId, documentId) {
      if (!confirm('Remove this file from future context loads? Historical receipts remain queryable.')) return;
      await deleteJson('/v1/context-packs/'+encodeURIComponent(packId)+'/files/'+encodeURIComponent(documentId));
      await refreshContextPackFiles();
    }
    async function refreshAdminChoices() {
      try {
        const [usersPayload, workspacesPayload, userWorkspacesPayload, projectsPayload, packsPayload, assignmentsPayload, outputsPayload] = await Promise.all([
          fetchJson('/v1/users').catch(() => ({users: []})),
          fetchJson('/v1/workspace-roots').catch(() => ({workspaces: []})),
          fetchJson('/v1/user-workspaces').catch(() => ({user_workspaces: []})),
          fetchJson('/v1/projects?include_deleted=true').catch(() => ({projects: []})),
          fetchJson('/v1/context-packs').catch(() => ({packs: []})),
          fetchJson('/v1/context-pack-assignments').catch(() => ({assignments: []})),
          fetchJson('/v1/outputs').catch(() => ({outputs: []}))
        ]);
        const users = usersPayload.users || [];
        const workspaces = workspacesPayload.workspaces || [];
        const userWorkspaces = userWorkspacesPayload.user_workspaces || [];
        const projects = projectsPayload.projects || [];
        const packs = packsPayload.packs || [];
        const assignments = assignmentsPayload.assignments || [];
        const outputs = outputsPayload.outputs || [];
        adminUsers = users;
        adminUserWorkspaces = userWorkspaces;
        adminProjects = projects;
        renderTable('user-index', [
          {label:'Email', value:(user) => user.email},
          {label:'Role', value:(user) => user.role},
          {label:'Status', value:(user) => user.status}
        ], users, {message:'No users exist yet. Bootstrap creates the first admin; add additional accounts before assigning roles or context.', href:'/admin/users/new', label:'Add account'});
        renderTable('workspace-index', [
          {label:'Root path', value:(workspace) => workspace.root_path},
          {label:'Created', value:(workspace) => workspace.created_at}
        ], workspaces, {message:'No registered workspaces are available. Add a server-visible workspace root before creating sessions or assigning workspace-scoped context.', href:'/admin/workspaces/register', label:'Add workspace root'});
        renderTable('context-pack-index', [
          {label:'Name', value:(pack) => pack.name},
          {label:'Status', value:(pack) => pack.status},
          {label:'Created', value:(pack) => pack.created_at}
        ], packs, {message:'No context packs exist yet. Add or import a Context Pack before assigning governed operating packages.', href:'/admin/context-packs/new', label:'Add Context Pack'});
        renderAssignments('assignment-index', assignments, packs, users);
        renderTable('output-index', [
          {label:'Category', value:(output) => output.category},
          {label:'Type', value:(output) => output.output_type},
          {label:'Title', value:(output) => output.title},
          {label:'Location', value:(output) => output.artifact_path},
          {label:'Status', value:(output) => output.status}
        ], outputs, {message:'No output metadata exists for your user yet. End outputs are deliverable reports or code-change artifacts; operational outputs are roadmap, bug, error, and workstatus records.', href:null, label:null});
        fillSelect('status-user-select', users, (user) => user.email+' ('+user.status+')', (user) => user.user_id, 'No users available');
        fillSelect('role-user-select', users, (user) => user.email+' ('+user.role+')', (user) => user.user_id, 'No users available');
        fillSelect('project-user-select', users, (user) => user.email+' ('+user.role+')', (user) => user.user_id, 'No users available');
        fillSelect('user-assignment-user-select', users, (user) => user.email, (user) => user.user_id, 'No users available');
        fillSelect('user-pack-select', packs, (pack) => pack.name, (pack) => pack.pack_id, 'No context packs available');
        fillSelect('pack-assignment-pack-select', packs, (pack) => pack.name, (pack) => pack.pack_id, 'No context packs available');
        fillSelect('pack-file-pack-select', packs, (pack) => pack.name, (pack) => pack.pack_id, 'No context packs available');
        fillSelect('pack-user-select', users, (user) => user.email, (user) => user.user_id, 'No users available');
        fillSelect('output-user-select', users, (user) => user.email+' ('+user.role+')', (user) => user.user_id, 'No users available');
        fillSelect('user-workspace-select', workspaces, (workspace) => workspace.root_path, (workspace) => workspace.root_path, 'No registered workspaces available');
        fillSelect('pack-workspace-select', workspaces, (workspace) => workspace.root_path, (workspace) => workspace.root_path, 'No registered workspaces available');
        setEmptyState('user-empty-state', users.length === 0, 'No users exist yet. Add a user before assigning roles, account status, or user-scoped context.', '/admin/users/new', 'Add user');
        setEmptyState('pack-empty-state', packs.length === 0, 'No context packs exist yet. Create a Context Pack before assigning it to a user.', '/admin/context-packs/new', 'Add Context Pack');
        setEmptyState('pack-file-empty-state', packs.length === 0, 'No context packs exist yet. Create or import a Context Pack before registering package files.', '/admin/context-packs/new', 'Add Context Pack');
        setEmptyState('context-pack-empty-state', packs.length === 0, 'No context packs exist yet. Create a Context Pack before assigning it.', '/admin/context-packs/new', 'Add Context Pack');
        setEmptyState('workspace-empty-state', workspaces.length === 0, 'No registered workspaces are available. Add a workspace before assigning workspace-scoped context.', '/admin/workspaces/register', 'Add workspace');
        setEmptyState('pack-workspace-empty-state', workspaces.length === 0, 'No registered workspaces are available. Add a workspace before assigning this context pack.', '/admin/workspaces/register', 'Add workspace');
        let disabled = packs.length === 0 || users.length === 0 || workspaces.length === 0;
        setButtonDisabled('user-pack-assign-submit', disabled);
        disabled = packs.length === 0 || workspaces.length === 0;
        setButtonDisabled('pack-assign-submit', disabled);
        setButtonDisabled('pack-file-upload-submit', packs.length === 0);
        setButtonDisabled('role-assign-submit', users.length === 0);
        setButtonDisabled('role-remove-submit', users.length === 0);
        setButtonDisabled('output-create-submit', users.length === 0);
        setButtonDisabled('user-status-deactivate-submit', users.length === 0);
        setButtonDisabled('user-status-reactivate-submit', users.length === 0);
        renderUserProjectControls();
        if (document.getElementById('context-pack-file-index')) {
          await refreshContextPackFiles();
        }
      } catch (error) {
        document.getElementById('result').textContent = error.message;
      }
    }
    async function createProjectForSelectedUser() {
      const userWorkspaceId = v('project-user-workspace-select');
      const name = v('project-name').trim();
      if (!userWorkspaceId || !name) {
        document.getElementById('result').textContent = 'Choose a user workspace and enter a project name.';
        return;
      }
      await postJson('/v1/user-workspaces/'+encodeURIComponent(userWorkspaceId)+'/projects', {name});
      await refreshAdminChoices();
    }
    async function renameSelectedProject() {
      const projectId = v('project-edit-select');
      const name = v('project-rename').trim();
      if (!projectId || !name) {
        document.getElementById('result').textContent = 'Choose a project and enter a new name.';
        return;
      }
      await patchJson('/v1/projects/'+encodeURIComponent(projectId), {name});
    }
    async function deleteSelectedProject() {
      const projectId = v('project-edit-select');
      if (!projectId) {
        document.getElementById('result').textContent = 'Choose a project to remove.';
        return;
      }
      await deleteJson('/v1/projects/'+encodeURIComponent(projectId));
    }
    async function restoreSelectedProject() {
      const projectId = v('project-restore-select');
      if (!projectId) {
        document.getElementById('result').textContent = 'Choose an archived project to restore.';
        return;
      }
      await postJson('/v1/projects/'+encodeURIComponent(projectId)+'/restorations', {});
      await refreshAdminChoices();
    }
    async function assignPacksFromUserPage() {
      if (document.getElementById('user-pack-assign-submit')?.disabled) {
        document.getElementById('result').textContent = 'Add users, context packs, and registered workspaces before assigning context.';
        return;
      }
      await postJson('/v1/context-pack-assignment-batches', {
        pack_ids: selectedValues('user-pack-select'),
        user_ids: selectedValues('user-assignment-user-select'),
        workspace_ids: selectedValues('user-workspace-select'),
        assignment_order: Number(v('user-assign-order')),
        required_session: true,
        required_worker: true
      });
      await refreshAdminChoices();
    }
    async function assignUsersFromPackPage() {
      if (document.getElementById('pack-assign-submit')?.disabled) {
        document.getElementById('result').textContent = 'Add a context pack and registered workspace before assigning context.';
        return;
      }
      await postJson('/v1/context-pack-assignment-batches', {
        pack_ids: selectedValues('pack-assignment-pack-select'),
        user_ids: selectedValues('pack-user-select'),
        workspace_ids: selectedValues('pack-workspace-select'),
        assignment_order: Number(v('pack-assign-order')),
        required_session: true,
        required_worker: true
      });
      await refreshAdminChoices();
    }
    function safeDocumentsFromTextareas() {
      return [
        {filename:'PACK.md', content:v('pack-manifest')},
        {filename:'CALIBRATION.md', content:v('pack-calibration')},
        {filename:'OPERATING-INSTRUCTIONS.md', content:v('pack-operating-instructions')},
        {filename:'PROJECT-RULES.md', content:v('pack-project-rules')},
        {filename:'WORKFLOWS.md', content:v('pack-workflows')},
        {filename:'HANDOFF.md', content:v('pack-handoff')},
        {filename:'VERIFICATION.md', content:v('pack-verification')},
        {filename:'ESCALATION.md', content:v('pack-escalation')},
        {filename:'CONTEXT.md', content:v('pack-context')},
        {filename:'PROMPTS.md', content:v('pack-prompts')}
      ];
    }
    function isContextPackMarkdownFile(fileName) {
      return /^[A-Z0-9][A-Z0-9._-]*\.md$/.test(fileName) && !fileName.startsWith('.');
    }
    async function createPackFromSelectedFiles() {
      const input = document.getElementById('pack-files');
      const documents = [];
      for (const file of input.files) {
        if (isContextPackMarkdownFile(file.name)) {
          documents.push({filename:file.name, content:await file.text()});
        }
      }
      if (!documents.some((document) => document.filename === 'PACK.md')) {
        document.getElementById('result').textContent = 'Select a context-pack folder or markdown files that include PACK.md.';
        return;
      }
      await postJson('/v1/context-packs',{name:v('pack-name'),documents});
    }
    async function postJsonPayload(url, body) {
      const response = await fetch(url, {method:'POST', headers:{'content-type':'application/json'}, credentials:'same-origin', body:JSON.stringify(body)});
      const text = await response.text();
      let json = {};
      try { json = text ? JSON.parse(text) : {}; } catch (_) { json = {body:text}; }
      if (!response.ok) {
        throw new Error(json.error || text || response.statusText);
      }
      const result = document.getElementById('result');
      if (result) result.textContent = JSON.stringify(json, null, 2);
      return json;
    }
    async function patchJsonPayload(url, body) {
      const response = await fetch(url, {method:'PATCH', headers:{'content-type':'application/json'}, credentials:'same-origin', body:JSON.stringify(body)});
      const text = await response.text();
      let json = {};
      try { json = text ? JSON.parse(text) : {}; } catch (_) { json = {body:text}; }
      if (!response.ok) {
        throw new Error(json.error || text || response.statusText);
      }
      const result = document.getElementById('result');
      if (result) result.textContent = JSON.stringify(json, null, 2);
      return json;
    }
    async function putJsonPayload(url, body) {
      const response = await fetch(url, {method:'PUT', headers:{'content-type':'application/json'}, credentials:'same-origin', body:JSON.stringify(body)});
      const text = await response.text();
      let json = {};
      try { json = text ? JSON.parse(text) : {}; } catch (_) { json = {body:text}; }
      if (!response.ok) {
        throw new Error(json.error || text || response.statusText);
      }
      const result = document.getElementById('result');
      if (result) result.textContent = JSON.stringify(json, null, 2);
      return json;
    }
    async function deleteJsonPayload(url) {
      const response = await fetch(url, {method:'DELETE', credentials:'same-origin'});
      const text = await response.text();
      let json = {};
      try { json = text ? JSON.parse(text) : {}; } catch (_) { json = {body:text}; }
      if (!response.ok) {
        throw new Error(json.error || text || response.statusText);
      }
      const result = document.getElementById('result');
      if (result) result.textContent = JSON.stringify(json, null, 2);
      return json;
    }
    class WorkbenchTranscript extends HTMLElement {
      constructor() {
        super();
        this.messages = [];
        this.editingIndex = null;
        this.maxRenderedMessages = 80;
        this.rowEstimate = 92;
        this.windowStart = 0;
        this.stickToBottom = true;
        this.attachShadow({mode:'open'});
      }
      connectedCallback() {
        this.shadowRoot.innerHTML = '<style>:host{display:block;height:100%;overflow:hidden;min-height:0;}#scroll{height:100%;overflow-y:auto;overflow-x:hidden;padding:28px max(22px,5vw) 24px;box-sizing:border-box;scrollbar-gutter:stable;}#top-spacer,#bottom-spacer{height:0;}#messages{display:flex;flex-direction:column;gap:22px;min-height:100%;}.message{max-width:min(var(--chat-content-width,980px),100%);border:0;border-radius:18px;padding:14px 16px;line-height:1.55;font-size:15px;box-sizing:border-box;position:relative;}.message.system,.message.assistant{background:transparent;color:#d7dce7;}.message.user{align-self:flex-end;background:#1f2027;color:#f5f6fa;}.message.pending div{color:#858b98;}.message small{display:block;color:#858b98;font-size:11px;font-weight:800;text-transform:uppercase;margin-bottom:6px;}.message-body{white-space:normal;}.message-body p{margin:0 0 10px;}.message-body p:last-child{margin-bottom:0;}.message-body h1,.message-body h2,.message-body h3{margin:16px 0 8px;color:#f4f6fb;line-height:1.25;}.message-body h1{font-size:22px;}.message-body h2{font-size:19px;}.message-body h3{font-size:16px;}.message-body ul,.message-body ol{margin:8px 0 12px;padding-left:22px;}.message-body li{margin:4px 0;}.message-body code{background:#151820;border:1px solid #2a2d37;border-radius:5px;padding:1px 5px;}.message-body pre{background:#0d0f14;border:1px solid #2a2d37;border-radius:10px;padding:12px;overflow:auto;white-space:pre-wrap;}.message-actions{display:flex;align-items:center;justify-content:flex-start;margin-top:12px;opacity:0;transition:opacity .14s ease;}.message:hover .message-actions,.message:focus-within .message-actions,.message.last-completed-assistant .message-actions{opacity:1;}.message-actions-left{display:inline-flex;align-items:center;gap:8px;color:#9aa3b2;}.message-timestamp{font-size:12px;color:#9aa3b2;}.copyable-message{appearance:none;border:0;background:transparent;color:#9aa3b2;padding:4px;border-radius:7px;display:inline-flex;align-items:center;justify-content:center;opacity:.84;cursor:pointer;}.copyable-message:hover,.copyable-message:focus{background:rgba(154,163,178,.12);color:#c9d1dd;opacity:1;outline:none;}.copyable-message.copied,.copyable-message.feedback-selected{color:#f5f6fa;opacity:1;}.copyable-message.feedback-selected{background:rgba(245,246,250,.10);}.copyable-message svg{width:16px;height:16px;}.inline-turn-editor{display:grid;gap:10px;min-width:min(520px,70vw);}.inline-turn-editor textarea{width:100%;min-height:96px;box-sizing:border-box;background:#0d0f14;color:#f5f6fa;border:1px solid #3a4151;border-radius:10px;padding:10px;font:inherit;resize:vertical;}.inline-turn-editor-actions{display:flex;gap:8px;justify-content:flex-end;}.inline-turn-editor-actions button{border:0;border-radius:9px;padding:8px 12px;font-weight:800;cursor:pointer;}.inline-turn-editor-actions .submit-edit{background:var(--lc-accent,#8bf5a2);color:var(--lc-accent-text,#061008);}.inline-turn-editor-actions .cancel-edit{background:#253044;color:#dce4f2;}.thinking-dots{display:inline-flex;align-items:center;gap:2px;color:#a3aab8;}.thinking-dots::after{content:\'...\';display:inline-block;min-width:22px;animation:thinkingPulse 1.1s steps(4,end) infinite;}@keyframes thinkingPulse{0%{content:\'\';}25%{content:\'.\';}50%{content:\'..\';}75%,100%{content:\'...\';}}@media (max-width:480px){#scroll{padding:18px 14px 18px;}.message{font-size:14px;padding:12px 13px;}}</style><div id="scroll"><div id="top-spacer"></div><div id="messages"></div><div id="bottom-spacer"></div></div>';
        this.scrollElement = this.shadowRoot.getElementById('scroll');
        this.messageElement = this.shadowRoot.getElementById('messages');
        this.topSpacerElement = this.shadowRoot.getElementById('top-spacer');
        this.bottomSpacerElement = this.shadowRoot.getElementById('bottom-spacer');
        this.shadowRoot.addEventListener('click', (event) => {
          const resubmitButton = event.target.closest('[data-resubmit-index]');
          if (resubmitButton) {
            this.resubmitWorkbenchMessage(Number(resubmitButton.dataset.resubmitIndex));
            return;
          }
          const beginEditButton = event.target.closest('[data-begin-edit-index]');
          if (beginEditButton) {
            this.beginEditResubmitWorkbenchMessage(Number(beginEditButton.dataset.beginEditIndex));
            return;
          }
          const submitEditButton = event.target.closest('[data-submit-edit-index]');
          if (submitEditButton) {
            this.submitEditedWorkbenchMessage(Number(submitEditButton.dataset.submitEditIndex));
            return;
          }
          const cancelEditButton = event.target.closest('[data-cancel-edit-index]');
          if (cancelEditButton) {
            this.editingIndex = null;
            this.renderAfterMessageChange();
            return;
          }
          const feedbackButton = event.target.closest('[data-feedback-index]');
          if (feedbackButton) {
            this.handleWorkbenchFeedback(Number(feedbackButton.dataset.feedbackIndex), feedbackButton.dataset.feedbackRating);
            return;
          }
          const saveOutputButton = event.target.closest('[data-save-output-index]');
          if (saveOutputButton) {
            this.saveWorkbenchMessageOutput(Number(saveOutputButton.dataset.saveOutputIndex));
            return;
          }
          const button = event.target.closest('[data-copy-index]');
          if (!button) return;
          this.copyWorkbenchMessage(Number(button.dataset.copyIndex), button);
        });
        this.scrollElement.addEventListener('scroll', () => {
          this.stickToBottom = this.isAtBottom();
          this.renderForScroll();
        });
        this.renderWindow(Math.max(0, this.messages.length - this.maxRenderedMessages));
      }
      isAtBottom() {
        return !this.scrollElement || this.scrollElement.scrollTop + this.scrollElement.clientHeight >= this.scrollElement.scrollHeight - 80;
      }
      scrollToBottom() {
        if (!this.scrollElement) return;
        this.scrollElement.scrollTop = this.scrollElement.scrollHeight;
        this.stickToBottom = true;
      }
      appendMessage(kind, label, text) {
        this.clearEphemeralMessages();
        this.messages.push({kind, label, text, createdAt:new Date().toISOString()});
        if (kind === 'user') this.stickToBottom = true;
        this.renderAfterMessageChange();
      }
      appendToLastMessage(kind, label, delta) {
        const last = this.messages[this.messages.length - 1];
        if (last && last.kind === kind && last.label === label) {
          last.text = String(last.text || '') + String(delta || '');
        } else {
          this.messages.push({kind, label, text: String(delta || ''), createdAt:new Date().toISOString()});
        }
        this.renderAfterMessageChange();
      }
      appendPendingMessage(kind, label, text) {
        const last = this.messages[this.messages.length - 1];
        if (last && last.pending && last.kind === kind && last.label === label) {
          last.text = text;
        } else {
          this.messages.push({kind, label, text, pending:true, streaming:kind === 'assistant', createdAt:new Date().toISOString()});
        }
        this.renderAfterMessageChange();
      }
      replacePendingMessage(kind, label, delta) {
        const last = this.messages[this.messages.length - 1];
        if (last && last.kind === kind && last.label === label && (last.pending || last.streaming)) {
          last.text = last.pending ? String(delta || '') : String(last.text || '') + String(delta || '');
          last.pending = false;
          last.streaming = true;
        } else {
          this.appendToLastMessage(kind, label, delta);
          return;
        }
        this.renderAfterMessageChange();
      }
      completeLastMessage(kind, label) {
        const last = [...this.messages].reverse().find((message) => message.kind === kind && message.label === label && (message.pending || message.streaming));
        if (last && last.kind === kind && last.label === label && (last.pending || last.streaming)) {
          last.pending = false;
          last.streaming = false;
          this.renderAfterMessageChange();
        }
      }
      attachLastPersistedMessage(kind, label, persisted) {
        if (!persisted) return;
        const last = [...this.messages].reverse().find((message) => message.kind === kind && message.label === label && !message.pending && !message.streaming && !message.ephemeral);
        if (!last) return;
        Object.assign(last, {
          message_id: persisted.message_id,
          session_id: persisted.session_id,
          feedbackRating: persisted.feedback_rating || persisted.feedbackRating || last.feedbackRating,
          createdAt: persisted.created_at || last.createdAt
        });
        this.renderAfterMessageChange();
      }
      setMessages(messages) {
        this.messages = Array.isArray(messages) ? messages.map((message) => Object.assign({}, message, {createdAt:message.created_at || message.createdAt || new Date().toISOString()})) : [];
        this.renderWindow(Math.max(0, this.messages.length - this.maxRenderedMessages));
        requestAnimationFrame(() => this.scrollToBottom());
      }
      setEmptyMessage(kind, label, text) {
        this.setMessages([{kind, label, text, ephemeral:true}]);
      }
      clearMessages() {
        this.setMessages([]);
      }
      clearEphemeralMessages() {
        if (!this.messages.some((message) => message.ephemeral)) return;
        this.messages = this.messages.filter((message) => !message.ephemeral);
      }
      renderAfterMessageChange() {
        const atBottom = this.stickToBottom || this.isAtBottom();
        if (atBottom) {
          this.renderWindow(Math.max(0, this.messages.length - this.maxRenderedMessages));
          requestAnimationFrame(() => this.scrollToBottom());
        } else {
          this.renderForScroll();
        }
      }
      renderForScroll() {
        if (!this.scrollElement) return;
        const estimatedStart = Math.floor(this.scrollElement.scrollTop / this.rowEstimate);
        const nextStart = Math.max(0, Math.min(estimatedStart, Math.max(0, this.messages.length - this.maxRenderedMessages)));
        if (Math.abs(nextStart - this.windowStart) > 8) {
          this.renderWindow(nextStart);
        }
      }
      renderWindow(start) {
        if (!this.messageElement || !this.topSpacerElement || !this.bottomSpacerElement) return;
        this.windowStart = start;
        const visible = this.messages.slice(start, start + this.maxRenderedMessages);
        this.topSpacerElement.style.height = String(start * this.rowEstimate) + 'px';
        this.bottomSpacerElement.style.height = String(Math.max(0, this.messages.length - start - visible.length) * this.rowEstimate) + 'px';
        const lastCompletedAssistant = this.messages.findLastIndex((message) => message.kind === 'assistant' && !message.pending && !message.streaming && !message.ephemeral);
        this.messageElement.innerHTML = visible.map((message, index) => this.renderMessage(message, start + index, start + index === lastCompletedAssistant)).join('');
      }
      renderMessage(message, index, isLastCompletedAssistant) {
        const copyable = (message.kind === 'user' || message.kind === 'assistant') && !message.pending && !message.streaming && !message.ephemeral;
        const copyButton = copyable ? '<button class="copyable-message" title="Copy turn" data-copy-index="'+String(index)+'">'+workbenchIcon('copy')+'</button>' : '';
        const resubmitButton = copyable && message.kind === 'user' ? '<button class="copyable-message" title="Resubmit turn" data-resubmit-index="'+String(index)+'">'+workbenchIcon('rotate-ccw')+'</button>' : '';
        const editResubmitButton = copyable && message.kind === 'user' ? '<button class="copyable-message" title="Edit turn" data-begin-edit-index="'+String(index)+'">'+workbenchIcon('pencil')+'</button>' : '';
        const saveOutputButton = copyable && message.kind === 'assistant' ? '<button class="copyable-message" title="Save as output" data-save-output-index="'+String(index)+'">'+workbenchIcon('file-output')+'</button>' : '';
        const feedbackButtons = copyable && message.kind === 'assistant'
          ? '<button class="copyable-message'+(message.feedbackRating === 'good' ? ' feedback-selected' : '')+'" title="Good response" data-feedback-rating="good" data-feedback-index="'+String(index)+'">'+workbenchIcon('thumbs-up')+'</button><button class="copyable-message'+(message.feedbackRating === 'bad' ? ' feedback-selected' : '')+'" title="Bad response" data-feedback-rating="bad" data-feedback-index="'+String(index)+'">'+workbenchIcon('thumbs-down')+'</button>'
          : '';
        const actions = copyable ? '<div class="message-actions"><div class="message-actions-left">'+copyButton+saveOutputButton+feedbackButtons+resubmitButton+editResubmitButton+'<span class="message-timestamp">'+workbenchEscape(formatWorkbenchTurnTime(message.createdAt))+'</span></div></div>' : '';
        const body = index === this.editingIndex && message.kind === 'user'
          ? '<div class="inline-turn-editor"><textarea data-edit-value-index="'+String(index)+'">'+workbenchEscape(message.text || '')+'</textarea><div class="inline-turn-editor-actions"><button class="cancel-edit" data-cancel-edit-index="'+String(index)+'">Cancel</button><button class="submit-edit" data-submit-edit-index="'+String(index)+'">Submit</button></div></div>'
          : message.pending && message.kind === 'assistant' ? '<span class="thinking-dots">Thinking</span>' : formatAssistantMessage(message);
        return '<div class="message '+workbenchEscape(message.kind)+(message.pending ? ' pending' : '')+(message.streaming ? ' streaming' : '')+(isLastCompletedAssistant ? ' last-completed-assistant' : '')+'"><small>'+workbenchEscape(message.label)+'</small><div class="message-body">'+body+actions+'</div></div>';
      }
      async copyWorkbenchMessage(index, button) {
        const message = this.messages[index];
        if (!message || message.pending || message.streaming) return;
        const text = String(message.text || '');
        try {
          await navigator.clipboard.writeText(text);
          if (button) {
            button.classList.add('copied');
            button.title = 'Copied';
            setTimeout(() => {
              button.classList.remove('copied');
              button.title = 'Copy turn';
            }, 1200);
          }
        } catch (_) {
          window.prompt('Copy this turn', text);
        }
      }
      async handleWorkbenchFeedback(index, rating) {
        const message = this.messages[index];
        if (!message || message.kind !== 'assistant' || !message.message_id || !workbench.sessionId) {
          workbenchMessage('system', 'Feedback', 'Save completed assistant turns before rating them.');
          return;
        }
        if (rating === 'bad') {
          openResponseFeedbackModal(index);
          return;
        }
        try {
          await submitWorkbenchFeedback(index, 'good', []);
        } catch (error) {
          workbenchMessage('system', 'Feedback failed', error.message || 'Could not save response feedback.');
        }
      }
      async saveWorkbenchMessageOutput(index) {
        const message = this.messages[index];
        if (!message || message.kind !== 'assistant' || !message.message_id || !workbench.sessionId) {
          workbenchMessage('system', 'Output', 'Save completed assistant turns after they are persisted.');
          return;
        }
        const title = window.prompt('Output title', workbench.threadTitle || 'Saved output');
        if (!title || !title.trim()) return;
        try {
          const savedOutput = await postJsonPayload('/v1/threads/'+encodeURIComponent(workbench.sessionId)+'/messages/'+encodeURIComponent(message.message_id)+'/outputs', {
            title:title.trim(),
            category:'deliverable',
            output_type:'markdown_report'
          });
          if (message.threadReferenceId && savedOutput.output?.output_id) {
            await patchJsonPayload('/v1/thread-references/'+encodeURIComponent(message.threadReferenceId), {
              status:'completed',
              output_id:savedOutput.output.output_id
            });
          }
          workbenchMessage('system', 'Output saved', 'Saved this assistant turn to My outputs.');
        } catch (error) {
          workbenchMessage('system', 'Output failed', error.message || 'Could not save this output.');
        }
      }
      async resubmitWorkbenchMessage(index) {
        const message = this.messages[index];
        if (!message || message.kind !== 'user' || message.pending || message.streaming) return;
        const text = String(message.text || '').trim();
        if (!text) return;
        this.editingIndex = null;
        this.setMessages(this.messages.slice(0, index + 1));
        workbench.appThreadId = null;
        workbench.pendingAssistantText = '';
        workbench.commandOutputChars = 0;
        try {
          await ensureWorkbenchConnected();
          if (message.message_id) {
            const saved = await recordWorkbenchThreadMessage('user', 'You', text, {retry_of_message_id:message.message_id, context_cutoff_message_id:message.message_id});
            this.attachLastPersistedMessage('user', 'You', saved?.message);
          }
          await sendWorkbenchRpcMessage(text, {recordUser:false, appendUser:false});
        } catch (error) {
          workbenchMessage('system', 'Resubmit failed', error.message || 'Could not resubmit this turn.');
        }
      }
      beginEditResubmitWorkbenchMessage(index) {
        const message = this.messages[index];
        if (!message || message.kind !== 'user' || message.pending || message.streaming) return;
        this.editingIndex = index;
        this.renderAfterMessageChange();
        requestAnimationFrame(() => {
          const input = this.shadowRoot.querySelector('[data-edit-value-index="'+String(index)+'"]');
          if (input) {
            input.focus();
            input.select();
          }
        });
      }
      async submitEditedWorkbenchMessage(index) {
        const message = this.messages[index];
        if (!message || message.kind !== 'user' || message.pending || message.streaming) return;
        const input = this.shadowRoot.querySelector('[data-edit-value-index="'+String(index)+'"]');
        const text = String(input?.value || '').trim();
        if (!text) return;
        const nextMessages = this.messages.slice(0, index + 1);
        nextMessages.push({kind:'user', label:'You', text, retry_of_message_id:message.message_id, context_cutoff_message_id:message.message_id, createdAt:new Date().toISOString()});
        this.editingIndex = null;
        this.setMessages(nextMessages);
        workbench.appThreadId = null;
        workbench.pendingAssistantText = '';
        workbench.commandOutputChars = 0;
        try {
          await ensureWorkbenchConnected();
          if (message.message_id) {
            const saved = await recordWorkbenchThreadMessage('user', 'You', text, {retry_of_message_id:message.message_id, supersedes_message_id:message.message_id, context_cutoff_message_id:message.message_id});
            this.attachLastPersistedMessage('user', 'You', saved?.message);
          }
          await sendWorkbenchRpcMessage(text, {recordUser:false, appendUser:false});
        } catch (error) {
          workbenchMessage('system', 'Edit failed', error.message || 'Could not resubmit this edited turn.');
        }
      }
    }
    if (!customElements.get('workbench-transcript')) {
      customElements.define('workbench-transcript', WorkbenchTranscript);
    }
    const workbench = { socket:null, connecting:null, rpcReady:false, appThreadId:null, rpcCounter:0, pendingRpc:{}, sessionId:null, workerId:null, workspacePath:null, projectId:null, repositoryId:null, threadTitle:'Local Codex Chat', sessions:[], userWorkspaces:[], projects:[], outputs:[], contextThreadId:null, activeThreadGeneration:0, pendingAssistantText:'', commandOutputChars:0, responsePreferences:null, feedbackMessageIndex:null, pendingKnowledgeReferenceId:null };
    function workbenchEscape(value) {
      return String(value ?? '').replace(/[&<>"']/g, (char) => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[char]));
    }
    function formatWorkbenchTurnTime(value) {
      const date = value ? new Date(value) : new Date();
      if (Number.isNaN(date.getTime())) return '';
      const now = new Date();
      const sameDay = date.getFullYear() === now.getFullYear() && date.getMonth() === now.getMonth() && date.getDate() === now.getDate();
      const time = date.toLocaleTimeString([], {hour:'numeric', minute:'2-digit'}).toLowerCase();
      if (sameDay) return time;
      // Turn action timestamp examples: "11:31 pm" today, "Jun 8, 11:31 pm" after the day has passed.
      return date.toLocaleDateString([], {month:'short', day:'numeric'})+', '+time;
    }
    function renderWorkbenchInlineMarkdown(value) {
      return workbenchEscape(value)
        .replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (_match, text, href) => {
          const safeHref = sanitizeWorkbenchLink(href);
          if (!safeHref) return workbenchEscape(text);
          return '<a href="'+safeHref+'" target="_blank" rel="noopener noreferrer">'+workbenchEscape(text)+'</a>';
        })
        .replace(/`([^`]+)`/g, '<code>$1</code>')
        .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
        .replace(/\*([^*]+)\*/g, '<em>$1</em>');
    }
    function sanitizeWorkbenchLink(value) {
      try {
        const url = new URL(String(value || '').replace(/&amp;/g, '&'), window.location.origin);
        if (url.protocol === 'javascript:') return '';
        if (!['http:', 'https:', 'mailto:'].includes(url.protocol)) return '';
        return workbenchEscape(url.href);
      } catch (_) {
        return '';
      }
    }
    function renderWorkbenchMarkdown(value) {
      const lines = String(value ?? '').replace(/\r\n/g, '\n').split('\n');
      const html = [];
      let paragraph = [];
      let listType = null;
      let inCode = false;
      const flushParagraph = () => {
        if (!paragraph.length) return;
        html.push('<p>'+renderWorkbenchInlineMarkdown(paragraph.join(' '))+'</p>');
        paragraph = [];
      };
      const closeList = () => {
        if (!listType) return;
        html.push('</'+listType+'>');
        listType = null;
      };
      for (const rawLine of lines) {
        const line = rawLine.trimEnd();
        if (line.trim().startsWith('```')) {
          flushParagraph();
          closeList();
          if (inCode) {
            html.push('</code></pre>');
          } else {
            html.push('<pre><code>');
          }
          inCode = !inCode;
          continue;
        }
        if (inCode) {
          html.push(workbenchEscape(rawLine)+'\n');
          continue;
        }
        const trimmed = line.trim();
        if (!trimmed) {
          flushParagraph();
          closeList();
          continue;
        }
        const heading = trimmed.match(/^(#{1,3})\s+(.+)$/);
        if (heading) {
          flushParagraph();
          closeList();
          const level = String(heading[1]).length;
          html.push('<h'+level+'>'+renderWorkbenchInlineMarkdown(heading[2])+'</h'+level+'>');
          continue;
        }
        const unordered = trimmed.match(/^[-*]\s+(.+)$/);
        const ordered = trimmed.match(/^\d+\.\s+(.+)$/);
        if (unordered || ordered) {
          flushParagraph();
          const nextType = unordered ? 'ul' : 'ol';
          if (listType !== nextType) {
            closeList();
            listType = nextType;
            html.push('<'+listType+'>');
          }
          html.push('<li>'+renderWorkbenchInlineMarkdown((unordered || ordered)[1])+'</li>');
          continue;
        }
        closeList();
        paragraph.push(trimmed);
      }
      flushParagraph();
      closeList();
      if (inCode) html.push('</code></pre>');
      return html.join('');
    }
    function formatAssistantMessage(message) {
      const text = message?.text || '';
      if (message?.label === 'Tool output') {
        return '<details class="tool-output-collapsed"><summary>Raw tool output collapsed</summary><pre><code>'+workbenchEscape(text)+'</code></pre></details>';
      }
      if (message?.kind === 'assistant' || message?.kind === 'system') {
        return renderWorkbenchMarkdown(text);
      }
      return '<p>'+workbenchEscape(text).replace(/\n/g, '<br>')+'</p>';
    }
    function workbenchIcon(name) {
      const icons = {
        'check': '<path d="M20 6 9 17l-5-5"/>',
        'message-circle': '<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/>',
        'copy': '<rect width="14" height="14" x="8" y="8" rx="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>',
        'file-output': '<path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8Z"/><path d="M14 2v6h6"/><path d="M12 18v-6"/><path d="m9 15 3 3 3-3"/>',
        'more-horizontal': '<circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/>',
        'panel-left-close': '<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M9 3v18"/><path d="m16 15-3-3 3-3"/>',
        'pencil': '<path d="M21.2 6.8a2.8 2.8 0 0 0-4-4L4 16v4h4Z"/><path d="m14 5 5 5"/>',
        'plus': '<path d="M5 12h14"/><path d="M12 5v14"/>',
        'rotate-ccw': '<path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5"/>',
        'send': '<path d="m22 2-7 20-4-9-9-4Z"/><path d="M22 2 11 13"/>',
        'thumbs-up': '<path d="M7 10v12"/><path d="M15 5.9 14 10h5.8a2 2 0 0 1 2 2.3l-1.4 7a2 2 0 0 1-2 1.7H7"/><path d="M7 10H2v12h5"/><path d="M14 10V5a3 3 0 0 0-3-3l-4 8"/>',
        'thumbs-down': '<path d="M17 14V2"/><path d="M9 18.1 10 14H4.2a2 2 0 0 1-2-2.3l1.4-7a2 2 0 0 1 2-1.7H17"/><path d="M17 14h5V2h-5"/><path d="M10 14v5a3 3 0 0 0 3 3l4-8"/>'
      };
      const body = icons[name] || icons['more-horizontal'];
      return '<svg class="lucide-icon" data-icon="'+workbenchEscape(name)+'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">'+body+'</svg>';
    }
    function workbenchMessage(kind, label, text) {
      const chat = document.getElementById('workbench-chat');
      if (!chat) return;
      if (typeof chat.appendMessage === 'function') {
        chat.appendMessage(kind, label, text);
      }
    }
    function showWorkbenchEmptyState(text) {
      const chat = document.getElementById('workbench-chat');
      if (chat && typeof chat.setEmptyMessage === 'function') {
        chat.setEmptyMessage('assistant', 'Local Codex', text);
      }
      workbench.pendingAssistantText = '';
    }
    function clearWorkbenchTranscript() {
      const chat = document.getElementById('workbench-chat');
      if (chat && typeof chat.clearMessages === 'function') {
        chat.clearMessages();
      }
      workbench.pendingAssistantText = '';
    }
    function bumpWorkbenchThreadGeneration() {
      workbench.activeThreadGeneration += 1;
      return workbench.activeThreadGeneration;
    }
    function isActiveWorkbenchGeneration(generation) {
      return generation === workbench.activeThreadGeneration;
    }
    function setWorkbenchTranscript(messages) {
      const chat = document.getElementById('workbench-chat');
      if (!chat || typeof chat.setMessages !== 'function') return;
      chat.setMessages((messages || []).map((message) => ({
        kind: message.kind || 'assistant',
        label: message.label || (message.kind === 'user' ? 'You' : 'Codex'),
        text: message.text || '',
        message_id: message.message_id,
        session_id: message.session_id,
        feedbackRating:message.feedback_rating || message.feedbackRating,
        createdAt:message.created_at || message.createdAt
      })));
      workbench.pendingAssistantText = '';
    }
    async function loadWorkbenchThreadHistory(sessionId, generation) {
      const payload = await fetchJson('/v1/threads/'+encodeURIComponent(sessionId)+'/messages');
      if (generation !== undefined && !isActiveWorkbenchGeneration(generation)) return;
      const messages = payload.messages || [];
      if (!messages.length) {
        showWorkbenchEmptyState('No messages yet in this thread.');
        return;
      }
      setWorkbenchTranscript(messages);
    }
    async function recordWorkbenchThreadMessage(kind, label, text, metadata) {
      if (!workbench.sessionId || !text || !String(text).trim()) return;
      return await postJsonPayload('/v1/threads/'+encodeURIComponent(workbench.sessionId)+'/messages', Object.assign({kind, label, text}, metadata || {}));
    }
    function openResponseFeedbackModal(index) {
      workbench.feedbackMessageIndex = index;
      const modal = document.getElementById('response-feedback-modal');
      if (!modal) return;
      modal.querySelectorAll('input[type="checkbox"]').forEach((input) => {
        input.checked = false;
      });
      modal.showModal();
    }
    async function submitResponseFeedbackFromModal() {
      const modal = document.getElementById('response-feedback-modal');
      const tags = [...modal.querySelectorAll('input[type="checkbox"]:checked')].map((input) => input.value);
      try {
        await submitWorkbenchFeedback(workbench.feedbackMessageIndex, 'bad', tags.length ? tags : ['other']);
        modal.close();
      } catch (error) {
        workbenchMessage('system', 'Feedback failed', error.message || 'Could not save response feedback.');
      }
    }
    async function submitWorkbenchFeedback(index, rating, reasonTags) {
      const chat = document.getElementById('workbench-chat');
      const message = chat?.messages?.[index];
      if (!message || !message.message_id || !workbench.sessionId) {
        throw new Error('This response is not ready for feedback yet.');
      }
      const payload = await putJsonPayload('/v1/threads/'+encodeURIComponent(workbench.sessionId)+'/messages/'+encodeURIComponent(message.message_id)+'/feedback', {
        rating,
        reason_tags: reasonTags || []
      });
      workbench.responsePreferences = payload.preferences || workbench.responsePreferences;
      message.feedbackRating = rating;
      chat.renderAfterMessageChange?.();
      return payload;
    }
    function openResponsePreferencesModal() {
      const modal = document.getElementById('response-preferences-modal');
      const body = document.getElementById('response-preferences-body');
      const summary = String(workbench.responsePreferences?.profile_summary || '').trim();
      if (body) {
        body.textContent = summary || 'No response preferences have been learned yet.';
      }
      modal?.showModal();
    }
    async function resetResponsePreferences() {
      await deleteJsonPayload('/v1/me/response-preferences');
      workbench.responsePreferences = {profile_summary:'', sample_count:0};
      openResponsePreferencesModal();
    }
    function appendWorkbenchAssistantDelta(delta) {
      if (!delta) return;
      const chat = document.getElementById('workbench-chat');
      if (chat && typeof chat.replacePendingMessage === 'function') {
        chat.replacePendingMessage('assistant', 'Codex', delta);
      } else {
        workbenchMessage('assistant', 'Codex', delta);
      }
      workbench.pendingAssistantText += delta;
    }
    function appendWorkbenchAssistantPending() {
      const chat = document.getElementById('workbench-chat');
      if (chat && typeof chat.appendPendingMessage === 'function') {
        chat.appendPendingMessage('assistant', 'Codex', 'Thinking...');
      } else {
        workbenchMessage('assistant', 'Codex', 'Thinking...');
      }
    }
    function completeWorkbenchAssistantTurn() {
      const chat = document.getElementById('workbench-chat');
      if (chat && typeof chat.completeLastMessage === 'function') {
        chat.completeLastMessage('assistant', 'Codex');
      }
    }
    function replacePendingAssistantDelta(delta) {
      appendWorkbenchAssistantDelta(delta);
    }
    function workbenchTurnGuidance() {
      const raw = document.getElementById('chat-turn-guidance')?.value || '{}';
      try {
        return JSON.parse(raw);
      } catch (_) {
        return {};
      }
    }
    function handleWorkbenchCommandOutputDelta(delta) {
      if (!delta) return;
      workbench.commandOutputChars += String(delta).length;
      const chat = document.getElementById('workbench-chat');
      const text = 'Raw tool output is collapsed in the browser transcript. Codex can use the result, but '+workbench.commandOutputChars+' characters of command output are hidden from chat.';
      if (chat && typeof chat.appendPendingMessage === 'function') {
        chat.appendPendingMessage('system', 'Tool output', text);
      } else {
        workbenchMessage('system', 'Tool output', text);
      }
    }
    function setWorkbenchStatus(text, connected) {
      const label = document.getElementById('workbench-status-label');
      const dot = document.getElementById('workbench-status-dot');
      if (label) label.textContent = text;
      if (dot) dot.classList.toggle('connected', Boolean(connected));
    }
    function setWorkbenchThreadTitle(title) {
      workbench.threadTitle = title || 'Local Codex Chat';
      const target = document.getElementById('workbench-thread-title');
      if (target) {
        target.textContent = workbench.threadTitle;
      } else {
        renderWorkbenchTitleStatic(workbench.threadTitle);
      }
    }
    function beginWorkbenchTitleEdit() {
      if (!workbench.sessionId && !workbench.projectId) return;
      const wrap = document.getElementById('workbench-thread-title-wrap');
      if (!wrap || wrap.querySelector('input')) return;
      const previous = workbench.threadTitle || 'New chat thread';
      wrap.dataset.previousTitle = previous;
      wrap.innerHTML = '<input id="workbench-thread-title-input" class="thread-title-input" value="'+workbenchEscape(previous)+'" onkeydown="threadTitleEditKeydown(event)" onblur="saveWorkbenchTitleEdit()"><button class="icon-button thread-title-edit-button" title="Save thread title" onclick="saveWorkbenchTitleEdit()">'+workbenchIcon('check')+'</button>';
      const input = document.getElementById('workbench-thread-title-input');
      if (input) {
        input.focus();
        input.select();
      }
    }
    function renderWorkbenchTitleStatic(title) {
      const wrap = document.getElementById('workbench-thread-title-wrap');
      if (!wrap) return;
      wrap.innerHTML = '<span id="workbench-thread-title" class="chat-title">'+workbenchEscape(title || 'Local Codex Chat')+'</span><button class="icon-button thread-title-edit-button" title="Rename thread" onclick="beginWorkbenchTitleEdit()">'+workbenchIcon('pencil')+'</button>';
    }
    function threadTitleEditKeydown(event) {
      if (event.key === 'Enter') {
        event.preventDefault();
        saveWorkbenchTitleEdit();
      } else if (event.key === 'Escape') {
        event.preventDefault();
        const previous = document.getElementById('workbench-thread-title-wrap')?.dataset.previousTitle || workbench.threadTitle;
        setWorkbenchThreadTitle(previous);
      }
    }
    async function renameWorkbenchThread(sessionId, nextTitle) {
      const title = String(nextTitle || '').trim() || 'New chat thread';
      if (!sessionId) {
        setWorkbenchThreadTitle(title);
        return null;
      }
      const response = await patchJsonPayload('/v1/threads/'+encodeURIComponent(sessionId), {title});
      const session = response.session || {};
      if (session.session_id === workbench.sessionId) setWorkbenchThreadTitle(session.title || title);
      workbench.sessions = workbench.sessions.map((item) => item.session_id === session.session_id ? Object.assign({}, item, session) : item);
      workbench.projects = workbench.projects.map((project) => Object.assign({}, project, {
        threads: (project.threads || []).map((thread) => thread.session_id === session.session_id ? Object.assign({}, thread, session) : thread)
      }));
      renderWorkbenchProjects();
      return session;
    }
    function isDefaultWorkbenchThreadTitle(title) {
      const normalized = String(title || '').trim();
      return !normalized || normalized === 'New chat thread' || normalized === 'Local Codex Chat';
    }
    function deriveWorkbenchThreadTitle(firstUserTurn, firstAssistantTurn) {
      const combined = [firstUserTurn?.text, firstAssistantTurn?.text]
        .filter(Boolean)
        .join(' ')
        .replace(/https?:\/\/\S+/g, ' ')
        .replace(/[`*_#>{}\[\]()"']/g, ' ')
        .replace(/&[a-z]+;/gi, ' ')
        .replace(/\s+/g, ' ')
        .trim();
      if (!combined) return '';
      const stopWords = new Set(['a','an','and','are','as','at','based','be','but','by','can','codex','do','for','from','have','hello','help','hey','hi','i','if','in','is','it','let','me','of','on','or','our','please','project','repo','repository','should','simple','specific','that','the','their','this','to','we','what','with','work','would','you','your']);
      const seen = new Set();
      const words = combined
        .split(/[^A-Za-z0-9.+-]+/)
        .map((word) => word.trim())
        .filter((word) => word.length > 2)
        .filter((word) => !stopWords.has(word.toLowerCase()))
        .filter((word) => {
          const key = word.toLowerCase();
          if (seen.has(key)) return false;
          seen.add(key);
          return true;
        })
        .slice(0, 6)
        .map((word) => word.length <= 4 && word === word.toUpperCase() ? word : word.charAt(0).toUpperCase()+word.slice(1).toLowerCase());
      const title = words.join(' ').trim();
      if (title.length > 54) return title.slice(0, 51).trim()+'...';
      return title || 'New chat thread';
    }
    async function autoLabelWorkbenchThreadAfterFirstAiTurn() {
      if (!workbench.sessionId) return;
      if (!isDefaultWorkbenchThreadTitle(workbench.threadTitle)) return;
      const chat = document.getElementById('workbench-chat');
      const messages = Array.isArray(chat?.messages) ? chat.messages.filter((message) => !message.ephemeral) : [];
      const userTurns = messages.filter((message) => message.kind === 'user');
      const assistantTurns = messages.filter((message) => message.kind === 'assistant' && !message.pending && !message.streaming);
      if (userTurns.length !== 1 || assistantTurns.length !== 1) return;
      const firstUserTurn = userTurns[0];
      const firstAssistantTurn = assistantTurns[0];
      const title = deriveWorkbenchThreadTitle(firstUserTurn, firstAssistantTurn);
      if (!title || isDefaultWorkbenchThreadTitle(title)) return;
      await renameWorkbenchThread(workbench.sessionId, title);
    }
    async function saveWorkbenchTitleEdit() {
      const input = document.getElementById('workbench-thread-title-input');
      if (!input) return;
      const nextTitle = input.value.trim() || 'New chat thread';
      try {
        await renameWorkbenchThread(workbench.sessionId, nextTitle);
      } catch (error) {
        workbenchMessage('system', 'Rename failed', error.message);
      }
    }
    function openThreadContextMenu(event, sessionId) {
      event.preventDefault();
      event.stopPropagation();
      workbench.contextThreadId = sessionId;
      const menu = document.getElementById('thread-context-menu');
      if (!menu) return;
      menu.style.left = String(Math.min(event.clientX, window.innerWidth - 180)) + 'px';
      menu.style.top = String(Math.min(event.clientY, window.innerHeight - 96)) + 'px';
      menu.classList.add('open');
    }
    function closeThreadContextMenu() {
      document.getElementById('thread-context-menu')?.classList.remove('open');
    }
    function renameThreadFromContextMenu() {
      const session = workbench.sessions.find((item) => item.session_id === workbench.contextThreadId);
      closeThreadContextMenu();
      if (!session) return;
      const modal = document.getElementById('thread-rename-modal');
      const input = document.getElementById('thread-rename-input');
      if (!modal || !input) return;
      input.value = session.title || 'New chat thread';
      modal.showModal();
      input.focus();
      input.select();
    }
    async function saveThreadRenameModal() {
      const modal = document.getElementById('thread-rename-modal');
      const input = document.getElementById('thread-rename-input');
      if (!input || !workbench.contextThreadId) return;
      try {
        await renameWorkbenchThread(workbench.contextThreadId, input.value);
        if (modal?.open) modal.close();
      } catch (error) {
        workbenchMessage('system', 'Rename failed', error.message);
      }
    }
    async function removeWorkbenchThread(sessionId) {
      const session = workbench.sessions.find((item) => item.session_id === sessionId);
      if (!session) return;
      if (!window.confirm('Remove thread "'+(session.title || 'New chat thread')+'"?')) return;
      try {
        await deleteJsonPayload('/v1/threads/'+encodeURIComponent(sessionId));
        workbench.sessions = workbench.sessions.filter((item) => item.session_id !== sessionId);
        workbench.projects = workbench.projects.map((project) => Object.assign({}, project, {
          threads: (project.threads || []).filter((thread) => thread.session_id !== sessionId)
        }));
        if (workbench.sessionId === sessionId) {
          resetWorkbenchConnection();
          workbench.sessionId = null;
          workbench.workerId = null;
          workbench.repositoryId = null;
          setComposerVisible(false);
          clearWorkbenchTranscript();
          setWorkbenchThreadTitle(session.project_name || 'Local Codex Chat');
          workbenchMessage('assistant', 'Thread removed', 'Choose another thread or start a new one from a project.');
        }
        renderWorkbenchProjects();
      } catch (error) {
        workbenchMessage('system', 'Remove failed', error.message);
      }
    }
    function removeThreadFromContextMenu() {
      const sessionId = workbench.contextThreadId;
      closeThreadContextMenu();
      removeWorkbenchThread(sessionId);
    }
    if (!window.__localCodexThreadMenuBound) {
      window.__localCodexThreadMenuBound = true;
      document.addEventListener('click', closeThreadContextMenu);
      document.addEventListener('keydown', (event) => {
        if (event.key === 'Escape') closeThreadContextMenu();
      });
    }
    function setComposerVisible(visible) {
      const composer = document.getElementById('chat-composer-wrap');
      if (composer) composer.classList.toggle('composer-hidden', !visible);
    }
    function toggleChatRail() {
      document.querySelector('.chat-shell-fullscreen')?.classList.toggle('chat-rail-collapsed');
    }
    function hasAssignedUserWorkspace() {
      return workbench.userWorkspaces.some((workspace) => Boolean(workspace.user_workspace_id));
    }
    function setWorkbenchProjectCreationAvailable(available) {
      const button = document.querySelector('.projects-new-button');
      if (!button) return;
      button.disabled = !available;
      button.setAttribute('aria-disabled', available ? 'false' : 'true');
      button.title = available
        ? 'New project'
        : 'New projects are unavailable until a user workspace is assigned.';
    }
    function showNoUserWorkspaceState() {
      resetWorkbenchConnection();
      workbench.sessionId = null;
      workbench.projectId = null;
      workbench.repositoryId = null;
      workbench.workspacePath = null;
      setComposerVisible(false);
      renderWorkbenchProjects();
      showWorkbenchEmptyState('No user workspace is assigned to this account yet. Ask an admin, manager, or workspace owner to grant access before creating projects or chat threads.');
    }
    async function loadWorkbench() {
      if (!document.getElementById('workbench-chat')) return;
      try {
        const [me, userWorkspaces, projects, preferences] = await Promise.all([
          fetchJson('/v1/auth/me'),
          fetchJson('/v1/user-workspaces').catch(() => ({user_workspaces: []})),
          fetchJson('/v1/projects').catch(() => ({projects: []})),
          fetchJson('/v1/me/response-preferences').catch(() => ({preferences: null}))
        ]);
        workbench.userWorkspaces = userWorkspaces.user_workspaces || [];
        workbench.projects = projects.projects || [];
        workbench.responsePreferences = preferences.preferences || null;
        const user = document.getElementById('workbench-user');
        if (user) {
          user.textContent = me.email + ' (' + me.role + ')';
        }
        setWorkbenchProjectCreationAvailable(hasAssignedUserWorkspace());
        await refreshWorkbenchThreads();
        if (!workbench.userWorkspaces.length) {
          showNoUserWorkspaceState();
        } else if (!workbench.projects.length) {
          workbenchMessage('assistant', 'Local Codex', 'No projects yet. Use the Projects plus button to create one.');
        } else if (!await selectLastWorkbenchThread()) {
          workbenchMessage('assistant', 'Local Codex', 'Please select a thread to start chatting, or use the chat bubble on a project to create one.');
        } else {
          if (!document.getElementById('workbench-chat')?.messages?.length) {
            workbenchMessage('assistant', 'Local Codex', 'Loaded your most recent chat thread.');
          }
        }
      } catch (error) {
        workbenchMessage('system', 'Error', error.message);
      }
    }
    async function refreshWorkbenchThreads() {
      if (!document.getElementById('workbench-project-list')) return;
      try {
        const payload = await fetchJson('/v1/projects').catch(() => ({projects: []}));
        workbench.projects = payload.projects || [];
        workbench.sessions = workbench.projects.flatMap((project) => (project.threads || []).map((thread) => Object.assign({}, thread, {project_id: project.project_id, project_name: project.name, project_path: project.project_path})));
        if (workbench.sessionId && !workbench.sessions.some((session) => session.session_id === workbench.sessionId)) {
          workbench.sessionId = null;
        }
        if (!workbench.projectId && workbench.projects.length) {
          workbench.projectId = workbench.projects[0].project_id;
          workbench.workspacePath = workbench.projects[0].project_path;
        }
        renderWorkbenchProjects();
      } catch (error) {
        workbenchMessage('system', 'Threads', error.message);
      }
    }
    function workspaceLabel(path) {
      const parts = String(path || '').split('/').filter(Boolean);
      return parts.pop() || path || 'Workspace';
    }
    function renderWorkbenchProjects() {
      const target = document.getElementById('workbench-project-list');
      if (!target) return;
      setWorkbenchProjectCreationAvailable(hasAssignedUserWorkspace());
      if (!workbench.projects.length) {
        target.innerHTML = !hasAssignedUserWorkspace()
          ? '<p class="hint">No user workspace assigned. Ask an admin or manager to grant your workspace before creating projects.</p>'
          : '<p class="hint">No projects yet. Use the Projects plus button to create one.</p>';
        return;
      }
      target.innerHTML = workbench.projects.map((project) => {
        const path = project.project_path;
        const sessions = project.threads || [];
        const rows = sessions.length
          ? sessions.map((session) => '<button class="thread-row '+(session.session_id === workbench.sessionId ? 'active' : '')+'" data-session-id="'+workbenchEscape(session.session_id)+'" onclick="selectWorkbenchThread(&quot;'+workbenchEscape(session.session_id)+'&quot;)" oncontextmenu="openThreadContextMenu(event, &quot;'+workbenchEscape(session.session_id)+'&quot;)">'+workbenchEscape(session.title || 'Untitled thread')+'</button>').join('')
          : '<p class="hint project-empty">No chats</p>';
        return '<section class="project-group" data-project-id="'+workbenchEscape(project.project_id)+'"><div class="project-header" onclick="chooseWorkbenchProject(&quot;'+workbenchEscape(project.project_id)+'&quot;)"><span class="project-name">▱ '+workbenchEscape(project.name || workspaceLabel(path))+'</span><span class="project-tools"><span class="project-menu-wrap"><button class="icon-button project-menu-button" title="Project menu" onclick="event.stopPropagation(); toggleProjectMenu(this)">'+workbenchIcon('more-horizontal')+'</button><span class="project-menu-dropdown"><button onclick="event.stopPropagation(); openProjectMenu(&quot;'+workbenchEscape(project.project_id)+'&quot;)">Add repository</button><button class="danger-menu-item" onclick="event.stopPropagation(); removeWorkbenchProject(&quot;'+workbenchEscape(project.project_id)+'&quot;)">Remove project</button></span></span><button class="icon-button project-new-thread-button" title="New chat thread" onclick="event.stopPropagation(); newWorkbenchThread(&quot;'+workbenchEscape(project.project_id)+'&quot;)">'+workbenchIcon('message-circle')+'</button></span></div><div class="project-thread-list">'+rows+'</div></section>';
      }).join('');
      document.querySelectorAll('.project-group').forEach((group) => group.classList.toggle('active', group.dataset.projectId === workbench.projectId));
    }
    async function selectLastWorkbenchThread() {
      if (!workbench.sessions.length || workbench.sessionId) return false;
      const latest = [...workbench.sessions].sort((a, b) => String(b.last_opened_at || b.updated_at || '').localeCompare(String(a.last_opened_at || a.updated_at || '')))[0];
      if (!latest) return false;
      const generation = bumpWorkbenchThreadGeneration();
      workbench.sessionId = latest.session_id;
      workbench.projectId = latest.project_id || null;
      workbench.repositoryId = latest.repository_id || null;
      workbench.workspacePath = latest.workspace_path;
      setWorkbenchThreadTitle(latest.title || 'Existing chat thread');
      setComposerVisible(true);
      renderWorkbenchProjects();
      clearWorkbenchTranscript();
      try {
        await loadWorkbenchThreadHistory(latest.session_id, generation);
      } catch (error) {
        if (isActiveWorkbenchGeneration(generation)) workbenchMessage('system', 'History', error.message);
      }
      return true;
    }
    function chooseWorkbenchProject(projectId) {
      bumpWorkbenchThreadGeneration();
      resetWorkbenchConnection();
      const project = workbench.projects.find((item) => item.project_id === projectId);
      workbench.projectId = projectId;
      workbench.repositoryId = null;
      workbench.workspacePath = project ? project.project_path : null;
      workbench.sessionId = null;
      setWorkbenchThreadTitle(project ? project.name : 'Local Codex Chat');
      setComposerVisible(false);
      renderWorkbenchProjects();
      clearWorkbenchTranscript();
      workbenchMessage('assistant', 'Project selected', 'Use the chat bubble on this project to start a new thread, or choose an existing thread below it.');
    }
    function newWorkbenchThread(projectId) {
      bumpWorkbenchThreadGeneration();
      resetWorkbenchConnection();
      const project = workbench.projects.find((item) => item.project_id === projectId);
      workbench.projectId = projectId;
      workbench.repositoryId = null;
      workbench.workspacePath = project ? project.project_path : null;
      workbench.sessionId = null;
      setWorkbenchThreadTitle('New chat thread');
      setComposerVisible(true);
      renderWorkbenchProjects();
      clearWorkbenchTranscript();
      workbenchMessage('system', 'New chat', 'Write a message to create this thread.');
    }
    function openProjectsUtilityModal() {
      const modal = document.getElementById('workbench-utility-modal');
      if (!modal) return;
      modal.showModal();
    }
    function openNewProjectModal() {
      if (!hasAssignedUserWorkspace()) {
        showNoUserWorkspaceState();
        return;
      }
      const modal = document.getElementById('workbench-project-modal');
      if (!modal) return;
      modal.showModal();
    }
    function toggleProjectMenu(button) {
      const wrap = button.closest('.project-menu-wrap');
      if (!wrap) return;
      document.querySelectorAll('.project-menu-wrap.open').forEach((item) => {
        if (item !== wrap) item.classList.remove('open');
      });
      wrap.classList.toggle('open');
    }
    function openProjectMenu(projectId) {
      document.querySelectorAll('.project-menu-wrap.open').forEach((item) => item.classList.remove('open'));
      const modal = document.getElementById('workbench-repository-modal');
      const selected = document.getElementById('repository-selected-project');
      if (!modal || !selected) return;
      const project = workbench.projects.find((item) => item.project_id === projectId);
      selected.value = projectId;
      modal.querySelector('strong').textContent = project?.name || 'Project';
      modal.showModal();
    }
    async function createWorkbenchProject() {
      try {
        const name = v('workbench-project-name');
        const userWorkspace = workbench.userWorkspaces[0];
        if (!userWorkspace?.user_workspace_id) {
          const modal = document.getElementById('workbench-project-modal');
          if (modal?.open) modal.close();
          showNoUserWorkspaceState();
          return;
        }
        if (!name) {
          workbenchMessage('system', 'Project', 'Enter a project name.');
          return;
        }
        const created = await postJsonPayload('/v1/user-workspaces/'+encodeURIComponent(userWorkspace.user_workspace_id)+'/projects', {name});
        document.getElementById('workbench-project-modal').close();
        workbenchMessage('assistant', 'Project created', created.project?.name || 'Project was created.');
        await loadWorkbench();
      } catch (error) {
        workbenchMessage('system', 'Project failed', error.message);
      }
    }
    async function addRepositoryToSelectedProject() {
      try {
        const repoUrl = v('workbench-clone-url');
        const destination = v('workbench-clone-destination');
        const projectId = v('repository-selected-project');
        if (!repoUrl || !destination || !projectId) {
          workbenchMessage('system', 'Clone', 'Choose a project, then enter an HTTPS repository URL and destination folder.');
          return;
        }
        const cloned = await postJsonPayload('/v1/projects/'+encodeURIComponent(projectId)+'/repositories/clone', {repo_url:repoUrl, destination_name:destination});
        document.getElementById('workbench-repository-modal').close();
        workbenchMessage('assistant', 'Repository cloned', cloned.repository?.repository_path || 'Repository was cloned into the selected project.');
        await loadWorkbench();
      } catch (error) {
        workbenchMessage('system', 'Clone failed', error.message);
      }
    }
    async function removeWorkbenchProject(projectId) {
      const project = workbench.projects.find((item) => item.project_id === projectId);
      if (!projectId || !project) return;
      if (!window.confirm('Remove project "'+(project.name || 'Project')+'"? Admins can restore it from the admin project page.')) return;
      try {
        await deleteJsonPayload('/v1/projects/'+encodeURIComponent(projectId));
        if (workbench.projectId === projectId) {
          resetWorkbenchConnection();
          workbench.sessionId = null;
          workbench.projectId = null;
          workbench.repositoryId = null;
          workbench.workspacePath = null;
          setComposerVisible(false);
          clearWorkbenchTranscript();
          setWorkbenchThreadTitle('Local Codex Chat');
        }
        workbenchMessage('assistant', 'Project removed', (project.name || 'Project')+' was removed from chat. An admin can restore it.');
        await loadWorkbench();
      } catch (error) {
        workbenchMessage('system', 'Remove failed', error.message);
      }
    }
    async function openWorkbenchOutputs() {
      const modal = document.getElementById('workbench-output-modal');
      const list = document.getElementById('workbench-output-list');
      if (!modal || !list) return;
      list.innerHTML = '<p class="hint">Loading outputs...</p>';
      modal.showModal();
      try {
        const payload = await fetchJson('/v1/outputs');
        const outputs = payload.outputs || [];
        workbench.outputs = outputs;
        if (!outputs.length) {
          list.innerHTML = '<p class="hint">No outputs are assigned to your user yet.</p>';
          return;
        }
        list.innerHTML = outputs.map((output) =>
          '<article class="output-row"><strong>'+workbenchEscape(output.title || 'Untitled output')+'</strong>'+
          '<div class="hint">'+workbenchEscape(output.category || 'output')+' · '+workbenchEscape(output.output_type || 'asset')+' · '+workbenchEscape(output.status || 'unknown')+'</div>'+
          '<code>'+workbenchEscape(output.artifact_path || '')+'</code><div><a href="/v1/outputs/'+encodeURIComponent(output.output_id)+'/download" target="_blank" rel="noopener">Download</a></div></article>'
        ).join('');
      } catch (error) {
        list.innerHTML = '<p class="hint">Could not load outputs: '+workbenchEscape(error.message)+'</p>';
      }
    }
    function currentKnowledgeThreadOptions() {
      return workbench.projects.flatMap((project) => (project.threads || []).map((thread) => ({
        id: thread.session_id,
        label: (project.name || 'Project')+' / '+(thread.title || 'Untitled thread')
      })));
    }
    function fillWorkbenchSelect(id, items, label, value, emptyLabel) {
      const element = document.getElementById(id);
      if (!element) return;
      element.innerHTML = '';
      element.disabled = items.length === 0;
      if (!items.length) {
        const option = document.createElement('option');
        option.textContent = emptyLabel || 'No options available';
        option.disabled = true;
        option.selected = true;
        element.appendChild(option);
        return;
      }
      for (const item of items) {
        const option = document.createElement('option');
        option.value = value(item);
        option.textContent = label(item);
        element.appendChild(option);
      }
    }
    async function openKnowledgeTransferModal() {
      const modal = document.getElementById('knowledge-transfer-modal');
      const status = document.getElementById('knowledge-transfer-status');
      if (!modal) return;
      if (status) status.textContent = workbench.sessionId ? '' : 'Select or create a target thread before using thread knowledge.';
      fillWorkbenchSelect('knowledge-source-thread', currentKnowledgeThreadOptions(), (thread) => thread.label, (thread) => thread.id, 'No accessible threads');
      try {
        const payload = await fetchJson('/v1/outputs');
        workbench.outputs = payload.outputs || [];
      } catch (_) {
        workbench.outputs = [];
      }
      fillWorkbenchSelect('knowledge-source-output', workbench.outputs, (output) => (output.title || 'Untitled output')+' · '+(output.output_type || 'output'), (output) => output.output_id, 'No outputs available');
      modal.showModal();
    }
    function selectedKnowledgeSourceThreadId() {
      return document.getElementById('knowledge-source-thread')?.value || '';
    }
    function selectedKnowledgeOutputId() {
      return document.getElementById('knowledge-source-output')?.value || '';
    }
    function knowledgeMaxChars() {
      const value = Number(document.getElementById('knowledge-max-chars')?.value || 12000);
      if (!Number.isFinite(value)) return 12000;
      return Math.max(64, Math.min(20000, Math.floor(value)));
    }
    function setKnowledgeTransferStatus(text) {
      const status = document.getElementById('knowledge-transfer-status');
      if (status) status.textContent = text || '';
    }
    async function sendWorkbenchGeneratedPrompt(text, label) {
      const generation = workbench.activeThreadGeneration;
      try {
        await ensureWorkbenchConnected();
        if (!isActiveWorkbenchGeneration(generation)) return;
        workbenchMessage('system', label || 'Thread knowledge', 'Sending bounded reference prompt through this thread worker.');
        await sendWorkbenchRpcMessage(text, {recordUser:false, appendUser:false, rawPrompt:true});
      } catch (firstError) {
        try {
          resetWorkbenchConnection();
          await ensureWorkbenchConnected();
          if (!isActiveWorkbenchGeneration(generation)) return;
          await sendWorkbenchRpcMessage(text, {recordUser:false, appendUser:false, rawPrompt:true});
        } catch (error) {
          workbenchMessage('system', 'Knowledge transfer failed', error.message || firstError.message || 'Could not send thread knowledge.');
        }
      }
    }
    async function summarizeKnowledgeThread() {
      if (!workbench.sessionId) {
        setKnowledgeTransferStatus('Select or create a target thread first.');
        return;
      }
      const sourceThreadId = selectedKnowledgeSourceThreadId();
      if (!sourceThreadId) {
        setKnowledgeTransferStatus('Choose a source thread.');
        return;
      }
      try {
        const payload = await postJsonPayload('/v1/threads/'+encodeURIComponent(workbench.sessionId)+'/references', {
          source_thread_id: sourceThreadId,
          reference_type: 'ai_summary',
          max_chars: knowledgeMaxChars()
        });
        document.getElementById('knowledge-transfer-modal')?.close();
        workbench.pendingKnowledgeReferenceId = payload.reference.reference_id;
        workbenchMessage('system', 'Thread reference', 'AI summary reference '+payload.reference.reference_id+' is pending in this thread.');
        await sendWorkbenchGeneratedPrompt(payload.summary_prompt, 'Summarize into this thread');
      } catch (error) {
        setKnowledgeTransferStatus(error.message || 'Could not create AI summary reference.');
      }
    }
    async function exportKnowledgeThread() {
      const sourceThreadId = selectedKnowledgeSourceThreadId();
      if (!sourceThreadId) {
        setKnowledgeTransferStatus('Choose a source thread.');
        return;
      }
      try {
        const payload = await postJsonPayload('/v1/threads/'+encodeURIComponent(sourceThreadId)+'/exports', {
          target_thread_id: workbench.sessionId || sourceThreadId,
          title: document.getElementById('knowledge-artifact-title')?.value || undefined,
          max_chars: knowledgeMaxChars()
        });
        document.getElementById('knowledge-transfer-modal')?.close();
        workbenchMessage('system', 'Transcript exported', 'Created output '+payload.output.output_id+' and reference '+payload.reference.reference_id+'.');
        workbench.outputs = [];
      } catch (error) {
        setKnowledgeTransferStatus(error.message || 'Could not export transcript.');
      }
    }
    async function createKnowledgeHandoff() {
      const sourceThreadId = selectedKnowledgeSourceThreadId();
      if (!sourceThreadId) {
        setKnowledgeTransferStatus('Choose a source thread.');
        return;
      }
      try {
        const payload = await postJsonPayload('/v1/threads/'+encodeURIComponent(sourceThreadId)+'/handoffs', {
          target_thread_id: workbench.sessionId || sourceThreadId,
          title: document.getElementById('knowledge-artifact-title')?.value || undefined,
          max_chars: knowledgeMaxChars()
        });
        document.getElementById('knowledge-transfer-modal')?.close();
        workbenchMessage('system', 'Handoff created', 'Created handoff output '+payload.output.output_id+' and reference '+payload.reference.reference_id+'.');
        workbench.outputs = [];
      } catch (error) {
        setKnowledgeTransferStatus(error.message || 'Could not create handoff.');
      }
    }
    async function importKnowledgeOutput() {
      if (!workbench.sessionId) {
        setKnowledgeTransferStatus('Select or create a target thread first.');
        return;
      }
      const outputId = selectedKnowledgeOutputId();
      if (!outputId) {
        setKnowledgeTransferStatus('Choose an output to import.');
        return;
      }
      try {
        const payload = await postJsonPayload('/v1/threads/'+encodeURIComponent(workbench.sessionId)+'/artifact-imports', {
          source_output_id: outputId,
          max_chars: knowledgeMaxChars()
        });
        document.getElementById('knowledge-transfer-modal')?.close();
        workbenchMessage('system', 'Output imported', 'Imported output through reference '+payload.reference.reference_id+'. Excerpt:\n\n'+(payload.excerpt || 'No preview available.'));
      } catch (error) {
        setKnowledgeTransferStatus(error.message || 'Could not import output.');
      }
    }
    async function selectWorkbenchThread(sessionId) {
      const session = workbench.sessions.find((item) => item.session_id === sessionId);
      if (!session) return;
      const generation = bumpWorkbenchThreadGeneration();
      resetWorkbenchConnection();
      workbench.sessionId = session.session_id;
      workbench.projectId = session.project_id || null;
      workbench.repositoryId = session.repository_id || null;
      workbench.workspacePath = session.workspace_path;
      setWorkbenchThreadTitle(session.title || 'Existing chat thread');
      setComposerVisible(true);
      setWorkbenchStatus('not connected', false);
      clearWorkbenchTranscript();
      renderWorkbenchProjects();
      try {
        await loadWorkbenchThreadHistory(session.session_id, generation);
      } catch (error) {
        if (isActiveWorkbenchGeneration(generation)) workbenchMessage('system', 'History', error.message);
      }
    }
    async function startWorkbenchSession() {
      const generation = workbench.activeThreadGeneration;
      try {
        const workspace = workbench.workspacePath;
        if (!workspace) {
          workbenchMessage('system', 'Workspace', 'Choose an assigned workspace before starting a session.');
          return;
        }
        if (!workbench.sessionId) {
          setWorkbenchStatus('creating thread', false);
          if (!workbench.projectId) throw new Error('Choose a project before starting a thread.');
          const sessionTitle = workbench.threadTitle && workbench.threadTitle !== 'Local Codex Chat' ? workbench.threadTitle : 'New chat thread';
          const session = await postJsonPayload('/v1/projects/'+encodeURIComponent(workbench.projectId)+'/threads', {repository_id:workbench.repositoryId, title:sessionTitle});
          workbench.sessionId = session.session.session_id;
          workbench.workspacePath = session.session.workspace_path;
          workbench.projectId = session.session.project_id;
          workbench.repositoryId = session.session.repository_id;
          setWorkbenchThreadTitle(session.session.title || sessionTitle);
          await refreshWorkbenchThreads();
          workbench.sessionId = session.session.session_id;
        }
        if (!isActiveWorkbenchGeneration(generation)) return;
        setWorkbenchStatus('starting worker', false);
        const worker = await postJsonPayload('/v1/threads/'+encodeURIComponent(workbench.sessionId)+'/workers', {});
        if (!isActiveWorkbenchGeneration(generation)) return;
        workbench.workerId = worker.worker.worker_id;
        const handoff = await postJsonPayload('/v1/workers/'+encodeURIComponent(workbench.workerId)+'/handoffs', {});
        if (!isActiveWorkbenchGeneration(generation)) return;
        await connectWorkbenchSocket(handoff.handoff_token, generation);
      } catch (error) {
        if (!isActiveWorkbenchGeneration(generation)) return;
        setWorkbenchStatus('not connected', false);
        workbenchMessage('system', 'Start failed', error.message);
      }
    }
    async function connectWorkbenchSocket(handoffToken, generation) {
      if (!workbench.workerId || !handoffToken) return;
      const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
      const url = protocol+'//'+window.location.host+'/v1/workers/'+encodeURIComponent(workbench.workerId)+'/rpc?handoff_token='+encodeURIComponent(handoffToken);
      if (workbench.socket) {
        workbench.socket.onclose = null;
        workbench.socket.close();
      }
      workbench.rpcReady = false;
      workbench.appThreadId = null;
      Object.values(workbench.pendingRpc).forEach((pending) => pending.reject(new Error('Worker websocket was replaced.')));
      workbench.pendingRpc = {};
      workbench.socket = new WebSocket(url);
      const socket = workbench.socket;
      return new Promise((resolve, reject) => {
        let opened = false;
        socket.onopen = () => {
          if (!isActiveWorkbenchGeneration(generation)) {
            socket.close();
            return;
          }
          opened = true;
          setWorkbenchStatus('worker socket open', true);
          resolve(socket);
        };
        socket.onmessage = (event) => {
          if (!isActiveWorkbenchGeneration(generation) || socket !== workbench.socket) return;
          handleWorkbenchRpcMessage(event.data);
        };
        socket.onerror = () => {
          if (!isActiveWorkbenchGeneration(generation) || socket !== workbench.socket) return;
          setWorkbenchStatus('connection error', false);
          workbenchMessage('system', 'Websocket', 'The browser websocket reported an error.');
          if (!opened) reject(new Error('The browser websocket reported an error.'));
        };
        socket.onclose = () => {
          if (!isActiveWorkbenchGeneration(generation) || socket !== workbench.socket) return;
          setWorkbenchStatus('disconnected', false);
          workbench.rpcReady = false;
          Object.values(workbench.pendingRpc).forEach((pending) => pending.reject(new Error('Worker websocket disconnected.')));
          workbench.pendingRpc = {};
          if (!opened) reject(new Error('The browser websocket closed before connecting.'));
        };
      });
    }
    async function ensureWorkbenchConnected() {
      if (workbench.socket && workbench.socket.readyState === WebSocket.OPEN) {
        if (!workbench.rpcReady) await initializeWorkbenchRpc();
        return workbench.rpcReady;
      }
      if (!workbench.connecting) {
        workbench.connecting = startWorkbenchSession().finally(() => { workbench.connecting = null; });
      }
      await workbench.connecting;
      if (workbench.socket && workbench.socket.readyState === WebSocket.OPEN && !workbench.rpcReady) {
        await initializeWorkbenchRpc();
      }
      return Boolean(workbench.socket && workbench.socket.readyState === WebSocket.OPEN && workbench.rpcReady);
    }
    function resetWorkbenchConnection() {
      if (workbench.socket) {
        workbench.socket.onclose = null;
        workbench.socket.close();
      }
      workbench.socket = null;
      workbench.workerId = null;
      workbench.rpcReady = false;
      workbench.appThreadId = null;
      Object.values(workbench.pendingRpc).forEach((pending) => pending.reject(new Error('Worker websocket was reset.')));
      workbench.pendingRpc = {};
      workbench.connecting = null;
    }
    function nextWorkbenchRpcId(prefix) {
      workbench.rpcCounter += 1;
      return prefix+'-'+String(workbench.rpcCounter);
    }
    function sendWorkbenchRpcRequest(method, params) {
      if (!workbench.socket || workbench.socket.readyState !== WebSocket.OPEN) {
        return Promise.reject(new Error('This chat thread is not connected to a worker yet.'));
      }
      const id = nextWorkbenchRpcId(method.replaceAll('/', '-'));
      const payload = {id, method, params};
      return new Promise((resolve, reject) => {
        workbench.pendingRpc[id] = {resolve, reject, method};
        workbench.socket.send(JSON.stringify(payload));
      });
    }
    function sendWorkbenchRpcNotification(method, params) {
      if (!workbench.socket || workbench.socket.readyState !== WebSocket.OPEN) return;
      const payload = params === undefined ? {method} : {method, params};
      workbench.socket.send(JSON.stringify(payload));
    }
    async function initializeWorkbenchRpc() {
      await sendWorkbenchRpcRequest('initialize', {
        clientInfo: {
          name: 'local-codex-enterprise-browser',
          title: 'Local Codex Enterprise Browser Chat',
          version: '0.0.1-beta.7'
        },
        capabilities: {
          experimentalApi: true,
          requestAttestation: false,
          optOutNotificationMethods: []
        }
      });
      sendWorkbenchRpcNotification('initialized');
      workbench.rpcReady = true;
      setWorkbenchStatus('connected to worker', true);
    }
    async function startWorkbenchRpcThread() {
      const response = await sendWorkbenchRpcRequest('thread/start', {
        cwd: workbench.workspacePath,
        runtimeWorkspaceRoots: [workbench.workspacePath]
      });
      workbench.appThreadId = response.thread?.id || response.threadId || null;
      if (!workbench.appThreadId) throw new Error('Worker did not return an app-server thread id.');
      return workbench.appThreadId;
    }
    function handleWorkbenchRpcMessage(raw) {
      let message;
      try {
        message = JSON.parse(raw);
      } catch (_) {
        workbenchMessage('assistant', 'Worker', raw);
        return;
      }
      if (message.id && workbench.pendingRpc[message.id]) {
        const pending = workbench.pendingRpc[message.id];
        delete workbench.pendingRpc[message.id];
        if (message.error) {
          pending.reject(new Error(message.error.message || pending.method+' failed'));
        } else {
          pending.resolve(message.result || {});
        }
        return;
      }
      handleWorkbenchRpcNotification(message);
    }
    function handleWorkbenchRpcNotification(message) {
      const params = message.params || {};
      switch (message.method) {
        case 'item/agentMessage/delta':
          if (params.delta) replacePendingAssistantDelta(params.delta);
          break;
        case 'item/commandExecution/outputDelta':
        case 'command/exec/outputDelta':
          handleWorkbenchCommandOutputDelta(params.delta);
          break;
        case 'turn/started':
          setWorkbenchStatus('working', true);
          workbench.pendingAssistantText = '';
          workbench.commandOutputChars = 0;
          break;
        case 'turn/completed':
          setWorkbenchStatus('connected to worker', true);
          if (workbench.pendingAssistantText.trim()) {
            const assistantText = workbench.pendingAssistantText;
            workbench.pendingAssistantText = '';
            completeWorkbenchAssistantTurn();
            (async () => {
              const knowledgeReferenceId = workbench.pendingKnowledgeReferenceId;
              workbench.pendingKnowledgeReferenceId = null;
              const saved = await recordWorkbenchThreadMessage('assistant', 'Codex', assistantText);
              const chat = document.getElementById('workbench-chat');
              chat?.attachLastPersistedMessage?.('assistant', 'Codex', saved?.message);
              if (knowledgeReferenceId && saved?.message?.message_id && Array.isArray(chat?.messages)) {
                const last = chat.messages.findLast((message) => message.message_id === saved.message.message_id);
                if (last) last.threadReferenceId = knowledgeReferenceId;
              }
              await autoLabelWorkbenchThreadAfterFirstAiTurn();
            })().catch((error) => {
              workbenchMessage('system', 'History', 'Could not save Codex reply: '+error.message);
            });
          }
          break;
        case 'error':
          workbenchMessage('system', 'Worker error', params.message || 'The worker reported an error.');
          break;
        case 'warning':
        case 'configWarning':
          workbenchMessage('system', 'Worker warning', params.message || 'The worker reported a warning.');
          break;
        default:
          break;
      }
    }
    async function sendWorkbenchRpcMessage(text, options) {
      const settings = Object.assign({recordUser:true, appendUser:true}, options || {});
      if (!workbench.appThreadId) {
        await startWorkbenchRpcThread();
      }
      const modelText = settings.rawPrompt ? text : buildWorkbenchUserPrompt(text);
      if (settings.appendUser) workbenchMessage('user', 'You', text);
      if (settings.recordUser) {
        const saved = await recordWorkbenchThreadMessage('user', 'You', text);
        document.getElementById('workbench-chat')?.attachLastPersistedMessage?.('user', 'You', saved?.message);
      }
      appendWorkbenchAssistantPending();
      await sendWorkbenchRpcRequest('turn/start', {
        threadId: workbench.appThreadId,
        clientUserMessageId: null,
        input: [{type:'text', text:modelText, textElements: []}]
      });
    }
    function buildWorkbenchUserPrompt(text) {
      const responsePreferences = workbenchResponsePreferenceGuidance();
      if (workbench.repositoryId) return responsePreferences ? responsePreferences+'User request:\n'+text : text;
      if (isSocialWorkbenchMessage(text)) {
        return responsePreferences+'Conversational acknowledgement. Reply briefly and naturally. Do not start planning, inspect the repository, mention AGENTS.md, or ask what planning is needed. Do not mention unavailable tools or internal tool names.\\n\\nUser message:\\n'+text;
      }
      const guidance = workbenchTurnGuidance();
      if (!isPlanningWorkbenchMessage(text)) {
        return responsePreferences+'General chat request. Answer directly and concisely. Do not inspect the repository unless the user explicitly asks about the current codebase. Do not mention unavailable tools or internal tool names. '+(guidance.repository_tool_rule || '')+' '+(guidance.tool_output_rule || '')+'\\n\\nUser request:\\n'+text;
      }
      const planning = Array.isArray(guidance.planning_sequence)
        ? guidance.planning_sequence.map((item, index) => String(index + 1)+'. '+item).join(' ')
        : '1. business goal 2. users/stakeholders 3. decisions the system must support 4. data sources 5. architecture 6. implementation path';
      return responsePreferences+'Conceptual planning request. This Local Codex project does not currently have a selected repository. '+(guidance.repository_tool_rule || '')+' '+(guidance.tool_output_rule || '')+' Start planning tasks with: '+planning+'.\\n\\nUser request:\\n'+text;
    }
    function workbenchResponsePreferenceGuidance() {
      const summary = String(workbench.responsePreferences?.profile_summary || '').trim();
      if (!summary) return '';
      return 'User response preferences:\\n'+summary+'\\nThese preferences affect style and routing behavior only, not factual truth.\\n\\n';
    }
    function isSocialWorkbenchMessage(text) {
      const normalized = String(text || '').trim().toLowerCase().replace(/[.!?]+$/g, '');
      if (!normalized) return false;
      if (/^(thanks|thank you|thx|ty|appreciate it|got it|ok|okay|cool|nice|great|awesome|perfect|sounds good|that works|good deal|yep|yes|no problem|np)(\s+(thanks|thank you|again|codex|sir|man|bro|friend))?$/.test(normalized)) return true;
      return normalized.length <= 32 && /^(hi|hello|hey|yo|sup|good morning|good afternoon|good evening)$/.test(normalized);
    }
    function isPlanningWorkbenchMessage(text) {
      const normalized = String(text || '').toLowerCase();
      return /(plan|planning|strategy|architecture|architect|roadmap|design|proposal|approach|requirements|solution|system|workflow|implementation path|business goal|stakeholder|dashboard|portal|app|build|feature|product|project plan)/.test(normalized);
    }
    async function retryWorkbenchMessageOnce(text, firstError) {
      workbenchMessage('system', 'Reconnecting', firstError.message || 'Worker websocket disconnected. Starting a fresh handoff.');
      resetWorkbenchConnection();
      await ensureWorkbenchConnected();
      await sendWorkbenchRpcMessage(text);
    }
    async function sendWorkbenchMessage() {
      const input = document.getElementById('composer-input');
      const text = input?.value.trim();
      if (!text) return;
      input.disabled = true;
      try {
        try {
          await ensureWorkbenchConnected();
          await sendWorkbenchRpcMessage(text);
        } catch (firstError) {
          await retryWorkbenchMessageOnce(text, firstError);
        }
        input.value = '';
      } catch (error) {
        workbenchMessage('system', 'Not connected', error.message || 'Could not connect this chat thread to the worker.');
      } finally {
        input.disabled = false;
        input.focus();
      }
    }
    function composerKeydown(event) {
      if (event.key !== 'Enter' || event.shiftKey) return;
      event.preventDefault();
      sendWorkbenchMessage();
    }
    document.addEventListener('DOMContentLoaded', () => {
      if (document.getElementById('user-index') || document.getElementById('workspace-index') || document.getElementById('context-pack-index') || document.getElementById('assignment-index') || document.getElementById('output-index') || document.getElementById('project-index') || document.getElementById('user-pack-select') || document.getElementById('pack-user-select') || document.getElementById('output-user-select') || document.getElementById('project-user-select') || document.getElementById('role-user-select') || document.getElementById('status-user-select')) {
        refreshAdminChoices();
      }
      loadWorkbench();
    });
  </script>
</body>
</html>"#
}

fn setup_page(config: &EnterpriseConfig) -> String {
    let default_workspace_root =
        html_escape(config.default_workspace_root.as_deref().unwrap_or(""));
    format!(
        r##"
    <section>
      <h2>Initial Admin Account</h2>
      <label>Admin email<input id="setup-email" autocomplete="username"></label>
      <label>Admin password<input id="setup-password" type="password" autocomplete="new-password"></label>
      <label>Initial allowed workspace roots<textarea id="setup-roots">{default_workspace_root}</textarea></label>
      <p class="hint">Docker Compose: use /enterprise-workspaces. Local install: use a path that exists on the server running Local Codex for Enterprise.</p>
      <button onclick="postJson('/v1/setup/enterprise',{{owner_email:v('setup-email'),owner_password:v('setup-password'),workspace_roots:lines('setup-roots')}},'/login')">Bootstrap Admin</button>
    </section>"##
    )
}

fn login_page() -> &'static str {
    r#"
    <section>
      <h2>Credentials</h2>
      <label>Email<input id="login-email" autocomplete="username"></label>
      <label>Password<input id="login-password" type="password" autocomplete="current-password"></label>
      <button onclick="postJson('/v1/auth/browser-login',{email:v('login-email'),password:v('login-password')},'/chat')">Sign In</button>
    </section>"#
}

fn admin_overview_page(role: Option<EnterpriseRole>) -> String {
    if matches!(role, Some(EnterpriseRole::Manager)) {
        return admin_console_page_for(
            role,
            r#"
      <section>
        <h2>Manager Console</h2>
        <p class="hint">Managers can assign user reports, review trace evidence, use chat, and work with explicitly granted workspaces.</p>
        <div class="grid">
          <section>
            <h2>Reports</h2>
            <p>Assign read-only output metadata to users.</p>
            <div class="toolbar"><a href="/admin/outputs">Open Reports</a></div>
          </section>
          <section>
            <h2>Audit</h2>
            <p>Review trace and receipt evidence for permitted operational work.</p>
            <div class="toolbar"><a href="/admin/audit">Open Audit</a></div>
          </section>
        </div>
      </section>"#,
        );
    }
    admin_console_page_for(
        role,
        r#"
      <section>
        <h2>Control Plane</h2>
        <p class="hint">Use the navigation tree to manage identity, access, workspace allowlists, governed context, runtime sessions, and evidence records.</p>
        <div class="grid">
          <section>
            <h2>Identity</h2>
            <p>Manage people, seeded roles, and account status.</p>
            <div class="toolbar"><a href="/admin/users">Open Users</a><a class="secondary" href="/admin/rbac">Open RBAC</a></div>
          </section>
          <section>
            <h2>Workspaces</h2>
            <p>Register server-visible roots for user workspace boundaries. Repositories are added to projects.</p>
            <div class="toolbar"><a href="/admin/workspaces">Open Workspaces</a></div>
          </section>
          <section>
            <h2>Governed Context</h2>
            <p>Upload Context Pack operating packages and assign them to users or workspaces.</p>
            <div class="toolbar"><a href="/admin/context-packs">Open Context Packs</a></div>
          </section>
          <section>
            <h2>Demo Data</h2>
            <p>Optional sample users, a full demo Context Pack, and a workspace assignment for local review.</p>
            <button onclick="postJson('/v1/demo-data',{})">Load Demo Data</button>
          </section>
        </div>
      </section>"#,
    )
}

fn users_page() -> String {
    admin_console_page(
        r#"
      <section>
        <h2>Index</h2>
        <p class="hint">Manage users from their action pages. This list loads automatically from the admin API.</p>
        <div class="toolbar">
          <a href="/admin/users/new">Create User</a>
          <a class="secondary" href="/admin/users/status">Deactivate Or Reactivate</a>
          <a class="secondary" href="/admin/users/projects">Manage Projects</a>
          <a class="secondary" href="/admin/users/context-packs">Assign Context Packs</a>
        </div>
        <div id="user-index"></div>
      </section>"#,
    )
}

fn user_create_page() -> String {
    admin_console_page(
        r#"
      <section>
        <h2>Create User</h2>
        <label>Email<input id="user-email" autocomplete="off"></label>
        <label>Password<input id="user-password" type="password" autocomplete="new-password"></label>
        <label>Role<select id="user-role"><option>developer</option><option>viewer</option><option>manager</option><option>admin</option></select></label>
        <label>Per-user workspace roots<textarea id="user-workspace-roots"></textarea></label>
        <p class="hint">Leave blank to use the default user workspace, for example /enterprise-workspaces/user/alex@example.com. Admins can override this by entering one or more explicit allowed workspace paths.</p>
        <button onclick="postJson('/v1/users',{email:v('user-email'),password:v('user-password'),role:v('user-role'),workspace_roots:lines('user-workspace-roots')},'/admin/users')">Create User</button>
      </section>"#,
    )
}

fn user_status_page() -> String {
    admin_console_page(
        r#"
      <section>
        <h2>User Status</h2>
        <div id="user-empty-state" hidden></div>
        <label>User<select id="status-user-select"></select></label>
        <div class="actions">
          <button id="user-status-deactivate-submit" class="danger" onclick="postJson('/v1/users/'+encodeURIComponent(v('status-user-select'))+'/deactivations',{})">Deactivate</button>
          <button id="user-status-reactivate-submit" class="secondary" onclick="postJson('/v1/users/'+encodeURIComponent(v('status-user-select'))+'/reactivations',{})">Reactivate</button>
        </div>
        <p class="hint">The final active admin is protected by the server. Admins cannot deactivate themselves.</p>
      </section>"#,
    )
}

fn user_projects_page() -> String {
    admin_console_page(
        r#"
      <section>
        <h2>User Projects</h2>
        <p class="hint">Choose a user, then create or manage projects inside that user's assigned workspace. Projects do not require repositories; repositories are added later from the project menu.</p>
        <div id="user-empty-state" hidden></div>
        <label>User<select id="project-user-select" onchange="refreshProjectsForSelectedUser()"></select></label>
        <div id="project-workspace-empty-state" hidden></div>
        <label>User workspace<select id="project-user-workspace-select"></select></label>
        <label>New project name<input id="project-name" placeholder="Client Portal"></label>
        <button id="project-create-submit" onclick="createProjectForSelectedUser()">Create Project</button>
      </section>
      <section>
        <h2>Existing Projects</h2>
        <div id="project-empty-state" hidden></div>
        <label>Project<select id="project-edit-select"></select></label>
        <label>Rename project<input id="project-rename" placeholder="Updated project name"></label>
        <div class="actions">
          <button id="project-rename-submit" onclick="renameSelectedProject()">Rename Project</button>
          <button id="project-delete-submit" class="danger" onclick="deleteSelectedProject()">Remove Project</button>
        </div>
        <div id="project-index"></div>
      </section>
      <section>
        <h2>Archived projects</h2>
        <p class="hint">Removed projects are hidden from chat and normal project lists. Admins can view and restore them here.</p>
        <label>Archived project<select id="project-restore-select"></select></label>
        <button id="project-restore-submit" onclick="restoreSelectedProject()">Restore Project</button>
        <div id="project-archive-index"></div>
      </section>"#,
    )
}

fn user_context_packs_page() -> String {
    admin_console_page(
        r#"
      <section>
        <h2>Assign Context Packs</h2>
        <p class="hint">Choose a user, then select multiple context packs. Load order starts at the value below and increments deterministically for multiselect assignments.</p>
        <div id="user-empty-state" hidden></div>
        <div id="pack-empty-state" hidden></div>
        <div id="workspace-empty-state" hidden></div>
        <label>User<select id="user-assignment-user-select"></select></label>
        <label>Context packs <span class="muted">(select multiple)</span><select id="user-pack-select" multiple></select></label>
        <label>Workspace<select id="user-workspace-select"></select></label>
        <label>Starting load order<input id="user-assign-order" value="10"></label>
        <button id="user-pack-assign-submit" onclick="assignPacksFromUserPage()">Assign Selected Packs</button>
      </section>
      <section>
        <h2>Current Assignments</h2>
        <div id="assignment-index"></div>
      </section>"#,
    )
}

fn rbac_page() -> String {
    admin_console_page(&format!(
        r#"
      <section>
        <h2>RBAC Role Assignment CRUD</h2>
        <p class="hint">Roles are seeded in v1, but assignments are managed here. Create or update an assignment by choosing a role; remove an elevated assignment by setting the user back to viewer. At least one active admin must always remain, and admins cannot remove their own admin role.</p>
        <div id="user-empty-state" hidden></div>
        <label>User<select id="role-user-select"></select></label>
        <label>Role<select id="role-value"><option>developer</option><option>viewer</option><option>manager</option><option>admin</option></select></label>
        <div class="actions">
          <button id="role-assign-submit" onclick="postJson('/v1/users/'+encodeURIComponent(v('role-user-select'))+'/role',{{role:v('role-value')}})">Create assignment / Update assignment</button>
          <button id="role-remove-submit" class="secondary" onclick="postJson('/v1/users/'+encodeURIComponent(v('role-user-select'))+'/role',{{role:'viewer'}})">Remove assignment</button>
        </div>
      </section>
      <section>
        <h2>Effective Permission Matrix</h2>
        <p class="hint">This is the active seeded policy used by the API authorization checks.</p>
        {}
      </section>"#,
        rbac_permission_matrix_html()
    ))
}

fn workspaces_page() -> String {
    admin_console_page(
        r#"
      <section>
        <h2>Workspace Index</h2>
        <p class="hint">Workspace roots listed here are visible to the server runtime. Docker Compose paths may differ from host paths.</p>
        <div class="toolbar">
          <a href="/admin/workspaces/register">Register Workspace</a>
          <a class="secondary" href="/admin/workspaces/validate">Validate Path</a>
        </div>
        <div id="workspace-index"></div>
      </section>"#,
    )
}

fn workspace_register_page(config: &EnterpriseConfig) -> String {
    let default_workspace_root =
        html_escape(config.default_workspace_root.as_deref().unwrap_or(""));
    admin_console_page(&format!(
        r#"
      <section>
        <h2>Register Workspace</h2>
        <label>Allowed workspace root<input id="workspace-root" value="{default_workspace_root}"></label>
        <p class="hint">Docker Compose: use /enterprise-workspaces. Local install: use a path that exists on the server running Local Codex for Enterprise.</p>
        <button onclick="postJson('/v1/workspace-roots',{{root_path:v('workspace-root')}},'/admin/workspaces')">Register Workspace</button>
      </section>"#
    ))
}

fn workspace_validate_page() -> String {
    admin_console_page(
        r#"
      <section>
        <h2>Validate Workspace</h2>
        <label>Workspace path<input id="workspace-validate"></label>
        <button onclick="postJson('/v1/workspace-path-validations',{workspace_path:v('workspace-validate')})">Validate Workspace</button>
      </section>"#,
    )
}

fn context_packs_page() -> String {
    admin_console_page(
        r#"
      <section>
        <h2>Context Pack Index</h2>
        <p class="hint">Context Packs are versioned lifecycle packages for session guidance. They may include operating documents, reusable templates, outputs, and optional Codex skill files. Importing or loading a pack does not automatically execute skills, run workflows, change RBAC, or run governance reasoning.</p>
        <div class="toolbar">
          <a href="/admin/context-packs/new">Create Pack</a>
          <a class="secondary" href="/admin/context-packs/import">Import Folder Or Files</a>
          <a class="secondary" href="/admin/context-packs/files">Manage Files</a>
          <a class="secondary" href="/admin/context-packs/assignments">Manage Assignments</a>
        </div>
        <div id="context-pack-index"></div>
      </section>"#,
    )
}

fn context_pack_create_page() -> String {
    admin_console_page(
        r#"
      <section>
        <h2>Create Context Pack</h2>
        <p class="hint">Context Packs are versioned lifecycle packages. They may contain or reference Codex skill files, but a Context Pack is not itself the runtime. Importing or loading a pack does not automatically execute skills, call tools, create sessions, alter RBAC, or run governance reasoning. Canonical files are listed below; custom uppercase Markdown files such as CUSTOM-STANDARD.md can be imported from a folder.</p>
        <label>Name<input id="pack-name" value="Standard Engineering Pack"></label>
        <label>PACK.md<textarea id="pack-manifest">name: Standard Engineering Pack
version: 1
required_documents:
- CALIBRATION.md
- OPERATING-INSTRUCTIONS.md
- PROJECT-RULES.md
- WORKFLOWS.md
- HANDOFF.md
- VERIFICATION.md
- ESCALATION.md
- CONTEXT.md
- PROMPTS.md
load_order:
- PACK.md
- CALIBRATION.md
- OPERATING-INSTRUCTIONS.md
- PROJECT-RULES.md
- WORKFLOWS.md
- HANDOFF.md
- VERIFICATION.md
- ESCALATION.md
- CONTEXT.md
- PROMPTS.md</textarea></label>
        <label>CALIBRATION.md<textarea id="pack-calibration"># Calibration

Treat the user as the accountable project owner. Preserve their intent, ask for missing context only when needed, and raise concerns with concrete tradeoffs.</textarea></label>
        <label>OPERATING-INSTRUCTIONS.md<textarea id="pack-operating-instructions"># Operating Instructions

Read the required session-start context, follow repository instructions, keep work tied to a real outcome, and continue until verified or truly blocked.</textarea></label>
        <label>PROJECT-RULES.md<textarea id="pack-project-rules"># Project Rules

Respect auth, RBAC, workspace allowlisting, trace receipts, and local-only boundaries. Do not bypass controls to force a task through.</textarea></label>
        <label>WORKFLOWS.md<textarea id="pack-workflows"># Workflows

Describe task procedures and workflow guidance as inert operating material. Do not define executable workflows, schedules, agents, or tool calls here.</textarea></label>
        <label>HANDOFF.md<textarea id="pack-handoff"># Handoff

Record current status, completed work, open decisions, validation evidence, and next action. Do not include secrets, private examples, private/runtime prompts, or model outputs.</textarea></label>
        <label>VERIFICATION.md<textarea id="pack-verification"># Verification

Run focused tests for changed behavior, then the relevant package validation. Report exact results and any unverified surfaces.</textarea></label>
        <label>ESCALATION.md<textarea id="pack-escalation"># Escalation

Escalate only when a decision, access boundary, or safety issue cannot be resolved locally. State the minimum needed action and the risk of proceeding.</textarea></label>
        <label>CONTEXT.md<textarea id="pack-context">Follow repository instructions and verify before completion.</textarea></label>
        <label>PROMPTS.md<textarea id="pack-prompts"># Prompt Templates

Reusable prompt templates may live here as inert text assets. Sessions or schedules choose whether to use a template; templates do not execute by themselves.</textarea></label>
        <button onclick="postJson('/v1/context-packs',{name:v('pack-name'),documents:safeDocumentsFromTextareas()},'/admin/context-packs')">Create Pack</button>
      </section>"#,
    )
}

fn context_pack_import_page() -> String {
    admin_console_page(
        r#"
      <section>
        <h2>Import Folder Or Files</h2>
        <p class="hint">Canonical names: PACK.md, CALIBRATION.md, OPERATING-INSTRUCTIONS.md, PROJECT-RULES.md, WORKFLOWS.md, VERIFICATION.md, HANDOFF.md, ESCALATION.md, CONTEXT.md, PROMPTS.md. Custom uppercase Markdown files such as CUSTOM-STANDARD.md are also allowed.</p>
        <label>Context pack folder or markdown files<input id="pack-files" type="file" accept=".md,text/markdown" multiple webkitdirectory></label>
        <button onclick="createPackFromSelectedFiles()">Import Pack</button>
      </section>"#,
    )
}

fn context_pack_files_page() -> String {
    admin_console_page(
        r#"
      <section>
        <h2>Manage Context Pack Files</h2>
        <p class="hint">Context Pack bundle files may be included but not auto-executed. Stored files are hashed, audited, downloadable package contents; only active loadable text files are injected as session context.</p>
        <div id="pack-file-empty-state" hidden></div>
        <label>Context pack<select id="pack-file-pack-select" onchange="refreshContextPackFiles()"></select></label>
        <label>Files or bundle folder<input id="pack-file-input" type="file" multiple webkitdirectory></label>
        <label>File kind<select id="pack-file-kind"><option value="">Infer from path</option><option value="document">document</option><option value="bundle">bundle</option><option value="asset">asset</option></select></label>
        <label>Source type<select id="pack-file-source"><option value="upload">upload</option><option value="manual">manual</option><option value="import">import</option></select></label>
        <label><input id="pack-file-loadable" type="checkbox"> Force loadable</label>
        <label>Starting load order<input id="pack-file-load-order" value="100"></label>
        <button id="pack-file-upload-submit" onclick="uploadContextPackFiles()">Upload/Register Files</button>
      </section>
      <section>
        <h2>Current Files</h2>
        <div id="context-pack-file-index"></div>
      </section>"#,
    )
}

fn context_pack_assignments_page() -> String {
    admin_console_page(
        r#"
      <section>
        <h2>Assign To Users</h2>
        <p class="hint">Choose a context pack, then select one or more users and one workspace. Add missing prerequisites before assigning context.</p>
        <div id="context-pack-empty-state" hidden></div>
        <div id="pack-workspace-empty-state" hidden></div>
        <label>Context pack<select id="pack-assignment-pack-select"></select></label>
        <label>Users <span class="muted">(select multiple)</span><select id="pack-user-select" multiple></select></label>
        <label>Workspace<select id="pack-workspace-select"></select></label>
        <label>Starting load order<input id="pack-assign-order" value="10"></label>
        <button id="pack-assign-submit" onclick="assignUsersFromPackPage()">Assign Selected Users</button>
      </section>
      <section>
        <h2>Current Assignments</h2>
        <div id="assignment-index"></div>
      </section>"#,
    )
}

fn admin_console_page(content: &str) -> String {
    admin_console_page_for(Some(EnterpriseRole::Admin), content)
}

fn admin_console_page_for(role: Option<EnterpriseRole>, content: &str) -> String {
    format!(
        r##"
    <div class="console-layout">
      {}
      <div>{}</div>
    </div>"##,
        admin_tree(role),
        content
    )
}

fn admin_tree(role: Option<EnterpriseRole>) -> String {
    if matches!(role, Some(EnterpriseRole::Manager)) {
        return r#"
      <section class="tree-nav">
        <h2>Manager Console</h2>
        <h3>Permitted sections</h3>
        <div class="tree-group">
          <strong>Evidence</strong>
          <a href="/admin/outputs">Reports and outputs</a>
          <a href="/admin/audit">Audit</a>
        </div>
        <div class="tree-group">
          <strong>Work surface</strong>
          <a href="/chat">Chat</a>
          <a href="/app/terminal">Terminal instructions</a>
        </div>
      </section>"#
            .to_string();
    }
    r#"
      <section class="tree-nav">
        <h2>Admin Console</h2>
        <h3>Service tree</h3>
        <div class="tree-group">
          <strong>Identity</strong>
          <a href="/admin/users">Users index</a>
          <a href="/admin/users/new">Create user</a>
          <a href="/admin/users/projects">User projects</a>
          <a href="/admin/rbac">RBAC role assignment</a>
          <a href="/admin/users/status">User status</a>
        </div>
        <div class="tree-group">
          <strong>Workspaces</strong>
          <a href="/admin/workspaces">Workspace index</a>
          <a href="/admin/workspaces/register">Register workspace</a>
          <a href="/admin/workspaces/validate">Validate path</a>
        </div>
        <div class="tree-group">
          <strong>Governed Context</strong>
          <a href="/admin/context-packs">Context pack index</a>
          <a href="/admin/context-packs/new">Create pack</a>
          <a href="/admin/context-packs/import">Import pack</a>
          <a href="/admin/users/context-packs">Assign packs by user</a>
          <a href="/admin/context-packs/assignments">Assign users by pack</a>
        </div>
        <div class="tree-group">
          <strong>Evidence</strong>
          <a href="/admin/outputs">Reports and outputs</a>
          <a href="/admin/audit">Audit</a>
        </div>
      </section>"#
        .to_string()
}

fn app_page(_config: &EnterpriseConfig) -> String {
    r#"
    <section>
      <h2>Browser Workbench</h2>
      <p class="hint">Use Chat for project threads, repository clone actions, workers, and local model turns. Terminal instructions remain available for shell-based use.</p>
      <div class="toolbar">
        <a href="/chat">Open Chat</a>
        <a class="secondary" href="/app/terminal">Terminal instructions</a>
        <a class="secondary" href="/app/outputs">My outputs</a>
      </div>
    </section>"#
        .to_string()
}

fn chat_page(
    config: &EnterpriseConfig,
    principal: Option<&crate::storage::AuthPrincipal>,
) -> String {
    let default_workspace_root =
        html_escape(config.default_workspace_root.as_deref().unwrap_or(""));
    let turn_guidance_json = html_escape(
        &serde_json::to_string(&turn_guidance_response()).unwrap_or_else(|_| "{}".to_string()),
    );
    let admin_menu_item = principal
        .filter(|principal| role_has_admin_console(principal.role))
        .map(|_| r#"<a href="/admin">Admin / settings</a>"#.to_string())
        .unwrap_or_default();
    format!(
        r#"
    <style>
      header, #page-intro, #result-panel, footer {{ display: none; }}
      body {{ background: #050507; color: #d7dce7; overflow: hidden; }}
      main {{ max-width: none; margin: 0; padding: 0; height: 100vh; }}
      .chat-shell-fullscreen {{ --lc-accent: AccentColor; --lc-accent-text: AccentColorText; --chat-content-width: 980px; position: fixed; inset: 0; overflow: hidden; height: 100vh; min-height: 0; display: grid; grid-template-columns: 320px minmax(0, 1fr); background: #050507; color: #d7dce7; }}
      @supports not (color: AccentColor) {{ .chat-shell-fullscreen {{ --lc-accent: #9df2b4; --lc-accent-text: #061009; }} }}
      .chat-shell-fullscreen.chat-rail-collapsed {{ grid-template-columns: 44px minmax(0, 1fr); }}
      .chat-shell-fullscreen.chat-rail-collapsed .chat-rail {{ padding-inline: 6px; }}
      .chat-shell-fullscreen.chat-rail-collapsed .rail-action,
      .chat-shell-fullscreen.chat-rail-collapsed .rail-section-title span:first-child,
      .chat-shell-fullscreen.chat-rail-collapsed #workbench-project-list,
      .chat-shell-fullscreen.chat-rail-collapsed .chat-rail-footer,
      .chat-shell-fullscreen.chat-rail-collapsed .projects-menu-button,
      .chat-shell-fullscreen.chat-rail-collapsed .projects-new-button {{ display: none; }}
      .chat-rail {{ border-right: 1px solid #24262e; background: #08090d; padding: 14px 8px; display: flex; flex-direction: column; min-height: 0; }}
      .rail-collapse-button {{ margin: 0 0 12px 6px; }}
      .chat-rail-footer {{ margin-top: auto; border-top: 1px solid #22242b; padding: 12px 14px 6px; display: grid; gap: 18px; position: relative; }}
      .chat-account-line {{ min-width: 0; }}
      .chat-account-menu {{ position: relative; }}
      .chat-account-menu summary {{ cursor: pointer; color: #dce4f2; font-size: 13px; font-weight: 850; list-style: none; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
      .chat-account-menu summary::after {{ content: ' >>'; color: var(--lc-accent); font-weight: 900; }}
      .chat-account-menu summary::-webkit-details-marker {{ display: none; }}
      .chat-account-menu summary:hover {{ color: #f5f6fa; }}
      .account-menu-panel {{ display: none; position: absolute; left: 0; bottom: 28px; min-width: 220px; background: #14161d; border: 1px solid #303442; border-radius: 10px; box-shadow: 0 18px 48px rgba(0,0,0,.42); padding: 6px; z-index: 12; }}
      .chat-account-menu[open] .account-menu-panel {{ display: grid; gap: 2px; }}
      .account-menu-panel a, .account-menu-panel button {{ appearance: none; border: 0; width: 100%; text-align: left; color: #c8cfdb; background: transparent; border-radius: 7px; padding: 8px 9px; margin: 0; font-size: 12px; font-weight: 650; text-decoration: none; }}
      .account-menu-panel a:hover, .account-menu-panel button:hover {{ background: #222630; color: #f5f6fa; }}
      .rail-action {{ display: flex; align-items: center; gap: 10px; width: 100%; min-height: 34px; padding: 6px 14px; border: 0; border-radius: 8px; background: transparent; color: #b5bac7; text-align: left; font-weight: 600; text-decoration: none; }}
      .rail-action:hover, .thread-row:hover, .project-header:hover {{ background: #1f2027; }}
      .rail-section-title {{ margin: 18px 14px 8px; min-height: 28px; color: #858b98; font-size: 13px; font-weight: 600; letter-spacing: 0; display: flex; align-items: center; justify-content: space-between; }}
      .section-tools, .project-tools {{ display: inline-flex; align-items: center; gap: 3px; opacity: 0; transition: opacity .12s ease; }}
      .rail-section-title:hover .section-tools, .project-header:hover .project-tools {{ opacity: 1; }}
      .icon-button {{ appearance: none; border: 0; width: 24px; height: 24px; padding: 0; margin: 0; border-radius: 7px; display: inline-grid; place-items: center; background: transparent; color: #8c93a3; font-weight: 750; cursor: pointer; line-height: 1; }}
      .icon-button:hover {{ background: #272932; color: #f5f6fa; }}
      .icon-button:disabled {{ opacity: .35; cursor: not-allowed; }}
      .icon-button:disabled:hover {{ background: transparent; color: #8c93a3; }}
      .lucide-icon {{ width: 16px; height: 16px; display: block; }}
      .project-tools .icon-button {{ font-size: 17px; }}
      .project-tools .project-menu-button {{ font-size: 18px; letter-spacing: 1px; }}
      .project-tools .project-new-thread-button {{ font-size: 20px; }}
      .chat-bubble-icon {{ width: 14px; height: 11px; border: 2px solid currentColor; border-radius: 999px; position: relative; display: inline-block; box-sizing: border-box; }}
      .chat-bubble-icon::after {{ content: ""; position: absolute; right: 0; bottom: -5px; width: 5px; height: 5px; border-left: 2px solid currentColor; border-bottom: 2px solid currentColor; transform: rotate(-18deg); border-bottom-left-radius: 4px; }}
      .project-menu-wrap {{ position: relative; display: inline-grid; place-items: center; }}
      .project-menu-dropdown {{ display: none; position: absolute; right: 0; top: 28px; min-width: 174px; background: #14161d; border: 1px solid #303442; border-radius: 10px; box-shadow: 0 18px 48px rgba(0,0,0,.42); padding: 6px; z-index: 10; }}
      .project-menu-wrap.open .project-menu-dropdown {{ display: grid; gap: 2px; }}
      .project-menu-dropdown button {{ appearance: none; border: 0; width: 100%; text-align: left; color: #c8cfdb; background: transparent; border-radius: 7px; padding: 8px 9px; margin: 0; font-size: 12px; font-weight: 650; }}
      .project-menu-dropdown button:hover {{ background: #222630; color: #f5f6fa; }}
      .project-menu-dropdown button.danger-menu-item {{ color: #ffb4a8; }}
      .chat-rail label {{ color: #aeb6c6; font-size: 13px; margin: 8px 10px; }}
      .chat-rail select, .chat-rail input {{ background: #11131a; color: #f3f6fb; border-color: #303442; border-radius: 8px; }}
      .chat-rail details {{ margin: 14px; border-top: 1px solid #22242b; padding-top: 12px; }}
      .chat-rail summary {{ cursor: pointer; color: #c8cfdb; font-weight: 750; }}
      .chat-thread-scroll {{ overflow: auto; min-height: 0; padding: 0 6px 18px; }}
      .chat-rail .project-group {{ display: block; background: transparent; border: 0; border-radius: 0; padding: 0; margin: 0 0 8px; }}
      .project-header {{ display: flex; align-items: center; justify-content: space-between; gap: 8px; min-height: 34px; padding: 4px 8px 4px 14px; border-radius: 8px; color: #969ca9; cursor: pointer; }}
      .project-group.active .project-header {{ color: #f5f6fa; }}
      .project-name {{ overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
      .project-thread-list {{ padding-left: 30px; padding-right: 4px; }}
      .project-empty {{ margin: 7px 8px 12px; color: #575d6a; }}
      .thread-row {{ width: 100%; margin: 1px 0; text-align: left; background: transparent; border: 0; color: #b9becb; padding: 7px 8px; border-radius: 8px; font-weight: 500; display: block; overflow-wrap: anywhere; }}
      .thread-row.active {{ background: #24262f; color: #fff; }}
      .chat-main {{ min-width: 0; min-height: 0; height: 100%; overflow: hidden; display: grid; grid-template-rows: 48px minmax(0, 1fr) auto; background: #050507; position: relative; }}
      .chat-topbar {{ display: flex; justify-content: space-between; align-items: center; border-bottom: 1px solid #22242b; padding: 0 16px; }}
      .chat-title {{ min-width: 0; font-size: 14px; font-weight: 750; color: #f4f6fb; display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
      .workbench-status {{ display: inline-flex; align-items: center; gap: 8px; color: #8f96a3; font-size: 13px; }}
      .workbench-dot {{ width: 8px; height: 8px; border-radius: 999px; background: #d89b36; }}
      .workbench-dot.connected {{ background: #2ec27e; }}
      workbench-transcript {{ display: block; height: 100%; overflow: hidden; min-height: 0; }}
      .chat-composer-wrap {{ padding: 18px max(22px, 5vw) 76px; display: grid; justify-items: center; }}
      .chat-composer {{ width: min(var(--chat-content-width), 100%); border: 1px solid #2a2d37; border-radius: 18px; background: #0d0f14; padding: 10px; box-shadow: 0 18px 60px rgba(0,0,0,.35); }}
      .chat-composer textarea {{ background: transparent; color: #f5f6fa; border: 0; min-height: 72px; resize: vertical; padding: 10px 6px; }}
      .chat-composer textarea:focus {{ outline: none; }}
      .chat-composer-actions {{ display: flex; justify-content: flex-end; gap: 8px; align-items: center; }}
      .chat-composer button {{ border-radius: 10px; margin-top: 0; }}
      .chat-composer .send-button {{ background: var(--lc-accent); color: var(--lc-accent-text); display: inline-flex; align-items: center; gap: 7px; }}
      .composer-hidden {{ display: none; }}
      .chat-title-wrap {{ min-width: 0; display: inline-flex; align-items: center; gap: 8px; max-width: min(70vw, 760px); padding: 4px 8px; margin-left: -8px; border-radius: 8px; }}
      .chat-title-wrap:hover {{ background: #151820; }}
      .thread-title-edit-button {{ opacity: 0; transition: opacity .12s ease; }}
      .chat-title-wrap:hover .thread-title-edit-button, .chat-title-wrap:focus-within .thread-title-edit-button {{ opacity: 1; }}
      .thread-title-input {{ width: min(520px, 60vw); background: #11131a; color: #f5f6fa; border: 1px solid #333746; border-radius: 8px; padding: 7px 9px; font: inherit; font-weight: 750; }}
      .chat-brand-corner {{ min-width: min(250px, 100%); display: grid; gap: 2px; color: #858b98; font-size: 12px; font-weight: 750; line-height: 1.15; pointer-events: none; }}
      .chat-brand-corner strong {{ display: block; color: #f4f6fb; font-size: 15px; font-weight: 850; text-align: left; }}
      .chat-brand-meta {{ display: grid; grid-template-columns: minmax(0, 1fr) auto; align-items: baseline; column-gap: 18px; }}
      .chat-brand-motto {{ color: var(--lc-accent); font-weight: 800; text-align: left; }}
      .chat-brand-version {{ color: #858b98; text-align: right; }}
      .thread-context-menu {{ display: none; position: fixed; min-width: 160px; background: #14161d; border: 1px solid #303442; border-radius: 10px; box-shadow: 0 18px 48px rgba(0,0,0,.42); padding: 6px; z-index: 40; }}
      .thread-context-menu.open {{ display: grid; gap: 2px; }}
      .thread-context-menu button {{ appearance: none; border: 0; width: 100%; text-align: left; color: #c8cfdb; background: transparent; border-radius: 7px; padding: 8px 9px; margin: 0; font-size: 12px; font-weight: 650; }}
      .thread-context-menu button:hover {{ background: #222630; color: #f5f6fa; }}
      .thread-context-menu button.danger-menu-item {{ color: #ffb4a8; }}
      .chat-modal {{ border: 1px solid #333746; border-radius: 12px; background: #101219; color: #f5f6fa; padding: 18px; max-width: 420px; }}
      .output-modal {{ width: min(760px, calc(100vw - 48px)); max-width: 760px; }}
      .knowledge-transfer-modal {{ width: min(820px, calc(100vw - 48px)); max-width: 820px; }}
      .chat-modal::backdrop {{ background: rgba(0,0,0,.62); }}
      .output-list {{ display: grid; gap: 10px; margin: 14px 0; max-height: 60vh; overflow: auto; }}
      .output-row {{ border: 1px solid #2a2d37; border-radius: 10px; background: #0d0f14; padding: 12px; }}
      .output-row strong {{ display: block; color: #f5f6fa; margin-bottom: 4px; }}
      .output-row code {{ color: #aab2c3; overflow-wrap: anywhere; }}
      .knowledge-transfer-grid {{ display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 14px; margin: 14px 0; }}
      .knowledge-transfer-grid label {{ margin: 0; }}
      .knowledge-transfer-grid select, .knowledge-transfer-grid input {{ width: 100%; }}
      .knowledge-transfer-actions {{ display: flex; gap: 8px; flex-wrap: wrap; margin-top: 12px; }}
      @media (max-width: 480px) {{ body {{ overflow: auto; }} main {{ height: auto; }} .chat-shell-fullscreen {{ --chat-content-width: 100%; position: static; height: auto; min-height: 100vh; overflow: visible; grid-template-columns: 1fr; }} .chat-rail {{ min-height: 44vh; border-right: 0; border-bottom: 1px solid #22242b; }} .chat-topbar {{ padding: 0 10px; }} .chat-title-wrap {{ max-width: 58vw; }} .workbench-status {{ font-size: 12px; }} .chat-composer-wrap {{ padding: 12px 14px 70px; }} .chat-composer {{ border-radius: 14px; }} .thread-title-input {{ width: 58vw; }} }}
      @media (min-width: 481px) and (max-width: 820px) {{ body {{ overflow: auto; }} main {{ height: auto; }} .chat-shell-fullscreen {{ --chat-content-width: 100%; position: static; height: auto; min-height: 100vh; overflow: visible; grid-template-columns: 1fr; }} .chat-rail {{ min-height: 38vh; border-right: 0; border-bottom: 1px solid #22242b; }} .chat-composer-wrap {{ padding: 16px 20px 74px; }} .chat-title-wrap {{ max-width: 64vw; }} }}
      @media (min-width: 821px) and (max-width: 1439px) {{ .chat-shell-fullscreen {{ --chat-content-width: 980px; grid-template-columns: 320px minmax(0, 1fr); }} }}
      @media (min-width: 1440px) {{ .chat-shell-fullscreen {{ --chat-content-width: 1120px; grid-template-columns: 360px minmax(0, 1fr); }} .chat-composer-wrap {{ padding-inline: max(28px, 6vw); }} }}
    </style>
    <div class="chat-shell-fullscreen">
      <aside class="chat-rail">
        <button class="icon-button rail-collapse-button" title="Collapse sidebar" onclick="toggleChatRail()"><svg class="lucide-icon" data-icon="panel-left-close" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect width="18" height="18" x="3" y="3" rx="2"/><path d="M9 3v18"/><path d="m16 15-3-3 3-3"/></svg></button>
        <button class="rail-action" onclick="openWorkbenchOutputs()">My outputs</button>
        <button class="rail-action" onclick="openKnowledgeTransferModal()">Use thread knowledge</button>
        <div class="rail-section-title">
          <span>Projects</span>
          <span class="section-tools">
            <button class="icon-button projects-new-button" title="New project" onclick="openNewProjectModal()"><svg class="lucide-icon" data-icon="plus" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 12h14"/><path d="M12 5v14"/></svg></button>
            <button class="icon-button projects-menu-button" title="Projects menu" onclick="openProjectsUtilityModal()"><svg class="lucide-icon" data-icon="more-horizontal" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/></svg></button>
          </span>
        </div>
        <div id="workbench-project-list" class="chat-thread-scroll"></div>
        <div class="chat-rail-footer">
          <div class="chat-account-line">
            <details class="chat-account-menu">
              <summary id="workbench-user">Loading signed-in user...</summary>
              <div class="account-menu-panel">
                <a href="/app/terminal">Terminal instructions</a>
                <button onclick="openResponsePreferencesModal()">Response preferences</button>
                {admin_menu_item}
              </div>
            </details>
          </div>
          <div class="chat-brand-corner"><strong>Local Codex for Enterprise</strong><div class="chat-brand-meta"><span class="chat-brand-motto">Made with Codex</span><span class="chat-brand-version">v__VERSION__</span></div></div>
        </div>
      </aside>
      <div class="chat-main">
        <div class="chat-topbar">
          <div id="workbench-thread-title-wrap" class="chat-title-wrap"><span id="workbench-thread-title" class="chat-title">Local Codex Chat</span><button class="icon-button thread-title-edit-button" title="Rename thread" onclick="beginWorkbenchTitleEdit()"><svg class="lucide-icon" data-icon="pencil" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21.2 6.8a2.8 2.8 0 0 0-4-4L4 16v4h4Z"/><path d="m14 5 5 5"/></svg></button></div>
          <span class="workbench-status"><span id="workbench-status-dot" class="workbench-dot"></span><span id="workbench-status-label">not connected</span></span>
        </div>
        <workbench-transcript id="workbench-chat" aria-label="Chat conversation"></workbench-transcript>
        <div id="chat-composer-wrap" class="chat-composer-wrap composer-hidden">
          <div class="chat-composer">
            <textarea id="composer-input" class="composer-input" placeholder="Ask Codex to inspect, edit, test, or explain this project." onkeydown="composerKeydown(event)"></textarea>
            <div class="chat-composer-actions">
              <button class="send-button" onclick="sendWorkbenchMessage()"><svg class="lucide-icon" data-icon="send" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m22 2-7 20-4-9-9-4Z"/><path d="M22 2 11 13"/></svg>Send</button>
            </div>
          </div>
        </div>
      </div>
    </div>
    <div id="thread-context-menu" class="thread-context-menu">
      <button onclick="renameThreadFromContextMenu()">Rename thread</button>
      <button class="danger-menu-item" onclick="removeThreadFromContextMenu()">Remove thread</button>
    </div>
    <dialog id="thread-rename-modal" class="chat-modal">
      <strong>Rename thread</strong>
      <label>Thread name<input id="thread-rename-input" placeholder="Thread title"></label>
      <div class="actions">
        <button onclick="saveThreadRenameModal()">Save</button>
        <button class="secondary" onclick="document.getElementById('thread-rename-modal').close()">Close</button>
      </div>
    </dialog>
    <dialog id="workbench-project-modal" class="chat-modal">
      <strong>New project</strong>
      <p class="hint">Create an empty project in your assigned user workspace. Repositories are optional and can be added later from the project menu.</p>
      <label>Project name<input id="workbench-project-name" placeholder="Client Portal"></label>
      <div class="actions">
        <button onclick="createWorkbenchProject()">Create project</button>
        <button class="secondary" onclick="document.getElementById('workbench-project-modal').close()">Close</button>
      </div>
    </dialog>
    <dialog id="workbench-repository-modal" class="chat-modal">
      <strong>Project menu</strong>
      <p class="hint">Add an HTTPS repository to this project. The server keeps the destination under the selected project.</p>
      <input id="repository-selected-project" type="hidden">
      <label>Repository URL<input id="workbench-clone-url" placeholder="https://github.com/org/project.git"></label>
      <label>Destination folder<input id="workbench-clone-destination" placeholder="project"></label>
      <div class="actions">
        <button onclick="addRepositoryToSelectedProject()">Add repository</button>
        <button class="secondary" onclick="document.getElementById('workbench-repository-modal').close()">Close</button>
      </div>
    </dialog>
    <dialog id="workbench-output-modal" class="chat-modal output-modal">
      <strong>My outputs</strong>
      <p class="hint">Read-only assets assigned to your user. End outputs are stakeholder-facing deliverables; operational outputs are work records.</p>
      <div id="workbench-output-list" class="output-list"><p class="hint">Loading outputs...</p></div>
      <button onclick="document.getElementById('workbench-output-modal').close()">Close</button>
    </dialog>
    <dialog id="knowledge-transfer-modal" class="chat-modal knowledge-transfer-modal" data-reference-update-path="/v1/thread-references/">
      <strong>Use thread knowledge</strong>
      <p class="hint">Move bounded knowledge into the selected thread. References record provenance only; they do not execute work outside the current session/worker.</p>
      <div class="knowledge-transfer-grid">
        <label>Source thread<select id="knowledge-source-thread"></select></label>
        <label>Existing output<select id="knowledge-source-output"></select></label>
        <label>Artifact title<input id="knowledge-artifact-title" placeholder="Thread handoff"></label>
        <label>Max characters<input id="knowledge-max-chars" type="number" min="64" max="20000" value="12000"></label>
      </div>
      <div class="knowledge-transfer-actions">
        <button onclick="summarizeKnowledgeThread()">Summarize into this thread</button>
        <button class="secondary" onclick="exportKnowledgeThread()">Export transcript</button>
        <button class="secondary" onclick="createKnowledgeHandoff()">Create handoff</button>
        <button class="secondary" onclick="importKnowledgeOutput()">Import output</button>
        <button class="secondary" onclick="document.getElementById('knowledge-transfer-modal').close()">Close</button>
      </div>
      <p id="knowledge-transfer-status" class="hint"></p>
    </dialog>
    <dialog id="response-feedback-modal" class="chat-modal">
      <strong>Bad response</strong>
      <p class="hint">Choose why this response missed. These tags update user-scoped response preferences; comments and message text are not injected into future prompts.</p>
      <label><input type="checkbox" value="too_verbose"> Too verbose</label>
      <label><input type="checkbox" value="too_generic"> Too generic</label>
      <label><input type="checkbox" value="wrong_context"> Wrong context</label>
      <label><input type="checkbox" value="used_repo_when_not_needed"> Used repository when not needed</label>
      <label><input type="checkbox" value="poor_formatting"> Poor formatting</label>
      <label><input type="checkbox" value="missed_business_goal"> Missed business goal</label>
      <label><input type="checkbox" value="raw_tool_output"> Raw tool output</label>
      <label><input type="checkbox" value="other"> Other</label>
      <div class="actions">
        <button onclick="submitResponseFeedbackFromModal()">Save feedback</button>
        <button class="secondary" onclick="document.getElementById('response-feedback-modal').close()">Close</button>
      </div>
    </dialog>
    <dialog id="response-preferences-modal" class="chat-modal">
      <strong>User response preferences</strong>
      <p class="hint">Inspectable per-user harness guidance. These preferences affect style and routing behavior only, not factual truth.</p>
      <pre id="response-preferences-body">Loading...</pre>
      <div class="actions">
        <button class="secondary" onclick="resetResponsePreferences()">Reset preferences</button>
        <button onclick="document.getElementById('response-preferences-modal').close()">Close</button>
      </div>
    </dialog>
    <dialog id="workbench-utility-modal" class="chat-modal">
      <strong>Projects utility menu</strong>
      <p class="hint">Project utilities will appear here as this surface grows.</p>
      <button class="secondary" onclick="document.getElementById('workbench-utility-modal').close()">Close</button>
    </dialog>
    <input id="chat-default-workspace-root" type="hidden" value="{default_workspace_root}">
    <input id="chat-turn-guidance" type="hidden" value="{turn_guidance_json}">"#
    )
}

fn terminal_page(config: &EnterpriseConfig) -> String {
    let default_workspace_root =
        html_escape(config.default_workspace_root.as_deref().unwrap_or(""));
    format!(
        r##"
    <style>#result-panel {{ display: none; }}</style>
    <div class="terminal-instructions">
      <section>
        <h2>Terminal Login Instructions</h2>
        <p class="hint">Use this page when you want to work from a local shell. The browser work surface lives at /chat.</p>
        <div class="toolbar"><a href="/chat">Use Browser Workbench</a></div>
        <pre>curl -s http://127.0.0.1:8787/v1/auth/login \
  -H 'content-type: application/json' \
  -d '{{"email":"developer@example.com","password":"replace-me"}}'

export LOCAL_CODEX_ENTERPRISE_TOKEN='paste-api-token-here'

curl -s http://127.0.0.1:8787/v1/threads \
  -H "authorization: Bearer $LOCAL_CODEX_ENTERPRISE_TOKEN" \
  -H 'content-type: application/json' \
  -d '{{"workspace_path":"{default_workspace_root}","title":"Terminal session"}}'</pre>
      </section>
      <section>
        <h2>CLI Workflow</h2>
        <p class="hint">Terminal usage is API-token based. Workspace access is still enforced by the server; use an assigned workspace path, not arbitrary server paths.</p>
        <pre>curl -s http://127.0.0.1:8787/v1/user-workspaces \
  -H "authorization: Bearer $LOCAL_CODEX_ENTERPRISE_TOKEN"

curl -s http://127.0.0.1:8787/v1/workers \
  -H "authorization: Bearer $LOCAL_CODEX_ENTERPRISE_TOKEN" \
  -H 'content-type: application/json' \
  -d '{{"workspace_path":"{default_workspace_root}","session_id":"session-id-from-create"}}'</pre>
      </section>
    </div>"##
    )
}

fn outputs_page() -> &'static str {
    r#"
    <section>
      <h2>My Outputs</h2>
      <p class="hint">Output metadata is scoped to the signed-in user. The server stores category, type, title, status, and a relative artifact location only; it does not store prompt text, raw model output, secrets, or cross-user deliverables here.</p>
      <div class="grid">
        <section>
          <h2>Operational outputs</h2>
          <p class="hint">Roadmap, bugs, errors, workstatus, and other delivery-health records for the user's working loop.</p>
        </section>
        <section>
          <h2>End outputs</h2>
          <p class="hint">Executive or stakeholder-facing deliverable reports, code-change summaries, exported patches, and final evidence packages.</p>
        </section>
      </div>
      <div id="output-index"></div>
    </section>
    "#
}

fn outputs_admin_page() -> &'static str {
    r#"
    <section>
      <h2>Assign User Report</h2>
      <p class="hint">Managers assign read-only output metadata to a specific user. The file itself must live under that user's output artifact folder on the server.</p>
      <div id="user-empty-state" hidden></div>
      <label>User<select id="output-user-select"></select></label>
      <label>Category<select id="admin-output-category"><option>deliverable</option><option>operational</option></select></label>
      <label>Type<input id="admin-output-type" value="end_report"></label>
      <label>Title<input id="admin-output-title"></label>
      <label>Relative artifact location<input id="admin-output-path" value="outputs/end-report.md"></label>
      <label>Status<select id="admin-output-status"><option>completed</option><option>active</option><option>draft</option><option>archived</option></select></label>
      <button id="output-create-submit" onclick="postJson('/v1/outputs',{owner_user_id:v('output-user-select'),category:v('admin-output-category'),output_type:v('admin-output-type'),title:v('admin-output-title'),artifact_path:v('admin-output-path'),status:v('admin-output-status'),metadata_json:{source:'manager'}})">Assign Output</button>
    </section>"#
}

fn session_page() -> &'static str {
    r#"
    <div class="grid">
      <section>
        <h2>Session Lookup</h2>
        <label>Session ID<input id="lookup-session-id"></label>
        <button onclick="getJson('/v1/threads/'+encodeURIComponent(v('lookup-session-id')))">Load Thread</button>
      </section>
      <section>
        <h2>Workers</h2>
        <button onclick="getJson('/v1/workers')">Refresh Workers</button>
      </section>
    </div>"#
}

fn audit_page() -> &'static str {
    r#"
    <section>
      <h2>Trace Search</h2>
      <label>Trace ID<input id="audit-trace-id"></label>
      <button onclick="getJson('/v1/evidence-records?trace_id='+encodeURIComponent(v('audit-trace-id')))">Search Audit</button>
      <button class="secondary" onclick="getJson('/v1/evidence-records')">Recent Audit</button>
    </section>"#
}

fn rbac_permission_matrix_html() -> String {
    let roles = [
        EnterpriseRole::Admin,
        EnterpriseRole::Manager,
        EnterpriseRole::Developer,
        EnterpriseRole::Viewer,
    ];
    let actions = [
        EnterpriseAction::AdministerUsers,
        EnterpriseAction::AssignRoles,
        EnterpriseAction::ManageWorkspaces,
        EnterpriseAction::GrantWorkspaceAccess,
        EnterpriseAction::ManageContextPacks,
        EnterpriseAction::ManageOutputs,
        EnterpriseAction::ManageOwnContextPacks,
        EnterpriseAction::StartWorker,
        EnterpriseAction::ReadThreads,
        EnterpriseAction::ReadAudit,
    ];
    let mut html = String::from(r#"<table class="resource-table"><thead><tr><th>Role</th>"#);
    for action in actions {
        html.push_str(&format!("<th>{}</th>", action_label(action)));
    }
    html.push_str("</tr></thead><tbody>");
    for role in roles {
        html.push_str(&format!("<tr><td>{}</td>", role.as_str()));
        for action in actions {
            let value = if rbac::role_allows(role, action) {
                "Allowed"
            } else {
                "Denied"
            };
            html.push_str(&format!("<td>{value}</td>"));
        }
        html.push_str("</tr>");
    }
    html.push_str("</tbody></table>");
    html
}

fn action_label(action: EnterpriseAction) -> &'static str {
    match action {
        EnterpriseAction::AdministerUsers => "administer_users",
        EnterpriseAction::AssignRoles => "assign_roles",
        EnterpriseAction::ManageWorkspaces => "manage_workspaces",
        EnterpriseAction::GrantWorkspaceAccess => "grant_workspace_access",
        EnterpriseAction::ManageContextPacks => "manage_context_packs",
        EnterpriseAction::ManageOutputs => "manage_outputs",
        EnterpriseAction::ManageOwnContextPacks => "manage_own_context_packs",
        EnterpriseAction::StartWorker => "start_worker",
        EnterpriseAction::ReadThreads => "read_threads",
        EnterpriseAction::ReadAudit => "read_audit",
    }
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

struct BoundedContent {
    content: String,
    truncated: bool,
}

async fn create_thread_knowledge_output<S>(
    state: AppState<S>,
    trace: TraceContext,
    headers: HeaderMap,
    source_thread_id: String,
    request: CreateThreadArtifactRequest,
    reference_type: &str,
    output_type: &str,
) -> Result<(StatusCode, Json<ThreadReferenceOutputResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &trace, &principal, EnterpriseAction::ReadThreads).await?;
    let source_session = state
        .store
        .get_session(&principal, &source_thread_id)
        .await
        .map_err(ApiError::storage)?;
    let target_thread_id = request
        .target_thread_id
        .clone()
        .unwrap_or_else(|| source_session.session_id.clone());
    let target_session = state
        .store
        .get_session(&principal, &target_thread_id)
        .await
        .map_err(ApiError::storage)?;
    let messages = state
        .store
        .list_session_messages(&principal, &source_session.session_id)
        .await
        .map_err(ApiError::storage)?;
    let title = request
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| default_thread_knowledge_title(reference_type, &source_session));
    let bounded = if reference_type == "handoff" {
        bounded_thread_handoff_markdown(&source_session, &messages, request.max_chars)
    } else {
        bounded_thread_markdown(&source_session, &messages, request.max_chars)
    };
    let artifact_path =
        generated_thread_knowledge_artifact_path(&source_session.session_id, &title);
    let output = state
        .store
        .create_output(CreateOutputInput {
            owner_user_id: principal.user_id.clone(),
            workspace_id: Some(target_session.workspace_id.clone()),
            session_id: Some(source_session.session_id.clone()),
            worker_id: source_session.last_worker_id.clone(),
            category: OutputCategory::Deliverable,
            output_type: output_type.to_string(),
            title,
            artifact_path,
            status: "completed".to_string(),
            metadata_json: serde_json::json!({
                "source": "thread_reference",
                "source_thread_id": source_session.session_id,
                "target_thread_id": target_session.session_id,
                "reference_type": reference_type,
                "knowledge_origin": "user_generated",
                "truncated": bounded.truncated,
                "message_count": messages.len(),
            }),
        })
        .await
        .map_err(ApiError::storage)?;
    let output_path = user_output_artifact_path(&state.config, &principal.user_id, &output)
        .map_err(ApiError::storage)?;
    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    }
    tokio::fs::write(&output_path, bounded.content.as_bytes())
        .await
        .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let reference = state
        .store
        .create_thread_reference(
            &principal,
            CreateThreadReferenceInput {
                source_thread_id: source_session.session_id.clone(),
                target_thread_id: target_session.session_id.clone(),
                source_output_id: None,
                output_id: Some(output.output_id.clone()),
                reference_type: reference_type.to_string(),
                knowledge_origin: "user_generated".to_string(),
                status: "completed".to_string(),
                metadata_json: serde_json::json!({
                    "source_thread_id": source_session.session_id,
                    "target_thread_id": target_session.session_id,
                    "output_id": output.output_id,
                    "reference_type": reference_type,
                    "knowledge_origin": "user_generated",
                    "truncated": bounded.truncated,
                    "message_count": messages.len(),
                }),
            },
        )
        .await
        .map_err(ApiError::storage)?;
    record_audit(
        &state,
        EvidenceRecordContext::new(&trace)
            .actor(principal.user_id.clone())
            .workspace(target_session.workspace_id.clone())
            .session(target_session.session_id.clone()),
        "thread_reference.create",
        TraceResult::Completed,
        serde_json::json!({
            "reference_id": reference.reference_id.clone(),
            "source_thread_id": reference.source_thread_id.clone(),
            "target_thread_id": reference.target_thread_id.clone(),
            "output_id": reference.output_id.clone(),
            "reference_type": reference.reference_type.clone(),
            "knowledge_origin": reference.knowledge_origin.clone(),
            "status": reference.status.clone(),
            "truncated": bounded.truncated,
        }),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(ThreadReferenceOutputResponse { reference, output }),
    ))
}

fn default_thread_knowledge_title(reference_type: &str, session: &SessionRecord) -> String {
    let title = session.title.as_deref().unwrap_or("Thread");
    if reference_type == "handoff" {
        format!("{title} Handoff")
    } else {
        format!("{title} Transcript Export")
    }
}

fn bounded_thread_markdown(
    session: &SessionRecord,
    messages: &[SessionMessageRecord],
    max_chars: Option<usize>,
) -> BoundedContent {
    let title = session.title.as_deref().unwrap_or("Untitled thread");
    let mut content = format!(
        "# Thread Export: {title}\n\nSource thread: `{}`\n\n## Transcript\n\n",
        session.session_id
    );
    for message in messages {
        content.push_str(&format!(
            "### {}\n\n{}\n\n",
            message.label,
            sanitize_artifact_markdown_text(&message.text)
        ));
    }
    bounded_text(content, max_chars)
}

fn bounded_thread_handoff_markdown(
    session: &SessionRecord,
    messages: &[SessionMessageRecord],
    max_chars: Option<usize>,
) -> BoundedContent {
    let title = session.title.as_deref().unwrap_or("Untitled thread");
    let source = bounded_thread_markdown(session, messages, max_chars).content;
    let content = format!(
        "# Thread Handoff: {title}\n\nSource thread: `{}`\n\n## Decisions\n\n{}\n\n## Completed Work\n\nSee source transcript excerpt below.\n\n## Open Questions\n\n{}\n\n## Next Actions\n\nReview this handoff in the target thread and decide the next session task.\n\n## Source Excerpt\n\n{}",
        session.session_id,
        extract_lines_for_handoff(messages, "decision"),
        extract_lines_for_handoff(messages, "question"),
        source
    );
    bounded_text(content, max_chars)
}

fn extract_lines_for_handoff(messages: &[SessionMessageRecord], needle: &str) -> String {
    let needle = needle.to_ascii_lowercase();
    let lines = messages
        .iter()
        .flat_map(|message| message.text.lines())
        .filter(|line| line.to_ascii_lowercase().contains(&needle))
        .map(|line| format!("- {}", sanitize_artifact_markdown_text(line)))
        .collect::<Vec<_>>();
    if lines.is_empty() {
        "- No explicit entries found in the bounded source thread.".to_string()
    } else {
        lines.join("\n")
    }
}

fn bounded_text(content: String, max_chars: Option<usize>) -> BoundedContent {
    let limit = max_chars.unwrap_or(12_000).clamp(64, 20_000);
    let mut truncated = false;
    let mut bounded = String::new();
    for (index, character) in content.chars().enumerate() {
        if index >= limit {
            truncated = true;
            break;
        }
        bounded.push(character);
    }
    if truncated {
        bounded.push_str("\n\n[Truncated by Local Codex for Enterprise.]\n");
    }
    BoundedContent {
        content: bounded,
        truncated,
    }
}

fn generated_thread_knowledge_artifact_path(thread_id: &str, title: &str) -> String {
    let slug = slugify_output_title(title);
    format!("thread-knowledge/{thread_id}/{}-{slug}.md", Uuid::new_v4())
}

fn sanitize_artifact_markdown_text(value: &str) -> String {
    value.replace('\0', "")
}

fn parse_role(value: &str) -> Result<EnterpriseRole, ApiError> {
    if value == "owner" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "owner is not an assignable enterprise role; use admin",
        ));
    }
    EnterpriseRole::from_storage(value)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "unknown enterprise role"))
}

fn parse_output_category(value: &str) -> Result<OutputCategory, ApiError> {
    OutputCategory::from_storage(value)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "unknown output category"))
}

fn generated_chat_output_artifact_path(thread_id: &str, title: &str) -> String {
    let slug = slugify_output_title(title);
    format!("chat/{thread_id}/{}-{slug}.md", Uuid::new_v4())
}

fn slugify_output_title(title: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;
    for character in title.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            last_was_dash = false;
        } else if !last_was_dash && !slug.is_empty() {
            slug.push('-');
            last_was_dash = true;
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "output".to_string()
    } else {
        slug.chars().take(64).collect()
    }
}

async fn user_workspace_roots_from_request(
    config: &EnterpriseConfig,
    request: &CreateUserRequest,
) -> anyhow::Result<Vec<String>> {
    if let Some(roots) = &request.workspace_roots
        && !roots.is_empty()
    {
        return Ok(roots.clone());
    }
    Ok(
        default_user_workspace_root_for_email(config, &request.email)
            .await?
            .into_iter()
            .collect(),
    )
}

async fn default_user_workspace_root_for_email(
    config: &EnterpriseConfig,
    email: &str,
) -> anyhow::Result<Option<String>> {
    let Some(default_root) = &config.default_workspace_root else {
        return Ok(None);
    };
    Ok(Some(
        default_user_workspace_root_at_base(default_root, email).await?,
    ))
}

async fn default_user_workspace_root_at_base(
    default_root: &str,
    email: &str,
) -> anyhow::Result<String> {
    let email_namespace = email
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '@' | '.' | '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches(|ch| matches!(ch, '-' | '.' | '@' | '_'))
        .to_string();
    let email_namespace = if email_namespace.is_empty() {
        "user".to_string()
    } else {
        email_namespace
    };
    let path = std::path::PathBuf::from(default_root)
        .join("user")
        .join(email_namespace);
    tokio::fs::create_dir_all(&path)
        .await
        .with_context(|| format!("create default user workspace {}", path.display()))?;
    Ok(path.to_string_lossy().to_string())
}

fn user_output_artifact_path(
    config: &EnterpriseConfig,
    user_id: &str,
    output: &OutputRecord,
) -> anyhow::Result<std::path::PathBuf> {
    let root = std::path::PathBuf::from(&config.output_artifact_root)
        .canonicalize()
        .with_context(|| {
            format!(
                "output artifact root is not accessible from this Enterprise server runtime: {}",
                config.output_artifact_root
            )
        })?;
    let user_root = root.join(user_id);
    let requested = user_root.join(&output.artifact_path);
    let parent = requested
        .parent()
        .ok_or_else(|| anyhow::anyhow!("output artifact path has no parent"))?;
    if !requested.starts_with(&user_root) {
        anyhow::bail!("output artifact path escapes user output root");
    }
    std::fs::create_dir_all(parent)
        .with_context(|| format!("create output artifact parent {}", parent.display()))?;
    let canonical_user_root = user_root
        .canonicalize()
        .context("canonicalize output artifact user root")?;
    let canonical_parent = parent
        .canonicalize()
        .context("canonicalize output artifact parent")?;
    if !canonical_parent.starts_with(&canonical_user_root) {
        anyhow::bail!("output artifact path escapes user output root");
    }
    Ok(requested)
}

fn validate_output_status(value: &str) -> Result<(), ApiError> {
    match value {
        "draft" | "active" | "completed" | "archived" => Ok(()),
        _ => Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "unknown output status",
        )),
    }
}

fn sanitized_output_metadata(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    if metadata_key_is_sensitive(&key) {
                        (key, serde_json::Value::String("[redacted]".to_string()))
                    } else {
                        (key, sanitized_output_metadata(value))
                    }
                })
                .collect(),
        ),
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(sanitized_output_metadata).collect())
        }
        other => other,
    }
}

fn metadata_key_is_sensitive(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "password",
        "token",
        "secret",
        "authorization",
        "auth",
        "prompt",
        "model_output",
        "repo_url",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

fn nullable_selection(values: Vec<String>) -> Vec<Option<String>> {
    let selected = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(Some)
        .collect::<Vec<_>>();
    if selected.is_empty() {
        vec![None]
    } else {
        selected
    }
}

async fn demo_data_status<S>(
    state: &AppState<S>,
    principal: &crate::storage::AuthPrincipal,
) -> Result<DemoDataStatusResponse, ApiError>
where
    S: EnterpriseStore,
{
    let users = state
        .store
        .list_users(principal)
        .await
        .map_err(ApiError::internal)?;
    let packs = state
        .store
        .list_context_packs(principal)
        .await
        .map_err(ApiError::internal)?;
    let assignments = state
        .store
        .list_context_pack_assignments(principal)
        .await
        .map_err(ApiError::internal)?;
    let demo_pack_ids = packs
        .iter()
        .filter(|pack| pack.name == DEMO_CONTEXT_PACK_NAME)
        .map(|pack| pack.pack_id.clone())
        .collect::<Vec<_>>();
    let demo_user_count = users
        .iter()
        .filter(|user| DEMO_USER_EMAILS.contains(&user.email.as_str()))
        .count();
    let demo_context_pack_count = demo_pack_ids.len();
    let demo_assignment_count = assignments
        .iter()
        .filter(|assignment| demo_pack_ids.contains(&assignment.pack_id))
        .count();
    Ok(DemoDataStatusResponse {
        installed: demo_user_count > 0 || demo_context_pack_count > 0 || demo_assignment_count > 0,
        demo_user_count,
        demo_context_pack_count,
        demo_assignment_count,
    })
}

async fn ensure_demo_user<S>(
    state: &AppState<S>,
    principal: &crate::storage::AuthPrincipal,
    email: &str,
    role: EnterpriseRole,
) -> Result<DemoUserSeedRecord, ApiError>
where
    S: EnterpriseStore,
{
    let users = state
        .store
        .list_users(principal)
        .await
        .map_err(ApiError::internal)?;
    if let Some(user) = users.into_iter().find(|user| user.email == email) {
        return Ok(DemoUserSeedRecord {
            user_id: user.user_id,
            email: user.email,
            role: user.role,
            created: false,
            password: None,
        });
    }

    let password = format!("demo-{}", Uuid::new_v4().simple());
    let user = state
        .store
        .create_user(
            principal,
            CreateUserInput {
                email: email.to_string(),
                password_hash: auth::hash_password(&password).map_err(ApiError::internal)?,
                role,
                workspace_roots: Vec::new(),
            },
        )
        .await
        .map_err(ApiError::storage)?;
    Ok(DemoUserSeedRecord {
        user_id: user.user_id,
        email: user.email,
        role: user.role,
        created: true,
        password: Some(password),
    })
}

async fn ensure_demo_context_pack<S>(
    state: &AppState<S>,
    principal: &crate::storage::AuthPrincipal,
) -> Result<(ContextPackRecord, Vec<ContextDocumentRecord>), ApiError>
where
    S: EnterpriseStore,
{
    let packs = state
        .store
        .list_context_packs(principal)
        .await
        .map_err(ApiError::internal)?;
    if let Some(pack) = packs
        .into_iter()
        .find(|pack| pack.name == DEMO_CONTEXT_PACK_NAME)
    {
        return Ok((pack, Vec::new()));
    }
    state
        .store
        .create_context_pack(
            principal,
            CreateContextPackInput {
                name: DEMO_CONTEXT_PACK_NAME.to_string(),
                documents: demo_context_pack_documents(),
            },
        )
        .await
        .map_err(ApiError::storage)
}

async fn ensure_demo_assignments<S>(
    state: &AppState<S>,
    principal: &crate::storage::AuthPrincipal,
    pack_id: &str,
    users: &[DemoUserSeedRecord],
    workspace_id: &str,
) -> Result<Vec<ContextPackAssignmentRecord>, ApiError>
where
    S: EnterpriseStore,
{
    let mut existing = state
        .store
        .list_context_pack_assignments(principal)
        .await
        .map_err(ApiError::internal)?;
    let mut results = Vec::new();
    for user in users.iter().filter(|user| user.role == "developer") {
        if let Some(assignment) = existing.iter().find(|assignment| {
            assignment.pack_id == pack_id
                && assignment.user_id.as_deref() == Some(user.user_id.as_str())
                && assignment.workspace_id.as_deref() == Some(workspace_id)
        }) {
            results.push(assignment.clone());
            continue;
        }

        let mut order = 1000;
        while existing.iter().any(|assignment| {
            assignment.user_id.as_deref() == Some(user.user_id.as_str())
                && assignment.workspace_id.as_deref() == Some(workspace_id)
                && assignment.assignment_order == order
        }) {
            order += 10;
        }
        let assignment = state
            .store
            .assign_context_pack(
                principal,
                AssignContextPackInput {
                    pack_id: pack_id.to_string(),
                    user_id: Some(user.user_id.clone()),
                    workspace_id: Some(workspace_id.to_string()),
                    assignment_order: order,
                    required_session: true,
                    required_worker: true,
                },
            )
            .await
            .map_err(ApiError::storage)?;
        existing.push(assignment.clone());
        results.push(assignment);
    }
    Ok(results)
}

fn demo_context_pack_documents() -> Vec<crate::context_packs::ContextPackDocumentInput> {
    vec![
        document(
            "PACK.md",
            "name: Demo Engineering Context Pack\nversion: 1\nrequired_documents:\n- CALIBRATION.md\n- OPERATING-INSTRUCTIONS.md\n- PROJECT-RULES.md\n- WORKFLOWS.md\n- HANDOFF.md\n- VERIFICATION.md\n- ESCALATION.md\n- CONTEXT.md\n- PROMPTS.md\nload_order:\n- PACK.md\n- CALIBRATION.md\n- OPERATING-INSTRUCTIONS.md\n- PROJECT-RULES.md\n- WORKFLOWS.md\n- HANDOFF.md\n- VERIFICATION.md\n- ESCALATION.md\n- CONTEXT.md\n- PROMPTS.md\n",
        ),
        document(
            "CALIBRATION.md",
            "# Calibration\n\nTreat the user as a senior project owner. Assume proposals are intentional requirements, ask for missing context when needed, and raise concerns with specific tradeoffs rather than generic best-practice objections.\n",
        ),
        document(
            "OPERATING-INSTRUCTIONS.md",
            "# Operating Instructions\n\nRead this pack at session start. Keep work tied to a concrete outcome, preserve user intent, avoid unnecessary fragmentation, and continue until the stated objective is verified or truly blocked.\n",
        ),
        document(
            "PROJECT-RULES.md",
            "# Project Rules\n\nUse repository instructions as the local source of truth. Do not bypass auth, workspace allowlisting, trace receipts, or safety boundaries to make a task appear complete.\n",
        ),
        document(
            "WORKFLOWS.md",
            "# Workflows\n\nDescribe repeatable procedures as guidance only. Context Packs do not execute code, call tools, create sessions, dispatch agents, or trigger schedules.\n",
        ),
        document(
            "HANDOFF.md",
            "# Handoff\n\nRecord current status, completed work, open decisions, validation evidence, and the next concrete action. Do not include private examples, credentials, private/runtime prompts, or raw model outputs.\n",
        ),
        document(
            "VERIFICATION.md",
            "# Verification\n\nRun focused tests for changed behavior, then the relevant package validation. Report exact validation results and any remaining unverified surfaces.\n",
        ),
        document(
            "ESCALATION.md",
            "# Escalation\n\nEscalate only when a decision, access boundary, or safety issue cannot be resolved locally. State the blocker, the minimum needed action, and the risk of proceeding without it.\n",
        ),
        document(
            "CONTEXT.md",
            "# Context\n\nThis demo pack represents durable organizational operating context for a governed coding-agent session. It is an operating package, not a Codex skill, workflow engine, or governance runtime.\n",
        ),
        document(
            "PROMPTS.md",
            "# Prompt Templates\n\nPrompt templates may be reusable text assets. A session or future schedule chooses whether to use one; the template does not execute by itself.\n",
        ),
    ]
}

fn document(filename: &str, content: &str) -> crate::context_packs::ContextPackDocumentInput {
    crate::context_packs::ContextPackDocumentInput {
        filename: filename.to_string(),
        content: content.to_string(),
        relative_path: None,
        content_type: None,
        load_order: None,
        required: None,
        file_kind: None,
        loadable: None,
        source_type: None,
    }
}

const DEMO_CONTEXT_PACK_NAME: &str = "Demo Engineering Context Pack";
const DEMO_USER_EMAILS: &[&str] = &["demo.developer@example.test", "demo.viewer@example.test"];

fn redacted_repo_url(value: &str) -> String {
    let Ok(mut url) = url::Url::parse(value) else {
        return "<invalid-repo-url>".to_string();
    };
    if url.scheme() != "https" {
        return format!("<{}-repo-url>", url.scheme());
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.to_string()
}

fn auth_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    if let Some(token) = bearer_token(headers)? {
        return Ok(token);
    }
    if let Some(token) = cookie_token(headers)? {
        return Ok(token);
    }
    Err(ApiError::new(
        StatusCode::UNAUTHORIZED,
        "missing or invalid API token",
    ))
}

fn bearer_token(headers: &HeaderMap) -> Result<Option<&str>, ApiError> {
    let Some(value) = headers.get(axum::http::header::AUTHORIZATION) else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| ApiError::new(StatusCode::UNAUTHORIZED, "missing or invalid API token"))?;
    let token = value
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty())
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "missing or invalid API token"))?;
    Ok(Some(token))
}

fn cookie_token(headers: &HeaderMap) -> Result<Option<&str>, ApiError> {
    let Some(value) = headers.get(axum::http::header::COOKIE) else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| ApiError::new(StatusCode::UNAUTHORIZED, "missing or invalid API token"))?;
    Ok(value.split(';').find_map(|part| {
        let part = part.trim();
        part.strip_prefix("lce_api_token=")
            .filter(|token| !token.is_empty())
    }))
}
