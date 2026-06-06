use axum::body::Body;
use axum::body::to_bytes;
use axum::http::Request;
use axum::http::StatusCode;
use codex_enterprise_server::api;
use codex_enterprise_server::auth;
use codex_enterprise_server::config::EnterpriseConfig;
use codex_enterprise_server::config::ServerMode;
use codex_enterprise_server::rbac;
use codex_enterprise_server::rbac::EnterpriseAction;
use codex_enterprise_server::rbac::EnterpriseRole;
use codex_enterprise_server::repo_clone;
use codex_enterprise_server::setup::BootstrapReceipt;
use codex_enterprise_server::setup::SetupMode;
use codex_enterprise_server::storage::InMemoryEnterpriseStore;
use codex_enterprise_server::worker::WorkerState;
use codex_enterprise_server::worker::WorkerSupervisor;
use codex_enterprise_server::workspace::WorkspacePolicy;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

#[test]
fn default_config_selects_enterprise_mode_and_local_model_defaults() {
    let config = EnterpriseConfig::default();

    assert_eq!(config.mode, ServerMode::Enterprise);
    assert_eq!(config.default_model_provider, "docker-model-runner");
    assert_eq!(config.default_model, "ai/qwen3-coder");
}

#[test]
fn health_response_names_enterprise_product() {
    let health = api::health_response();

    assert_eq!(health.product, "Local Codex for Enterprise");
    assert_eq!(health.status, "ok");
}

#[test]
fn owner_can_administer_but_viewer_cannot() {
    assert!(rbac::role_allows(
        EnterpriseRole::Owner,
        EnterpriseAction::AdministerUsers
    ));
    assert!(!rbac::role_allows(
        EnterpriseRole::Viewer,
        EnterpriseAction::AdministerUsers
    ));
}

#[test]
fn password_hashes_verify_without_storing_plaintext() {
    let hash = auth::hash_password("correct horse battery staple").expect("hash password");

    assert!(hash.starts_with("$argon2"));
    assert_ne!(hash, "correct horse battery staple");
    assert!(auth::verify_password("correct horse battery staple", &hash).expect("verify password"));
    assert!(!auth::verify_password("wrong", &hash).expect("reject wrong password"));
}

#[tokio::test]
async fn casbin_policy_allows_owner_but_rejects_viewer_admin() {
    assert!(
        rbac::casbin_role_allows(EnterpriseRole::Owner, EnterpriseAction::AdministerUsers)
            .await
            .expect("owner policy")
    );
    assert!(
        !rbac::casbin_role_allows(EnterpriseRole::Viewer, EnterpriseAction::AdministerUsers)
            .await
            .expect("viewer policy")
    );
}

#[test]
fn openapi_document_describes_health_route() {
    let document = api::openapi_document();

    assert!(document.paths.paths.contains_key("/healthz"));
}

#[tokio::test]
async fn setup_endpoint_bootstraps_once_and_returns_owner_api_token() {
    let router = api::build_test_router();
    let temp = tempfile::tempdir().expect("temp dir");
    let workspace_root = temp.path().join("workspaces");
    std::fs::create_dir_all(&workspace_root).expect("workspace root");
    let request = Request::builder()
        .method("POST")
        .uri("/v1/setup/enterprise")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "owner_email": "owner@example.com",
                "owner_password": "correct horse battery staple",
                "workspace_roots": [workspace_root]
            })
            .to_string(),
        ))
        .expect("request");

    let response = router.clone().oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    assert_eq!(json["owner_email"], "owner@example.com");
    assert!(
        json["api_token"]
            .as_str()
            .expect("api token")
            .starts_with("lce_owner_")
    );

    let second = Request::builder()
        .method("POST")
        .uri("/v1/setup/enterprise")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "owner_email": "owner@example.com",
                "owner_password": "correct horse battery staple",
                "workspace_roots": [temp.path().join("workspaces")]
            })
            .to_string(),
        ))
        .expect("request");

    let response = router.oneshot(second).await.expect("second response");
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn traced_responses_emit_generated_preserved_and_replaced_trace_ids() {
    let router = api::build_test_router();
    let generated = Request::builder()
        .method("GET")
        .uri("/v1/config")
        .body(Body::empty())
        .expect("request");
    let response = router
        .clone()
        .oneshot(generated)
        .await
        .expect("generated trace response");
    let generated_trace = response
        .headers()
        .get("x-trace-id")
        .expect("generated trace header")
        .to_str()
        .expect("generated trace string");
    Uuid::parse_str(generated_trace).expect("generated trace is uuid");

    let supplied_trace = Uuid::new_v4().to_string();
    let preserved = Request::builder()
        .method("GET")
        .uri("/v1/config")
        .header("x-trace-id", &supplied_trace)
        .body(Body::empty())
        .expect("request");
    let response = router
        .clone()
        .oneshot(preserved)
        .await
        .expect("preserved trace response");
    assert_eq!(
        response
            .headers()
            .get("x-trace-id")
            .expect("preserved trace header")
            .to_str()
            .expect("preserved trace string"),
        supplied_trace
    );

    let replaced = Request::builder()
        .method("GET")
        .uri("/v1/config")
        .header("x-trace-id", "not-a-uuid")
        .body(Body::empty())
        .expect("request");
    let response = router
        .oneshot(replaced)
        .await
        .expect("replaced trace response");
    let replaced_trace = response
        .headers()
        .get("x-trace-id")
        .expect("replaced trace header")
        .to_str()
        .expect("replaced trace string");
    Uuid::parse_str(replaced_trace).expect("replaced trace is uuid");
    assert_ne!(replaced_trace, "not-a-uuid");
}

