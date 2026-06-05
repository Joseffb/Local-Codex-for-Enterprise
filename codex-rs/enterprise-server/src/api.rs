use axum::Json;
use axum::Router;
use axum::routing::get;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

pub fn build_router() -> Router {
    Router::new().route("/healthz", get(|| async { Json(health_response()) }))
}
