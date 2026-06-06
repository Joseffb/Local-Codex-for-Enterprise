use axum::body::Body;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;
use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

pub const TRACE_ID_HEADER: &str = "x-trace-id";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceContext {
    pub trace_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TraceResult {
    Allowed,
    Denied,
    Failed,
    Completed,
}

impl TraceResult {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Denied => "denied",
            Self::Failed => "failed",
            Self::Completed => "completed",
        }
    }

    pub fn from_storage(value: &str) -> Option<Self> {
        match value {
            "allowed" => Some(Self::Allowed),
            "denied" => Some(Self::Denied),
            "failed" => Some(Self::Failed),
            "completed" => Some(Self::Completed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct EvidenceRecordContext {
    pub trace_id: String,
    pub actor_user_id: Option<String>,
    pub workspace_id: Option<String>,
    pub session_id: Option<String>,
    pub worker_id: Option<String>,
}

impl EvidenceRecordContext {
    pub fn new(trace: &TraceContext) -> Self {
        Self {
            trace_id: trace.trace_id.clone(),
            actor_user_id: None,
            workspace_id: None,
            session_id: None,
            worker_id: None,
        }
    }

    pub fn actor(mut self, actor_user_id: impl Into<String>) -> Self {
        self.actor_user_id = Some(actor_user_id.into());
        self
    }

    pub fn workspace(mut self, workspace_id: impl Into<String>) -> Self {
        self.workspace_id = Some(workspace_id.into());
        self
    }

    pub fn session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    pub fn worker(mut self, worker_id: impl Into<String>) -> Self {
        self.worker_id = Some(worker_id.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct EvidenceRecord {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ExecutionReceipt {
    pub receipt_id: String,
    pub execution_id: String,
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

pub type AuditEvent = EvidenceRecord;
pub type PolicyDecisionLog = EvidenceRecord;
pub type WorkspaceAccessLog = EvidenceRecord;
pub type ToolInvocationLog = EvidenceRecord;
pub type ModelInvocationLog = EvidenceRecord;
pub type ApprovalRecord = EvidenceRecord;
pub type SecurityEvent = EvidenceRecord;

impl TraceContext {
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let trace_id = headers
            .get(TRACE_ID_HEADER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| Uuid::parse_str(value).ok())
            .map(|uuid| uuid.to_string())
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        Self { trace_id }
    }

    pub fn execution_id(&self) -> String {
        Uuid::new_v4().to_string()
    }
}

pub async fn trace_middleware(mut request: Request<Body>, next: Next) -> Response {
    let context = TraceContext::from_headers(request.headers());
    request.extensions_mut().insert(context.clone());
    let mut response = next.run(request).await;
    if let Ok(value) = HeaderValue::from_str(&context.trace_id) {
        response.headers_mut().insert(TRACE_ID_HEADER, value);
    }
    response
}