#[tokio::test]
async fn login_endpoint_issues_token_for_bootstrapped_owner() {
    let router = api::build_test_router();
    let temp = tempfile::tempdir().expect("temp dir");
    let workspace_root = temp.path().join("workspaces");
    std::fs::create_dir_all(&workspace_root).expect("workspace root");
    let setup = Request::builder()
        .method("POST")
        .uri("/v1/setup/enterprise")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "owner_email": "owner@example.com",
                "owner_password": "correct horse battery staple",
                "workspace_roots": [workspace_root]
            })
            .to_string(),
        ))
        .expect("request");
    let response = router.clone().oneshot(setup).await.expect("setup response");
    assert_eq!(response.status(), StatusCode::CREATED);

    let login = Request::builder()
        .method("POST")
        .uri("/v1/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "email": "owner@example.com",
                "password": "correct horse battery staple"
            })
            .to_string(),
        ))
        .expect("request");
    let response = router.clone().oneshot(login).await.expect("login response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    assert_eq!(json["email"], "owner@example.com");
    assert_eq!(json["role"], "owner");
    let token = json["api_token"].as_str().expect("api token");
    assert!(token.starts_with("lce_login_"));

    let list_workers = Request::builder()
        .method("GET")
        .uri("/v1/workers")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("request");
    let response = router.oneshot(list_workers).await.expect("list response");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn workers_api_requires_token_then_tracks_started_worker() {
    let temp = tempfile::tempdir().expect("temp dir");
    let workspace_root = temp.path().join("workspaces");
    let project = workspace_root.join("project");
    let socket_dir = temp.path().join("sockets");
    let log_dir = temp.path().join("logs");
    std::fs::create_dir_all(&project).expect("project workspace");

    let mut config = EnterpriseConfig::default();
    config.worker_command = "/bin/sh".to_string();
    config.worker_args = vec!["-c".to_string(), "sleep 30".to_string()];
    config.worker_socket_dir = socket_dir.to_string_lossy().to_string();
    config.worker_log_dir = log_dir.to_string_lossy().to_string();
    let router = api::build_router_with_store(InMemoryEnterpriseStore::default(), config);
    let unauthenticated = Request::builder()
        .method("GET")
        .uri("/v1/workers")
        .body(Body::empty())
        .expect("request");

    let response = router
        .clone()
        .oneshot(unauthenticated)
        .await
        .expect("unauthenticated response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let setup = Request::builder()
        .method("POST")
        .uri("/v1/setup/enterprise")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "owner_email": "owner@example.com",
                "owner_password": "correct horse battery staple",
                "workspace_roots": [workspace_root]
            })
            .to_string(),
        ))
        .expect("request");
    let setup_response = router.clone().oneshot(setup).await.expect("setup response");
    let body = to_bytes(setup_response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    let token = json["api_token"].as_str().expect("api token");

    let start_worker = Request::builder()
        .method("POST")
        .uri("/v1/workers")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(
            serde_json::json!({
                "workspace_path": project,
                "session_id": "session-1"
            })
            .to_string(),
        ))
        .expect("request");
    let response = router
        .clone()
        .oneshot(start_worker)
        .await
        .expect("start worker response");
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("start worker body");
    let started: serde_json::Value = serde_json::from_slice(&body).expect("start json body");
    assert_eq!(started["worker"]["state"], "Running");
    assert!(started["worker"]["pid"].as_u64().expect("pid") > 0);
    assert!(
        started["worker"]["log_path"]
            .as_str()
            .expect("log path")
            .starts_with(log_dir.to_str().expect("log dir"))
    );
    let worker_id = started["worker"]["worker_id"]
        .as_str()
        .expect("worker id")
        .to_string();

    let list_workers = Request::builder()
        .method("GET")
        .uri("/v1/workers")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("request");
    let response = router
        .clone()
        .oneshot(list_workers)
        .await
        .expect("list worker response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    assert_eq!(json["workers"].as_array().expect("workers").len(), 1);
    assert_eq!(json["workers"][0]["state"], "Running");
    assert_eq!(json["workers"][0]["session_id"], "session-1");

    let list_sessions = Request::builder()
        .method("GET")
        .uri("/v1/sessions")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("request");
    let response = router
        .clone()
        .oneshot(list_sessions)
        .await
        .expect("list session response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("sessions body");
    let sessions: serde_json::Value = serde_json::from_slice(&body).expect("sessions json body");
    assert_eq!(sessions["sessions"].as_array().expect("sessions").len(), 1);
    assert_eq!(sessions["sessions"][0]["session_id"], "session-1");
    assert_eq!(sessions["sessions"][0]["last_worker_id"], worker_id);

    let stop_worker = Request::builder()
        .method("DELETE")
        .uri(format!("/v1/workers/{worker_id}"))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("request");
    let response = router
        .oneshot(stop_worker)
        .await
        .expect("stop worker response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("stop body");
    let stopped: serde_json::Value = serde_json::from_slice(&body).expect("stop json body");
    assert_eq!(stopped["worker"]["state"], "Stopped");
}

#[tokio::test]
async fn sessions_api_persists_workspace_bound_thread_history() {
    let router = api::build_test_router();
    let temp = tempfile::tempdir().expect("temp dir");
    let workspace_root = temp.path().join("workspaces");
    let project = workspace_root.join("project");
    std::fs::create_dir_all(&project).expect("project workspace");

    let setup = Request::builder()
        .method("POST")
        .uri("/v1/setup/enterprise")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "owner_email": "owner@example.com",
                "owner_password": "correct horse battery staple",
                "workspace_roots": [workspace_root]
            })
            .to_string(),
        ))
        .expect("request");
    let setup_response = router.clone().oneshot(setup).await.expect("setup response");
    let body = to_bytes(setup_response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    let token = json["api_token"].as_str().expect("api token");
    let owner_user_id = json["owner_user_id"].as_str().expect("owner user id");

    let create_session = Request::builder()
        .method("POST")
        .uri("/v1/sessions")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(
            serde_json::json!({
                "workspace_path": project,
                "title": "Governance sprint"
            })
            .to_string(),
        ))
        .expect("request");
    let response = router
        .clone()
        .oneshot(create_session)
        .await
        .expect("create session response");
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("create session body");
    let created: serde_json::Value = serde_json::from_slice(&body).expect("create session json");
    let session_id = created["session"]["session_id"]
        .as_str()
        .expect("session id")
        .to_string();
    assert_eq!(created["session"]["owner_user_id"], owner_user_id);
    assert_eq!(created["session"]["title"], "Governance sprint");
    assert_eq!(
        created["session"]["workspace_path"],
        project
            .canonicalize()
            .expect("canonical project")
            .to_str()
            .expect("canonical project str")
    );

    let list_sessions = Request::builder()
        .method("GET")
        .uri("/v1/sessions")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("request");
    let response = router
        .clone()
        .oneshot(list_sessions)
        .await
        .expect("list session response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("list session body");
    let listed: serde_json::Value = serde_json::from_slice(&body).expect("list session json");
    assert_eq!(listed["sessions"].as_array().expect("sessions").len(), 1);
    assert_eq!(listed["sessions"][0]["session_id"], session_id);

    let get_session = Request::builder()
        .method("GET")
        .uri(format!("/v1/sessions/{session_id}"))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("request");
    let response = router
        .oneshot(get_session)
        .await
        .expect("get session response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("get session body");
    let fetched: serde_json::Value = serde_json::from_slice(&body).expect("get session json");
    assert_eq!(fetched["session"]["session_id"], session_id);
}

#[tokio::test]
async fn workers_api_rejects_session_workspace_mismatch() {
    let temp = tempfile::tempdir().expect("temp dir");
    let workspace_root = temp.path().join("workspaces");
    let project_a = workspace_root.join("project-a");
    let project_b = workspace_root.join("project-b");
    std::fs::create_dir_all(&project_a).expect("project a workspace");
    std::fs::create_dir_all(&project_b).expect("project b workspace");

    let mut config = EnterpriseConfig::default();
    config.worker_command = "/bin/sh".to_string();
    config.worker_args = vec!["-c".to_string(), "sleep 30".to_string()];
    config.worker_socket_dir = temp.path().join("sockets").to_string_lossy().to_string();
    config.worker_log_dir = temp.path().join("logs").to_string_lossy().to_string();
    let router = api::build_router_with_store(InMemoryEnterpriseStore::default(), config);

    let setup = Request::builder()
        .method("POST")
        .uri("/v1/setup/enterprise")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "owner_email": "owner@example.com",
                "owner_password": "correct horse battery staple",
                "workspace_roots": [workspace_root]
            })
            .to_string(),
        ))
        .expect("request");
    let setup_response = router.clone().oneshot(setup).await.expect("setup response");
    let body = to_bytes(setup_response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    let token = json["api_token"].as_str().expect("api token");

    let create_session = Request::builder()
        .method("POST")
        .uri("/v1/sessions")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(
            serde_json::json!({
                "session_id": "session-1",
                "workspace_path": project_a,
                "title": "Project A"
            })
            .to_string(),
        ))
        .expect("request");
    let response = router
        .clone()
        .oneshot(create_session)
        .await
        .expect("create session response");
    assert_eq!(response.status(), StatusCode::CREATED);

    let start_worker = Request::builder()
        .method("POST")
        .uri("/v1/workers")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(
            serde_json::json!({
                "workspace_path": project_b,
                "session_id": "session-1"
            })
            .to_string(),
        ))
        .expect("request");
    let response = router
        .oneshot(start_worker)
        .await
        .expect("start worker response");
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn viewer_token_can_read_but_cannot_start_worker() {
    let temp = tempfile::tempdir().expect("temp dir");
    let project = temp.path().join("project");
    std::fs::create_dir_all(&project).expect("project workspace");

    let issued = auth::issue_api_token("viewer").expect("viewer token");
    let store = InMemoryEnterpriseStore::default();
    store
        .insert_user_for_test(
            "viewer@example.com",
            EnterpriseRole::Viewer,
            issued.token_hash.clone(),
        )
        .await;
    let router = api::build_router_with_store(store, EnterpriseConfig::default());

    let list_workers = Request::builder()
        .method("GET")
        .uri("/v1/workers")
        .header("authorization", format!("Bearer {}", issued.plaintext))
        .body(Body::empty())
        .expect("request");
    let response = router
        .clone()
        .oneshot(list_workers)
        .await
        .expect("list response");
    assert_eq!(response.status(), StatusCode::OK);

    let start_worker = Request::builder()
        .method("POST")
        .uri("/v1/workers")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", issued.plaintext))
        .body(Body::from(
            serde_json::json!({
                "workspace_path": project,
                "session_id": "session-1"
            })
            .to_string(),
        ))
        .expect("request");
    let response = router.oneshot(start_worker).await.expect("start response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_and_rbac_decisions_are_audited_without_plaintext_secrets() {
    let temp = tempfile::tempdir().expect("temp dir");
    let project = temp.path().join("project");
    std::fs::create_dir_all(&project).expect("project workspace");

    let store = InMemoryEnterpriseStore::default();
    let viewer_token = auth::issue_api_token("viewer").expect("viewer token");
    store
        .insert_user_for_test(
            "viewer@example.com",
            EnterpriseRole::Viewer,
            viewer_token.token_hash.clone(),
        )
        .await;
    let router = api::build_router_with_store(store.clone(), EnterpriseConfig::default());

    let login = Request::builder()
        .method("POST")
        .uri("/v1/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "email": "viewer@example.com",
                "password": "wrong password"
            })
            .to_string(),
        ))
        .expect("request");
    let response = router.clone().oneshot(login).await.expect("login response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let start_worker = Request::builder()
        .method("POST")
        .uri("/v1/workers")
        .header("content-type", "application/json")
        .header(
            "authorization",
            format!("Bearer {}", viewer_token.plaintext),
        )
        .body(Body::from(
            serde_json::json!({
                "workspace_path": project,
                "session_id": "session-1"
            })
            .to_string(),
        ))
        .expect("request");
    let response = router.oneshot(start_worker).await.expect("start response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let audit = store.audit_events_for_test().await;
    assert!(
        audit
            .iter()
            .any(|event| event.event_type == "auth.login.failure")
    );
    assert!(audit.iter().any(|event| event.event_type == "rbac.deny"));
    let audit_json = serde_json::to_string(&audit).expect("audit json");
    assert!(!audit_json.contains("wrong password"));
    assert!(!audit_json.contains(&viewer_token.plaintext));
}

#[tokio::test]
async fn worker_handoff_tokens_are_scoped_and_single_use() {
    let temp = tempfile::tempdir().expect("temp dir");
    let workspace_root = temp.path().join("workspaces");
    let project = workspace_root.join("project");
    let socket_dir = temp.path().join("sockets");
    let log_dir = temp.path().join("logs");
    std::fs::create_dir_all(&project).expect("project workspace");

    let mut config = EnterpriseConfig::default();
    config.worker_command = "/bin/sh".to_string();
    config.worker_args = vec!["-c".to_string(), "sleep 30".to_string()];
    config.worker_socket_dir = socket_dir.to_string_lossy().to_string();
    config.worker_log_dir = log_dir.to_string_lossy().to_string();
    config.handoff_token_secret = "test-handoff-secret".to_string();
    config.handoff_token_ttl_seconds = 120;
    let router = api::build_router_with_store(InMemoryEnterpriseStore::default(), config);

    let setup = Request::builder()
        .method("POST")
        .uri("/v1/setup/enterprise")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "owner_email": "owner@example.com",
                "owner_password": "correct horse battery staple",
                "workspace_roots": [workspace_root]
            })
            .to_string(),
        ))
        .expect("request");
    let setup_response = router.clone().oneshot(setup).await.expect("setup response");
    let body = to_bytes(setup_response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    let token = json["api_token"].as_str().expect("api token");
    let owner_user_id = json["owner_user_id"].as_str().expect("owner user id");

    let start_worker = Request::builder()
        .method("POST")
        .uri("/v1/workers")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(
            serde_json::json!({
                "workspace_path": project,
                "session_id": "session-1"
            })
            .to_string(),
        ))
        .expect("request");
    let response = router
        .clone()
        .oneshot(start_worker)
        .await
        .expect("start worker response");
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("start worker body");
    let started: serde_json::Value = serde_json::from_slice(&body).expect("start json body");
    let worker_id = started["worker"]["worker_id"]
        .as_str()
        .expect("worker id")
        .to_string();

    let issue_handoff = Request::builder()
        .method("POST")
        .uri(format!("/v1/workers/{worker_id}/handoff"))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("request");
    let response = router
        .clone()
        .oneshot(issue_handoff)
        .await
        .expect("handoff response");
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("handoff body");
    let handoff: serde_json::Value = serde_json::from_slice(&body).expect("handoff json body");
    let handoff_token = handoff["handoff_token"]
        .as_str()
        .expect("handoff token")
        .to_string();
    let jti = handoff["jti"].as_str().expect("jti").to_string();
    assert_eq!(handoff["worker_id"], worker_id);
    assert_eq!(handoff["session_id"], "session-1");
    assert_eq!(handoff["owner_user_id"], owner_user_id);
    assert!(
        handoff["socket_path"]
            .as_str()
            .expect("socket path")
            .starts_with(socket_dir.to_str().expect("socket dir"))
    );

    let claims = auth::decode_worker_handoff_token(&handoff_token, "test-handoff-secret")
        .expect("decode handoff token");
    assert_eq!(claims.sub, owner_user_id);
    assert_eq!(claims.worker_id, worker_id);
    assert_eq!(claims.session_id, "session-1");
    assert_eq!(claims.jti, jti);

    let consume_handoff = Request::builder()
        .method("POST")
        .uri(format!("/v1/worker-handoffs/{jti}/consume"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "handoff_token": handoff_token
            })
            .to_string(),
        ))
        .expect("request");
    let response = router
        .clone()
        .oneshot(consume_handoff)
        .await
        .expect("consume response");
    assert_eq!(response.status(), StatusCode::OK);

    let replay = Request::builder()
        .method("POST")
        .uri(format!("/v1/worker-handoffs/{jti}/consume"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "handoff_token": handoff_token
            })
            .to_string(),
        ))
        .expect("request");
    let response = router.oneshot(replay).await.expect("replay response");
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn trace_continues_across_session_worker_and_handoff_receipts() {
    let temp = tempfile::tempdir().expect("temp dir");
    let workspace_root = temp.path().join("workspaces");
    let project = workspace_root.join("project");
    let socket_dir = temp.path().join("sockets");
    let log_dir = temp.path().join("logs");
    std::fs::create_dir_all(&project).expect("project workspace");

    let mut config = EnterpriseConfig::default();
    config.worker_command = "/bin/sh".to_string();
    config.worker_args = vec!["-c".to_string(), "sleep 30".to_string()];
    config.worker_socket_dir = socket_dir.to_string_lossy().to_string();
    config.worker_log_dir = log_dir.to_string_lossy().to_string();
    let trace_id = Uuid::new_v4().to_string();
    let store = InMemoryEnterpriseStore::default();
    let router = api::build_router_with_store(store.clone(), config);

    let setup = Request::builder()
        .method("POST")
        .uri("/v1/setup/enterprise")
        .header("content-type", "application/json")
        .header("x-trace-id", &trace_id)
        .body(Body::from(
            serde_json::json!({
                "owner_email": "owner@example.com",
                "owner_password": "correct horse battery staple",
                "workspace_roots": [workspace_root]
            })
            .to_string(),
        ))
        .expect("request");
    let setup_response = router.clone().oneshot(setup).await.expect("setup response");
    let body = to_bytes(setup_response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    let token = json["api_token"].as_str().expect("api token");
    let owner_user_id = json["owner_user_id"].as_str().expect("owner user id");

    let create_session = Request::builder()
        .method("POST")
        .uri("/v1/sessions")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("x-trace-id", &trace_id)
        .body(Body::from(
            serde_json::json!({
                "session_id": "session-1",
                "workspace_path": project,
                "title": "Trace sprint"
            })
            .to_string(),
        ))
        .expect("request");
    let response = router
        .clone()
        .oneshot(create_session)
        .await
        .expect("create session response");
    assert_eq!(response.status(), StatusCode::CREATED);

    let start_worker = Request::builder()
        .method("POST")
        .uri("/v1/workers")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("x-trace-id", &trace_id)
        .body(Body::from(
            serde_json::json!({
                "workspace_path": project,
                "session_id": "session-1"
            })
            .to_string(),
        ))
        .expect("request");
    let response = router
        .clone()
        .oneshot(start_worker)
        .await
        .expect("start worker response");
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("start worker body");
    let started: serde_json::Value = serde_json::from_slice(&body).expect("start json body");
    let worker_id = started["worker"]["worker_id"]
        .as_str()
        .expect("worker id")
        .to_string();
    let workspace_id = started["worker"]["workspace_id"]
        .as_str()
        .expect("workspace id")
        .to_string();

    let issue_handoff = Request::builder()
        .method("POST")
        .uri(format!("/v1/workers/{worker_id}/handoff"))
        .header("authorization", format!("Bearer {token}"))
        .header("x-trace-id", &trace_id)
        .body(Body::empty())
        .expect("request");
    let response = router
        .oneshot(issue_handoff)
        .await
        .expect("handoff response");
    assert_eq!(response.status(), StatusCode::CREATED);

    let audit = store.audit_events_for_test().await;
    let traced_events = audit
        .iter()
        .filter(|event| {
            matches!(
                event.event_type.as_str(),
                "session.create" | "worker.start" | "worker.handoff.issue"
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(traced_events.len(), 3);
    assert!(traced_events.iter().all(|event| event.trace_id == trace_id));
    assert!(
        traced_events
            .iter()
            .all(|event| event.result.as_str() == "completed")
    );
    assert!(
        traced_events
            .iter()
            .all(|event| event.actor_user_id.as_deref() == Some(owner_user_id))
    );

    let receipts = store.execution_receipts_for_test().await;
    assert!(receipts.iter().any(|receipt| {
        receipt.trace_id == trace_id
            && receipt.actor_user_id.as_deref() == Some(owner_user_id)
            && receipt.workspace_id.as_deref() == Some(workspace_id.as_str())
            && receipt.session_id.as_deref() == Some("session-1")
            && receipt.worker_id.as_deref() == Some(worker_id.as_str())
            && receipt.event_type == "worker.start"
            && receipt.result.as_str() == "completed"
    }));
    assert!(receipts.iter().all(|receipt| {
        Uuid::parse_str(&receipt.execution_id).is_ok() && Uuid::parse_str(&receipt.trace_id).is_ok()
    }));
}

#[tokio::test]
async fn trace_metadata_redacts_passwords_tokens_and_credentialed_repo_urls() {
    let temp = tempfile::tempdir().expect("temp dir");
    let workspace_root = temp.path().join("workspaces");
    std::fs::create_dir_all(&workspace_root).expect("workspace root");
    let trace_id = Uuid::new_v4().to_string();
    let store = InMemoryEnterpriseStore::default();
    let router = api::build_router_with_store(store.clone(), EnterpriseConfig::default());

    let setup = Request::builder()
        .method("POST")
        .uri("/v1/setup/enterprise")
        .header("content-type", "application/json")
        .header("x-trace-id", &trace_id)
        .body(Body::from(
            serde_json::json!({
                "owner_email": "owner@example.com",
                "owner_password": "correct horse battery staple",
                "workspace_roots": [workspace_root]
            })
            .to_string(),
        ))
        .expect("request");
    let setup_response = router.clone().oneshot(setup).await.expect("setup response");
    let body = to_bytes(setup_response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    let token = json["api_token"].as_str().expect("api token");

    let login = Request::builder()
        .method("POST")
        .uri("/v1/auth/login")
        .header("content-type", "application/json")
        .header("x-trace-id", &trace_id)
        .body(Body::from(
            serde_json::json!({
                "email": "owner@example.com",
                "password": "wrong secret password"
            })
            .to_string(),
        ))
        .expect("request");
    let response = router.clone().oneshot(login).await.expect("login response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let clone = Request::builder()
        .method("POST")
        .uri("/v1/workspaces/clone")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("x-trace-id", &trace_id)
        .body(Body::from(
            serde_json::json!({
                "repo_url": "https://secret-token@example.com/org/repo.git",
                "workspace_root": workspace_root,
                "destination_name": "repo"
            })
            .to_string(),
        ))
        .expect("request");
    let response = router.clone().oneshot(clone).await.expect("clone response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let file_clone = Request::builder()
        .method("POST")
        .uri("/v1/workspaces/clone")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("x-trace-id", &trace_id)
        .body(Body::from(
            serde_json::json!({
                "repo_url": "file:///private/local-secret/repo.git",
                "workspace_root": workspace_root,
                "destination_name": "repo"
            })
            .to_string(),
        ))
        .expect("request");
    let response = router
        .oneshot(file_clone)
        .await
        .expect("file clone response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let audit = store.audit_events_for_test().await;
    assert!(
        audit
            .iter()
            .any(|event| event.event_type == "auth.login.failure"
                && event.result.as_str() == "denied"
                && event.trace_id == trace_id)
    );
    assert!(
        audit
            .iter()
            .any(|event| event.event_type == "workspace.clone"
                && event.result.as_str() == "denied"
                && event.trace_id == trace_id)
    );
    let evidence_json = serde_json::to_string(&audit).expect("audit json");
    assert!(!evidence_json.contains("wrong secret password"));
    assert!(!evidence_json.contains(token));
    assert!(!evidence_json.contains("Bearer"));
    assert!(!evidence_json.contains("secret-token"));
    assert!(!evidence_json.contains("local-secret"));
}

#[tokio::test]
async fn workers_api_rejects_workspace_escape_before_launching_worker() {
    let temp = tempfile::tempdir().expect("temp dir");
    let workspace_root = temp.path().join("workspaces");
    let allowed_project = workspace_root.join("project");
    let outside_project = temp.path().join("outside");
    std::fs::create_dir_all(&allowed_project).expect("allowed project");
    std::fs::create_dir_all(&outside_project).expect("outside project");

    let mut config = EnterpriseConfig::default();
    config.worker_command = "/bin/sh".to_string();
    config.worker_args = vec!["-c".to_string(), "sleep 30".to_string()];
    config.worker_socket_dir = temp.path().join("sockets").to_string_lossy().to_string();
    config.worker_log_dir = temp.path().join("logs").to_string_lossy().to_string();
    let router = api::build_router_with_store(InMemoryEnterpriseStore::default(), config);

    let setup = Request::builder()
        .method("POST")
        .uri("/v1/setup/enterprise")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "owner_email": "owner@example.com",
                "owner_password": "correct horse battery staple",
                "workspace_roots": [workspace_root]
            })
            .to_string(),
        ))
        .expect("request");
    let setup_response = router.clone().oneshot(setup).await.expect("setup response");
    let body = to_bytes(setup_response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    let token = json["api_token"].as_str().expect("api token");

    let start_worker = Request::builder()
        .method("POST")
        .uri("/v1/workers")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(
            serde_json::json!({
                "workspace_path": outside_project,
                "session_id": "session-1"
            })
            .to_string(),
        ))
        .expect("request");
    let response = router
        .oneshot(start_worker)
        .await
        .expect("start worker response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[test]
fn repo_clone_plan_rejects_unsafe_urls_and_destinations() {
    let temp = tempfile::tempdir().expect("temp dir");

    let valid = repo_clone::plan_clone("https://example.com/org/repo.git", temp.path(), "repo")
        .expect("https repo plan");
    assert_eq!(valid.destination_name, "repo");

    let file_url = repo_clone::plan_clone("file:///tmp/repo.git", temp.path(), "repo");
    assert!(
        file_url
            .expect_err("file url rejected")
            .to_string()
            .contains("https")
    );

    let credentials = repo_clone::plan_clone(
        "https://token@example.com/org/repo.git",
        temp.path(),
        "repo",
    );
    assert!(
        credentials
            .expect_err("credentials rejected")
            .to_string()
            .contains("credentials")
    );

    let metadata = repo_clone::plan_clone(
        "https://169.254.169.254/latest/meta-data/repo.git",
        temp.path(),
        "repo",
    );
    assert!(
        metadata
            .expect_err("metadata host rejected")
            .to_string()
            .contains("not allowed")
    );

    let traversal =
        repo_clone::plan_clone("https://example.com/org/repo.git", temp.path(), "../repo");
    assert!(
        traversal
            .expect_err("path destination rejected")
            .to_string()
            .contains("destination")
    );
}

#[tokio::test]
async fn clone_workspace_rejects_unsafe_repo_before_git_runs() {
    let router = api::build_test_router();
    let temp = tempfile::tempdir().expect("temp dir");
    let workspace_root = temp.path().join("workspaces");
    std::fs::create_dir_all(&workspace_root).expect("workspace root");
    let setup = Request::builder()
        .method("POST")
        .uri("/v1/setup/enterprise")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "owner_email": "owner@example.com",
                "owner_password": "correct horse battery staple",
                "workspace_roots": [workspace_root]
            })
            .to_string(),
        ))
        .expect("request");
    let setup_response = router.clone().oneshot(setup).await.expect("setup response");
    let body = to_bytes(setup_response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    let token = json["api_token"].as_str().expect("api token");

    let clone = Request::builder()
        .method("POST")
        .uri("/v1/workspaces/clone")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(
            serde_json::json!({
                "repo_url": "file:///tmp/repo.git",
                "workspace_root": workspace_root,
                "destination_name": "repo"
            })
            .to_string(),
        ))
        .expect("request");
    let response = router.oneshot(clone).await.expect("clone response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn api_tokens_are_opaque_and_hash_stored() {
    let issued = auth::issue_api_token("local-test").expect("issue token");

    assert!(issued.plaintext.starts_with("lce_local-test_"));
    assert_ne!(issued.plaintext, issued.token_hash);
    assert!(auth::verify_api_token(
        &issued.plaintext,
        &issued.token_hash
    ));
}

#[test]
fn websocket_handoff_token_is_purpose_bound_and_short_lived() {
    let token = auth::issue_worker_handoff_token(
        "user-1",
        "workspace-1",
        "session-1",
        "worker-1",
        Duration::from_secs(120),
        "test-secret",
    )
    .expect("issue handoff token");

    let claims =
        auth::decode_worker_handoff_token(&token.jwt, "test-secret").expect("decode token");
    assert_eq!(claims.sub, "user-1");
    assert_eq!(claims.workspace_id, "workspace-1");
    assert_eq!(claims.session_id, "session-1");
    assert_eq!(claims.worker_id, "worker-1");
    assert_eq!(claims.aud, "codex-worker-ws");
    assert_eq!(claims.jti, token.jti);
}

#[test]
fn workspace_policy_rejects_symlink_escape() {
    let temp = tempfile::tempdir().expect("temp dir");
    let allowed = temp.path().join("allowed");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&allowed).expect("create allowed");
    std::fs::create_dir_all(&outside).expect("create outside");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, allowed.join("link")).expect("symlink");

    let policy = WorkspacePolicy::new(vec![allowed.clone()]).expect("policy");
    assert!(policy.authorize(&allowed).expect("allowed").allowed);
    #[cfg(unix)]
    assert!(
        !policy
            .authorize(allowed.join("link"))
            .expect("symlink rejected")
            .allowed
    );
}

#[test]
fn worker_supervisor_tracks_starting_worker() {
    let mut supervisor = WorkerSupervisor::default();
    let worker = supervisor.starting("user-1", "workspace-1", "session-1");

    assert_eq!(worker.state, WorkerState::Starting);
    assert_eq!(
        supervisor
            .get(&worker.worker_id)
            .expect("worker exists")
            .owner_user_id,
        "user-1"
    );
}

#[test]
fn bootstrap_receipt_records_enterprise_setup() {
    let receipt = BootstrapReceipt::new(
        SetupMode::EnterpriseServer,
        "owner@example.com",
        vec!["/srv/workspaces".into()],
    );

    assert_eq!(receipt.mode, SetupMode::EnterpriseServer);
    assert_eq!(receipt.initial_owner, "owner@example.com");
    assert_eq!(receipt.registered_workspace_roots, vec!["/srv/workspaces"]);
}
