use crate::auth;
use crate::config::EnterpriseConfig;
use crate::rbac;
use crate::rbac::EnterpriseAction;
use crate::repo_clone;
use crate::storage::BootstrapInput;
use crate::storage::EnterpriseStore;
use crate::storage::InMemoryEnterpriseStore;
use crate::worker::WorkerRecord;
use crate::worker::WorkerRuntimeSupervisor;
use crate::worker::WorkerState;
use axum::Json;
use axum::Router;
use axum::extract::Path;
use axum::extract::Query;
use axum::extract::State;
use axum::extract::ws::Message as AxumWsMessage;
use axum::extract::ws::WebSocket;
use axum::extract::ws::WebSocketUpgrade;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::routing::post;
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
pub struct StartWorkerRequest {
    pub workspace_path: String,
    pub session_id: String,
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
pub struct CloneWorkspaceRequest {
    pub repo_url: String,
    pub workspace_root: String,
    pub destination_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CloneWorkspaceResponse {
    pub repo_url: String,
    pub workspace_root: String,
    pub workspace_path: String,
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
        if message.contains("canonicalize workspace") {
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
    paths(healthz, get_config, setup_enterprise::<InMemoryEnterpriseStore>, login::<InMemoryEnterpriseStore>, clone_workspace::<InMemoryEnterpriseStore>, list_workers::<InMemoryEnterpriseStore>, start_worker::<InMemoryEnterpriseStore>, stop_worker::<InMemoryEnterpriseStore>, issue_worker_handoff::<InMemoryEnterpriseStore>, consume_worker_handoff::<InMemoryEnterpriseStore>, worker_rpc::<InMemoryEnterpriseStore>),
    components(schemas(
        CloneWorkspaceRequest,
        CloneWorkspaceResponse,
        ConsumeWorkerHandoffRequest,
        ConsumeWorkerHandoffResponse,
        ConfigResponse,
        EnterpriseSetupRequest,
        EnterpriseSetupResponse,
        ErrorResponse,
        HealthResponse,
        LoginRequest,
        LoginResponse,
        StartWorkerRequest,
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
        .route("/healthz", get(healthz))
        .route("/v1/config", get(get_config::<S>))
        .route("/v1/setup/enterprise", post(setup_enterprise::<S>))
        .route("/v1/auth/login", post(login::<S>))
        .route("/v1/workspaces/clone", post(clone_workspace::<S>))
        .route(
            "/v1/workers",
            get(list_workers::<S>).post(start_worker::<S>),
        )
        .route(
            "/v1/workers/{worker_id}",
            axum::routing::delete(stop_worker::<S>),
        )
        .route(
            "/v1/workers/{worker_id}/handoff",
            post(issue_worker_handoff::<S>),
        )
        .route("/v1/workers/{worker_id}/rpc", get(worker_rpc::<S>))
        .route(
            "/v1/worker-handoffs/{jti}/consume",
            post(consume_worker_handoff::<S>),
        )
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
    let issued_token = auth::issue_api_token("owner").map_err(ApiError::internal)?;
    let outcome = state
        .store
        .bootstrap_enterprise(BootstrapInput {
            owner_email: request.owner_email,
            owner_password_hash: password_hash,
            workspace_roots: request.workspace_roots,
            issued_token_hash: issued_token.token_hash,
        })
        .await
        .map_err(ApiError::internal)?;
    record_audit(
        &state,
        Some(&outcome.owner_user_id),
        "enterprise.bootstrap",
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
                None,
                "auth.login.failure",
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
        Some(&principal.user_id),
        "auth.login.success",
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
    headers: HeaderMap,
) -> Result<Json<WorkersResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &principal, EnterpriseAction::ReadThreads).await?;
    let workers = state
        .store
        .list_workers(&principal)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(WorkersResponse { workers }))
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
    headers: HeaderMap,
    Json(request): Json<StartWorkerRequest>,
) -> Result<(StatusCode, Json<WorkerResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &principal, EnterpriseAction::StartWorker).await?;
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
        Some(&principal.user_id),
        "worker.start",
        serde_json::json!({
            "worker_id": worker.worker_id.clone(),
            "workspace_id": worker.workspace_id.clone(),
            "session_id": worker.session_id.clone(),
            "state": worker.state,
        }),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(WorkerResponse { worker })))
}

