use crate::auth;
use crate::config::EnterpriseConfig;
use crate::storage::BootstrapInput;
use crate::storage::EnterpriseStore;
use crate::storage::InMemoryEnterpriseStore;
use crate::worker::WorkerRecord;
use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::routing::post;
use serde::Deserialize;
use serde::Serialize;
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
}

impl<S> AppState<S> {
    pub fn new(store: S, config: EnterpriseConfig) -> Self {
        Self { store, config }
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
pub struct StartWorkerRequest {
    pub workspace_id: String,
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
    paths(healthz, get_config, setup_enterprise::<InMemoryEnterpriseStore>, list_workers::<InMemoryEnterpriseStore>, start_worker::<InMemoryEnterpriseStore>),
    components(schemas(
        ConfigResponse,
        EnterpriseSetupRequest,
        EnterpriseSetupResponse,
        ErrorResponse,
        HealthResponse,
        StartWorkerRequest,
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
        .route(
            "/v1/workers",
            get(list_workers::<S>).post(start_worker::<S>),
        )
        .with_state(AppState::new(store, config))
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
    let worker = state
        .store
        .start_worker(&principal, request.workspace_id, request.session_id)
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(WorkerResponse { worker })))
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
