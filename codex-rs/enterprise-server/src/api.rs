use axum::Json;
use axum::Router;
use axum::routing::get;
use serde::Deserialize;
use serde::Serialize;
use utoipa::OpenApi;
use utoipa::ToSchema;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    pub product: &'static str,
    pub status: &'static str,
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
#[openapi(paths(healthz), components(schemas(HealthResponse)))]
struct EnterpriseApi;

pub fn openapi_document() -> utoipa::openapi::OpenApi {
    EnterpriseApi::openapi()
}

pub fn build_router() -> Router {
    Router::new().route("/healthz", get(healthz))
}