#[utoipa::path(
    post,
    path = "/v1/workspaces/clone",
    request_body = CloneWorkspaceRequest,
    responses(
        (status = 201, description = "Repository cloned into an allowlisted workspace root", body = CloneWorkspaceResponse),
        (status = 400, description = "Invalid repo URL or destination", body = ErrorResponse),
        (status = 401, description = "Missing or invalid API token", body = ErrorResponse),
        (status = 403, description = "Workspace root or repo host is not allowed", body = ErrorResponse)
    ),
    security(("bearer" = []))
)]
async fn clone_workspace<S>(
    State(state): State<AppState<S>>,
    headers: HeaderMap,
    Json(request): Json<CloneWorkspaceRequest>,
) -> Result<(StatusCode, Json<CloneWorkspaceResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &principal, EnterpriseAction::StartWorker).await?;
    let roots = state
        .store
        .list_workspace_roots(&principal)
        .await
        .map_err(ApiError::internal)?;
    let requested_root = std::path::PathBuf::from(&request.workspace_root)
        .canonicalize()
        .map_err(|error| {
            ApiError::storage(anyhow::anyhow!("canonicalize workspace root: {error}"))
        })?;
    let requested_root = requested_root.to_string_lossy().to_string();
    if !roots.iter().any(|root| root == &requested_root) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "workspace root is not allowlisted",
        ));
    }
    let plan = repo_clone::plan_clone(
        &request.repo_url,
        &requested_root,
        &request.destination_name,
    )
    .map_err(ApiError::storage)?;
    repo_clone::clone_repo(&plan)
        .await
        .map_err(ApiError::internal)?;
    record_audit(
        &state,
        Some(&principal.user_id),
        "workspace.clone",
        serde_json::json!({
            "repo_url": plan.repo_url.clone(),
            "workspace_root": plan.workspace_root.clone(),
            "workspace_path": plan.destination_path.clone(),
        }),
    )
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(CloneWorkspaceResponse {
            repo_url: plan.repo_url,
            workspace_root: plan.workspace_root,
            workspace_path: plan.destination_path,
        }),
    ))
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
    headers: HeaderMap,
    Path(worker_id): Path<String>,
) -> Result<Json<WorkerResponse>, ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &principal, EnterpriseAction::StartWorker).await?;
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
        Some(&principal.user_id),
        "worker.stop",
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
    path = "/v1/workers/{worker_id}/handoff",
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
    headers: HeaderMap,
    Path(worker_id): Path<String>,
) -> Result<(StatusCode, Json<WorkerHandoffResponse>), ApiError>
where
    S: EnterpriseStore,
{
    let principal = authenticate(&state, &headers).await?;
    authorize(&state, &principal, EnterpriseAction::StartWorker).await?;
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
        Some(&principal.user_id),
        "worker.handoff.issue",
        serde_json::json!({
            "jti": handoff.jti.clone(),
            "worker_id": handoff.worker_id.clone(),
            "workspace_id": handoff.workspace_id.clone(),
            "session_id": handoff.session_id.clone(),
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
        Some(&handoff.owner_user_id),
        "worker.handoff.consume",
        serde_json::json!({
            "jti": handoff.jti.clone(),
            "worker_id": handoff.worker_id.clone(),
            "workspace_id": handoff.workspace_id.clone(),
            "session_id": handoff.session_id.clone(),
        }),
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
        Some(&handoff.owner_user_id),
        "worker.rpc.connect",
        serde_json::json!({
            "jti": handoff.jti.clone(),
            "worker_id": handoff.worker_id.clone(),
            "workspace_id": handoff.workspace_id.clone(),
            "session_id": handoff.session_id.clone(),
        }),
    )
    .await?;
    let socket_path = handoff.socket_path;

    Ok(ws
        .on_upgrade(move |socket| async move {
            let _ = proxy_worker_rpc(socket, socket_path).await;
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
    let token = bearer_token(headers)?;
    state
        .store
        .authenticate_api_token(token)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "missing or invalid API token"))
}

async fn record_audit<S>(
    state: &AppState<S>,
    actor_user_id: Option<&str>,
    event_type: &str,
    event_json: serde_json::Value,
) -> Result<(), ApiError>
where
    S: EnterpriseStore,
{
    state
        .store
        .record_audit_event(actor_user_id, event_type, event_json)
        .await
        .map_err(ApiError::internal)
}

async fn proxy_worker_rpc(client_socket: WebSocket, socket_path: String) -> anyhow::Result<()> {
    let worker_stream = UnixStream::connect(FsPath::new(&socket_path)).await?;
    let (worker_socket, _response) = client_async("ws://localhost/", worker_stream).await?;
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
        result = client_to_worker => result?,
        result = worker_to_client => result?,
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
            Some(&principal.user_id),
            "rbac.deny",
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

fn bearer_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    let Some(value) = headers.get(axum::http::header::AUTHORIZATION) else {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "missing or invalid API token",
        ));
    };
    let value = value
        .to_str()
        .map_err(|_| ApiError::new(StatusCode::UNAUTHORIZED, "missing or invalid API token"))?;
    value
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty())
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "missing or invalid API token"))
}
