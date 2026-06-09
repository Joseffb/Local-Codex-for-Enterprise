use axum::body::Body;
use axum::body::to_bytes;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
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
use codex_enterprise_server::worker::WorkerRuntimeSupervisor;
use codex_enterprise_server::worker::WorkerState;
use codex_enterprise_server::worker::WorkerSupervisor;
use codex_enterprise_server::workspace::WorkspacePolicy;
use std::path::Path;
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
fn admin_can_administer_but_viewer_cannot() {
    assert!(rbac::role_allows(
        EnterpriseRole::Admin,
        EnterpriseAction::AdministerUsers
    ));
    assert!(rbac::role_allows(
        EnterpriseRole::Admin,
        EnterpriseAction::ReadThreads
    ));
    assert!(rbac::role_allows(
        EnterpriseRole::Admin,
        EnterpriseAction::StartWorker
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
async fn casbin_policy_allows_admin_but_rejects_viewer_admin() {
    assert!(
        rbac::casbin_role_allows(EnterpriseRole::Admin, EnterpriseAction::AdministerUsers)
            .await
            .expect("admin policy")
    );
    assert!(
        rbac::casbin_role_allows(EnterpriseRole::Admin, EnterpriseAction::ReadThreads)
            .await
            .expect("admin read threads policy")
    );
    assert!(
        rbac::casbin_role_allows(EnterpriseRole::Admin, EnterpriseAction::StartWorker)
            .await
            .expect("admin start worker policy")
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
async fn app_server_worker_launch_waits_for_socket_path_before_returning() {
    let temp = tempfile::tempdir().expect("temp dir");
    let socket_dir = temp.path().join("sockets");
    let log_dir = temp.path().join("logs");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let mut config = EnterpriseConfig::default();
    config.worker_command = "/bin/sh".to_string();
    config.worker_args = vec![
        "-c".to_string(),
        "sleep 0.2; touch \"{socket_path}\"; sleep 30".to_string(),
    ];
    config.worker_socket_dir = socket_dir.to_string_lossy().to_string();
    config.worker_log_dir = log_dir.to_string_lossy().to_string();
    let worker_id = Uuid::new_v4().to_string();
    let worker = codex_enterprise_server::worker::WorkerRecord {
        worker_id: worker_id.clone(),
        owner_user_id: Uuid::new_v4().to_string(),
        workspace_id: Uuid::new_v4().to_string(),
        workspace_path: workspace.to_string_lossy().to_string(),
        session_id: Uuid::new_v4().to_string(),
        state: WorkerState::Starting,
        pid: None,
        socket_path: None,
        log_path: None,
        last_heartbeat_at: chrono::Utc::now(),
    };
    let supervisor = WorkerRuntimeSupervisor::default();

    let runtime = supervisor
        .launch(&worker, &config)
        .await
        .expect("worker launch should wait for socket path");

    assert!(Path::new(&runtime.socket_path).exists());
    assert!(supervisor.stop(&worker_id).await.expect("stop worker"));
}

#[tokio::test]
async fn setup_endpoint_bootstraps_once_and_returns_admin_api_token() {
    let temp = tempfile::tempdir().expect("temp dir");
    let socket_dir = temp.path().join("sockets");
    let log_dir = temp.path().join("logs");
    let mut config = EnterpriseConfig::default();
    config.worker_command = "/bin/sh".to_string();
    config.worker_args = vec!["-c".to_string(), "sleep 30".to_string()];
    config.worker_socket_dir = socket_dir.to_string_lossy().to_string();
    config.worker_log_dir = log_dir.to_string_lossy().to_string();
    let router = api::build_router_with_store(InMemoryEnterpriseStore::default(), config);
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
            .starts_with("lce_admin_")
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
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("config body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("config json body");
    assert_eq!(
        body["turn_guidance"]["planning_sequence"][0],
        "business goal"
    );
    assert!(
        body["turn_guidance"]["repository_tool_rule"]
            .as_str()
            .expect("repository rule")
            .contains("Before using repository tools")
    );
    assert!(
        body["turn_guidance"]["tool_output_rule"]
            .as_str()
            .expect("tool output rule")
            .contains("Do not print raw HTML, JSON, or full tool output")
    );
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
async fn setup_rejects_inaccessible_workspace_root_with_runtime_hint() {
    let router = api::build_test_router();

    let response = json_request(
        router,
        "POST",
        "/v1/setup/enterprise",
        None,
        None,
        serde_json::json!({
            "owner_email": "owner@example.com",
            "owner_password": "owner-password",
            "workspace_roots": ["/definitely-not-a-mounted-enterprise-workspace-root"]
        }),
    )
    .await;

    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert!(
        response.json["error"]
            .as_str()
            .expect("error")
            .contains("not accessible from this Enterprise server runtime")
    );
}

#[tokio::test]
async fn setup_assigns_bootstrap_admin_default_user_workspace() {
    let temp = tempfile::tempdir().expect("temp dir");
    let workspace_root = temp.path().join("enterprise-workspaces");
    std::fs::create_dir_all(&workspace_root).expect("workspace root");
    let mut config = EnterpriseConfig::default();
    config.default_workspace_root = Some(workspace_root.to_string_lossy().to_string());
    let router = api::build_router_with_store(InMemoryEnterpriseStore::default(), config);

    let setup = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        None,
        serde_json::json!({
            "owner_email": "admin@example.com",
            "owner_password": "owner-password",
            "workspace_roots": [workspace_root],
        }),
    )
    .await;
    assert_eq!(setup.status, StatusCode::CREATED, "{}", setup.text);
    let admin_token = setup.json["api_token"]
        .as_str()
        .expect("admin token")
        .to_string();

    let user_workspaces = empty_request(
        router,
        "GET",
        "/v1/user-workspaces",
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(
        user_workspaces.status,
        StatusCode::OK,
        "{}",
        user_workspaces.text
    );
    let path = user_workspaces.json["user_workspaces"][0]["path"]
        .as_str()
        .expect("admin user workspace path");
    assert!(
        path.ends_with("/enterprise-workspaces/user/admin@example.com"),
        "{path}"
    );
    assert!(
        std::path::Path::new(path).is_dir(),
        "default admin workspace should be created"
    );
}

#[tokio::test]
async fn user_creation_assigns_default_workspace_under_email_namespace() {
    let temp = tempfile::tempdir().expect("temp dir");
    let workspace_root = temp.path().join("enterprise-workspaces");
    std::fs::create_dir_all(&workspace_root).expect("workspace root");
    let mut config = EnterpriseConfig::default();
    config.default_workspace_root = Some(workspace_root.to_string_lossy().to_string());
    let router = api::build_router_with_store(InMemoryEnterpriseStore::default(), config);

    let setup = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        None,
        serde_json::json!({
            "owner_email": "admin@example.com",
            "owner_password": "owner-password",
            "workspace_roots": [workspace_root],
        }),
    )
    .await;
    assert_eq!(setup.status, StatusCode::CREATED, "{}", setup.text);
    let admin_token = setup.json["api_token"]
        .as_str()
        .expect("admin token")
        .to_string();

    let (_developer_token, developer_id) = create_and_login_user(
        router.clone(),
        &admin_token,
        "dev.user@example.com",
        "developer-password",
        "developer",
    )
    .await;

    let user_workspaces = empty_request(
        router.clone(),
        "GET",
        "/v1/user-workspaces",
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(
        user_workspaces.status,
        StatusCode::OK,
        "{}",
        user_workspaces.text
    );
    let developer_workspace = user_workspaces.json["user_workspaces"]
        .as_array()
        .expect("user workspaces")
        .iter()
        .find(|workspace| workspace["owner_user_id"] == developer_id)
        .expect("developer user workspace");
    let path = developer_workspace["path"]
        .as_str()
        .expect("developer workspace path");
    assert!(
        path.ends_with("/enterprise-workspaces/user/dev.user@example.com"),
        "{path}"
    );
    assert!(
        std::path::Path::new(path).is_dir(),
        "default developer workspace should be created"
    );
}

#[tokio::test]
async fn login_endpoint_issues_token_for_bootstrapped_admin() {
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
    assert_eq!(json["role"], "admin");
    let admin_token = json["api_token"].as_str().expect("api token").to_string();
    let (token, _) = create_and_login_user(
        router.clone(),
        &admin_token,
        "developer@example.com",
        "developer-password",
        "developer",
    )
    .await;
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
    let admin_token = json["api_token"].as_str().expect("api token").to_string();
    let (token, _) = create_and_login_user_with_workspace(
        router.clone(),
        &admin_token,
        "mismatch-dev@example.com",
        "developer-password",
        "developer",
        project.as_path(),
    )
    .await;

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
        .uri("/v1/threads")
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
    let admin_token = json["api_token"].as_str().expect("api token").to_string();
    let (token, developer_user_id) = create_and_login_user_with_workspace(
        router.clone(),
        &admin_token,
        "session-dev@example.com",
        "developer-password",
        "developer",
        project.as_path(),
    )
    .await;

    let create_session = Request::builder()
        .method("POST")
        .uri("/v1/threads")
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
    assert_eq!(created["session"]["owner_user_id"], developer_user_id);
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
        .uri("/v1/threads")
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
        .uri(format!("/v1/threads/{session_id}"))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("request");
    let response = router
        .clone()
        .oneshot(get_session)
        .await
        .expect("get session response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("get session body");
    let fetched: serde_json::Value = serde_json::from_slice(&body).expect("get session json");
    assert_eq!(fetched["session"]["session_id"], session_id);

    let rename_thread = json_request(
        router.clone(),
        "PATCH",
        &format!("/v1/threads/{session_id}"),
        Some(&token),
        None,
        serde_json::json!({
            "title": "Updated governance sprint"
        }),
    )
    .await;
    assert_eq!(
        rename_thread.status,
        StatusCode::OK,
        "{}",
        rename_thread.text
    );
    assert_eq!(
        rename_thread.json["session"]["title"],
        "Updated governance sprint"
    );

    let renamed_session = empty_request(
        router.clone(),
        "GET",
        &format!("/v1/threads/{session_id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(renamed_session.status, StatusCode::OK);
    assert_eq!(
        renamed_session.json["session"]["title"],
        "Updated governance sprint"
    );

    let renamed_sessions =
        empty_request(router.clone(), "GET", "/v1/threads", Some(&token), None).await;
    assert_eq!(renamed_sessions.status, StatusCode::OK);
    assert_eq!(
        renamed_sessions.json["sessions"][0]["title"],
        "Updated governance sprint"
    );

    let first_message = json_request(
        router.clone(),
        "POST",
        &format!("/v1/threads/{session_id}/messages"),
        Some(&token),
        None,
        serde_json::json!({
            "kind": "user",
            "label": "You",
            "text": "Inspect the workspace"
        }),
    )
    .await;
    assert_eq!(
        first_message.status,
        StatusCode::CREATED,
        "{}",
        first_message.text
    );
    assert_eq!(first_message.json["message"]["sequence"], 1);

    let second_message = json_request(
        router.clone(),
        "POST",
        &format!("/v1/threads/{session_id}/messages"),
        Some(&token),
        None,
        serde_json::json!({
            "kind": "assistant",
            "label": "Codex",
            "text": "I found the repository layout."
        }),
    )
    .await;
    assert_eq!(
        second_message.status,
        StatusCode::CREATED,
        "{}",
        second_message.text
    );
    assert_eq!(second_message.json["message"]["sequence"], 2);

    let messages = empty_request(
        router.clone(),
        "GET",
        &format!("/v1/threads/{session_id}/messages"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(messages.status, StatusCode::OK);
    let messages = messages.json["messages"].as_array().expect("messages");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["kind"], "user");
    assert_eq!(messages[0]["text"], "Inspect the workspace");
    assert_eq!(messages[1]["kind"], "assistant");
    assert_eq!(messages[1]["text"], "I found the repository layout.");

    let audit = empty_request(
        router.clone(),
        "GET",
        "/v1/evidence-records",
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(audit.status, StatusCode::OK);
    assert!(!audit.text.contains("Inspect the workspace"));
    assert!(!audit.text.contains("I found the repository layout."));

    let deleted_thread = empty_request(
        router.clone(),
        "DELETE",
        &format!("/v1/threads/{session_id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(
        deleted_thread.status,
        StatusCode::OK,
        "{}",
        deleted_thread.text
    );
    assert!(
        deleted_thread.json["session"]["deleted_at"]
            .as_str()
            .expect("deleted_at")
            .contains('T')
    );

    let missing_deleted = empty_request(
        router.clone(),
        "GET",
        &format!("/v1/threads/{session_id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(missing_deleted.status, StatusCode::NOT_FOUND);

    let active_after_delete = empty_request(router, "GET", "/v1/threads", Some(&token), None).await;
    assert_eq!(active_after_delete.status, StatusCode::OK);
    assert_eq!(
        active_after_delete.json["sessions"]
            .as_array()
            .expect("sessions")
            .len(),
        0
    );
}

#[tokio::test]
async fn assistant_response_feedback_updates_user_preferences_without_leaking_text() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let router = api::build_router_with_store(
        InMemoryEnterpriseStore::default(),
        EnterpriseConfig::default(),
    );
    let workspace_root = temp_dir.path().join("workspace-root");
    std::fs::create_dir_all(&workspace_root).expect("workspace root");
    let bootstrap = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        None,
        serde_json::json!({
            "owner_email": "owner@example.com",
            "owner_password": "owner-password",
            "workspace_roots": [workspace_root],
        }),
    )
    .await;
    assert_eq!(bootstrap.status, StatusCode::CREATED);
    let token = bootstrap.json["api_token"].as_str().expect("token");

    let thread = json_request(
        router.clone(),
        "POST",
        "/v1/threads",
        Some(token),
        None,
        serde_json::json!({
            "workspace_path": workspace_root,
            "title": "Feedback thread"
        }),
    )
    .await;
    assert_eq!(thread.status, StatusCode::CREATED, "{}", thread.text);
    let thread_id = thread.json["session"]["session_id"]
        .as_str()
        .expect("thread id")
        .to_string();

    let user_message = json_request(
        router.clone(),
        "POST",
        &format!("/v1/threads/{thread_id}/messages"),
        Some(token),
        None,
        serde_json::json!({
            "kind": "user",
            "label": "You",
            "text": "Please plan the portal"
        }),
    )
    .await;
    assert_eq!(user_message.status, StatusCode::CREATED);
    let user_message_id = user_message.json["message"]["message_id"]
        .as_str()
        .expect("user message id")
        .to_string();

    let assistant_message = json_request(
        router.clone(),
        "POST",
        &format!("/v1/threads/{thread_id}/messages"),
        Some(token),
        None,
        serde_json::json!({
            "kind": "assistant",
            "label": "Codex",
            "text": "RAW ASSISTANT TEXT THAT MUST NOT BE COPIED INTO PREFERENCES"
        }),
    )
    .await;
    assert_eq!(assistant_message.status, StatusCode::CREATED);
    let assistant_message_id = assistant_message.json["message"]["message_id"]
        .as_str()
        .expect("assistant message id")
        .to_string();

    let cannot_feedback_user_message = json_request(
        router.clone(),
        "PUT",
        &format!("/v1/threads/{thread_id}/messages/{user_message_id}/feedback"),
        Some(token),
        None,
        serde_json::json!({
            "rating": "bad",
            "reason_tags": ["too_verbose"]
        }),
    )
    .await;
    assert_eq!(
        cannot_feedback_user_message.status,
        StatusCode::BAD_REQUEST,
        "{}",
        cannot_feedback_user_message.text
    );

    let feedback = json_request(
        router.clone(),
        "PUT",
        &format!("/v1/threads/{thread_id}/messages/{assistant_message_id}/feedback"),
        Some(token),
        None,
        serde_json::json!({
            "rating": "bad",
            "reason_tags": ["too_verbose", "used_repo_when_not_needed", "raw_tool_output"],
            "comment": "do not inject this private freeform comment"
        }),
    )
    .await;
    assert_eq!(feedback.status, StatusCode::OK, "{}", feedback.text);
    assert_eq!(feedback.json["feedback"]["rating"], "bad");
    assert_eq!(feedback.json["preferences"]["sample_count"], 1);
    let summary = feedback.json["preferences"]["profile_summary"]
        .as_str()
        .expect("profile summary");
    assert!(summary.contains("Prefer concise answers."));
    assert!(summary.contains("Do not inspect repositories unless explicitly requested"));
    assert!(summary.contains("Collapse or summarize tool output"));
    assert!(!feedback.text.contains("RAW ASSISTANT TEXT"));
    assert!(!feedback.text.contains("private freeform comment"));

    let updated_feedback = json_request(
        router.clone(),
        "PUT",
        &format!("/v1/threads/{thread_id}/messages/{assistant_message_id}/feedback"),
        Some(token),
        None,
        serde_json::json!({
            "rating": "good",
            "reason_tags": ["poor_formatting"]
        }),
    )
    .await;
    assert_eq!(updated_feedback.status, StatusCode::OK);
    assert_eq!(updated_feedback.json["feedback"]["rating"], "good");
    assert_eq!(updated_feedback.json["preferences"]["sample_count"], 1);

    let messages_with_feedback = empty_request(
        router.clone(),
        "GET",
        &format!("/v1/threads/{thread_id}/messages"),
        Some(token),
        None,
    )
    .await;
    assert_eq!(messages_with_feedback.status, StatusCode::OK);
    let assistant_after_feedback = messages_with_feedback.json["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .find(|message| message["message_id"] == assistant_message_id)
        .expect("assistant message");
    assert_eq!(assistant_after_feedback["feedback_rating"], "good");

    let preferences = empty_request(
        router.clone(),
        "GET",
        "/v1/me/response-preferences",
        Some(token),
        None,
    )
    .await;
    assert_eq!(preferences.status, StatusCode::OK);
    assert_eq!(preferences.json["preferences"]["sample_count"], 1);

    let reset = empty_request(
        router.clone(),
        "DELETE",
        "/v1/me/response-preferences",
        Some(token),
        None,
    )
    .await;
    assert_eq!(reset.status, StatusCode::NO_CONTENT);

    let preferences_after_reset = empty_request(
        router,
        "GET",
        "/v1/me/response-preferences",
        Some(token),
        None,
    )
    .await;
    assert_eq!(preferences_after_reset.status, StatusCode::OK);
    assert_eq!(
        preferences_after_reset.json["preferences"]["profile_summary"],
        ""
    );
    assert_eq!(
        preferences_after_reset.json["preferences"]["sample_count"],
        0
    );
}

#[tokio::test]
async fn response_feedback_is_user_scoped_and_chat_ui_loads_preferences() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let router = api::build_router_with_store(
        InMemoryEnterpriseStore::default(),
        EnterpriseConfig::default(),
    );
    let workspace_root = temp_dir.path().join("workspace-root");
    std::fs::create_dir_all(&workspace_root).expect("workspace root");
    let bootstrap = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        None,
        serde_json::json!({
            "owner_email": "owner@example.com",
            "owner_password": "owner-password",
            "workspace_roots": [workspace_root],
        }),
    )
    .await;
    assert_eq!(bootstrap.status, StatusCode::CREATED);
    let admin_token = bootstrap.json["api_token"].as_str().expect("admin token");

    let user = json_request(
        router.clone(),
        "POST",
        "/v1/users",
        Some(admin_token),
        None,
        serde_json::json!({
            "email": "developer@example.com",
            "password": "developer-password",
            "role": "developer",
        }),
    )
    .await;
    assert_eq!(user.status, StatusCode::CREATED);
    let developer_login = json_request(
        router.clone(),
        "POST",
        "/v1/auth/login",
        None,
        None,
        serde_json::json!({
            "email": "developer@example.com",
            "password": "developer-password",
        }),
    )
    .await;
    assert_eq!(developer_login.status, StatusCode::OK);
    let developer_token = developer_login.json["api_token"]
        .as_str()
        .expect("developer token");

    let thread = json_request(
        router.clone(),
        "POST",
        "/v1/threads",
        Some(admin_token),
        None,
        serde_json::json!({
            "workspace_path": workspace_root,
            "title": "Admin feedback thread"
        }),
    )
    .await;
    assert_eq!(thread.status, StatusCode::CREATED);
    let thread_id = thread.json["session"]["session_id"]
        .as_str()
        .expect("thread id")
        .to_string();
    let assistant_message = json_request(
        router.clone(),
        "POST",
        &format!("/v1/threads/{thread_id}/messages"),
        Some(admin_token),
        None,
        serde_json::json!({
            "kind": "assistant",
            "label": "Codex",
            "text": "Admin-only answer"
        }),
    )
    .await;
    assert_eq!(assistant_message.status, StatusCode::CREATED);
    let assistant_message_id = assistant_message.json["message"]["message_id"]
        .as_str()
        .expect("message id")
        .to_string();

    let cross_user_feedback = json_request(
        router.clone(),
        "PUT",
        &format!("/v1/threads/{thread_id}/messages/{assistant_message_id}/feedback"),
        Some(developer_token),
        None,
        serde_json::json!({
            "rating": "bad",
            "reason_tags": ["wrong_context"]
        }),
    )
    .await;
    assert_eq!(cross_user_feedback.status, StatusCode::NOT_FOUND);

    let chat = empty_request(router, "GET", "/chat", Some(admin_token), None).await;
    assert_eq!(chat.status, StatusCode::OK);
    assert!(chat.text.contains("/v1/me/response-preferences"));
    assert!(chat.text.contains("/feedback"));
    assert!(chat.text.contains("data-feedback-rating=\"good\""));
    assert!(chat.text.contains("data-feedback-rating=\"bad\""));
    assert!(chat.text.contains("feedback-selected"));
    assert!(chat.text.contains("response-preferences-modal"));
    assert!(chat.text.contains("Reset preferences"));
    assert!(chat.text.contains("User response preferences"));
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
    let admin_token = json["api_token"].as_str().expect("api token").to_string();
    let (token, _) = create_and_login_user_with_workspace(
        router.clone(),
        &admin_token,
        "escape-dev@example.com",
        "developer-password",
        "developer",
        workspace_root.as_path(),
    )
    .await;

    let create_session = Request::builder()
        .method("POST")
        .uri("/v1/threads")
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
async fn viewer_token_can_read_threads_and_use_chat_but_cannot_administer() {
    let temp = tempfile::tempdir().expect("temp dir");
    let socket_dir = temp.path().join("sockets");
    let log_dir = temp.path().join("logs");
    let mut config = EnterpriseConfig::default();
    config.worker_command = "/bin/sh".to_string();
    config.worker_args = vec!["-c".to_string(), "sleep 30".to_string()];
    config.worker_socket_dir = socket_dir.to_string_lossy().to_string();
    config.worker_log_dir = log_dir.to_string_lossy().to_string();
    let router = api::build_router_with_store(InMemoryEnterpriseStore::default(), config);
    let workspace_root = temp.path().join("workspaces");
    let project = workspace_root.join("project");
    std::fs::create_dir_all(&project).expect("project workspace");

    let setup = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        None,
        serde_json::json!({
            "owner_email": "admin@example.com",
            "owner_password": "admin-password",
            "workspace_roots": [workspace_root],
        }),
    )
    .await;
    assert_eq!(setup.status, StatusCode::CREATED);
    let admin_token = setup.json["api_token"]
        .as_str()
        .expect("admin token")
        .to_string();
    let (viewer_token, _) = create_and_login_user_with_workspace(
        router.clone(),
        &admin_token,
        "viewer@example.com",
        "viewer-password",
        "viewer",
        project.as_path(),
    )
    .await;

    let list_workers = Request::builder()
        .method("GET")
        .uri("/v1/workers")
        .header("authorization", format!("Bearer {viewer_token}"))
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
        .header("authorization", format!("Bearer {viewer_token}"))
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
        .expect("start response");
    let response = json_response(response).await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text);

    let forbidden_admin = json_request(
        router,
        "POST",
        "/v1/users",
        Some(&viewer_token),
        None,
        serde_json::json!({
            "email": "not-allowed@example.com",
            "password": "password",
            "role": "viewer"
        }),
    )
    .await;
    assert_eq!(forbidden_admin.status, StatusCode::FORBIDDEN);
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

    let forbidden_admin = json_request(
        router,
        "POST",
        "/v1/users",
        Some(&viewer_token.plaintext),
        None,
        serde_json::json!({
            "email": "blocked@example.com",
            "password": "password",
            "role": "viewer"
        }),
    )
    .await;
    assert_eq!(forbidden_admin.status, StatusCode::FORBIDDEN);

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
    let admin_token = json["api_token"].as_str().expect("api token").to_string();
    let (token, developer_user_id) = create_and_login_user_with_workspace(
        router.clone(),
        &admin_token,
        "handoff-dev@example.com",
        "developer-password",
        "developer",
        project.as_path(),
    )
    .await;

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
        .uri(format!("/v1/workers/{worker_id}/handoffs"))
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
    assert_eq!(handoff["owner_user_id"], developer_user_id);
    assert!(
        handoff["socket_path"]
            .as_str()
            .expect("socket path")
            .starts_with(socket_dir.to_str().expect("socket dir"))
    );

    let claims = auth::decode_worker_handoff_token(&handoff_token, "test-handoff-secret")
        .expect("decode handoff token");
    assert_eq!(claims.sub, developer_user_id);
    assert_eq!(claims.worker_id, worker_id);
    assert_eq!(claims.session_id, "session-1");
    assert_eq!(claims.jti, jti);

    let consume_handoff = Request::builder()
        .method("POST")
        .uri(format!("/v1/worker-handoffs/{jti}/consumptions"))
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
        .uri(format!("/v1/worker-handoffs/{jti}/consumptions"))
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
    let admin_token = json["api_token"].as_str().expect("api token").to_string();
    let (token, developer_user_id) = create_and_login_user_with_workspace(
        router.clone(),
        &admin_token,
        "trace-dev@example.com",
        "developer-password",
        "developer",
        project.as_path(),
    )
    .await;

    let create_session = Request::builder()
        .method("POST")
        .uri("/v1/threads")
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
        .uri(format!("/v1/workers/{worker_id}/handoffs"))
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
            .all(|event| event.actor_user_id.as_deref() == Some(developer_user_id.as_str()))
    );

    let receipts = store.execution_receipts_for_test().await;
    assert!(receipts.iter().any(|receipt| {
        receipt.trace_id == trace_id
            && receipt.actor_user_id.as_deref() == Some(developer_user_id.as_str())
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
        .uri("/v1/projects/PROJECT_ID/repositories/clone")
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
        .uri("/v1/projects/PROJECT_ID/repositories/clone")
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
            .any(|event| event.event_type == "repository.clone"
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
async fn project_repository_clone_rejects_unsafe_repo_before_git_runs() {
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
        .uri("/v1/projects/PROJECT_ID/repositories/clone")
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

#[tokio::test]
async fn vertical_enterprise_control_plane_mvp_drives_developer_journey_with_receipts() {
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
    config.handoff_token_secret = "vertical-test-handoff-secret".to_string();

    let store = InMemoryEnterpriseStore::default();
    let router = api::build_router_with_store(store.clone(), config);
    let trace_id = Uuid::new_v4().to_string();

    let setup_response = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        Some(&trace_id),
        serde_json::json!({
            "owner_email": "owner@example.com",
            "owner_password": "correct horse battery staple",
            "workspace_roots": [workspace_root]
        }),
    )
    .await;
    assert_eq!(setup_response.status, StatusCode::CREATED);
    let owner_token = setup_response.json["api_token"]
        .as_str()
        .expect("owner token")
        .to_string();

    let created_user = json_request(
        router.clone(),
        "POST",
        "/v1/users",
        Some(&owner_token),
        Some(&trace_id),
        serde_json::json!({
            "email": "developer@example.com",
            "password": "developer password",
            "role": "viewer"
        }),
    )
    .await;
    assert_eq!(created_user.status, StatusCode::CREATED);
    let developer_user_id = created_user.json["user"]["user_id"]
        .as_str()
        .expect("developer user id")
        .to_string();
    assert_eq!(created_user.json["user"]["status"], "active");
    assert_eq!(created_user.json["user"]["role"], "viewer");

    let assigned_role = json_request(
        router.clone(),
        "PUT",
        &format!("/v1/users/{developer_user_id}/role"),
        Some(&owner_token),
        Some(&trace_id),
        serde_json::json!({ "role": "developer" }),
    )
    .await;
    assert_eq!(assigned_role.status, StatusCode::OK);
    assert_eq!(assigned_role.json["user"]["role"], "developer");

    let registered_workspace = json_request(
        router.clone(),
        "POST",
        "/v1/workspace-roots",
        Some(&owner_token),
        Some(&trace_id),
        serde_json::json!({ "root_path": project }),
    )
    .await;
    assert_eq!(registered_workspace.status, StatusCode::CREATED);
    let workspace_id = registered_workspace.json["workspace"]["workspace_id"]
        .as_str()
        .expect("workspace id")
        .to_string();

    let workspace_access = json_request(
        router.clone(),
        "POST",
        "/v1/user-workspace-access-grants",
        Some(&owner_token),
        Some(&trace_id),
        serde_json::json!({
            "user_id": developer_user_id,
            "workspace_root": project
        }),
    )
    .await;
    assert_eq!(
        workspace_access.status,
        StatusCode::CREATED,
        "{}",
        workspace_access.text
    );

    let pack = json_request(
        router.clone(),
        "POST",
        "/v1/context-packs",
        Some(&owner_token),
        Some(&trace_id),
        serde_json::json!({
            "name": "Standard Engineering Pack",
            "documents": [
                {
                    "filename": "PACK.md",
                    "content": "name: Standard Engineering Pack\nversion: 1\nrequired_files:\n- OPERATING-INSTRUCTIONS.md\nload_order:\n- OPERATING-INSTRUCTIONS.md\n"
                },
                {
                    "filename": "OPERATING-INSTRUCTIONS.md",
                    "content": "Use the project test suite before reporting completion."
                }
            ]
        }),
    )
    .await;
    assert_eq!(pack.status, StatusCode::CREATED);
    let pack_id = pack.json["pack"]["pack_id"]
        .as_str()
        .expect("pack id")
        .to_string();
    let document_id = pack.json["documents"]
        .as_array()
        .expect("documents")
        .iter()
        .find(|document| document["filename"] == "OPERATING-INSTRUCTIONS.md")
        .expect("operating instructions document")["document_id"]
        .as_str()
        .expect("document id")
        .to_string();

    let assignment = json_request(
        router.clone(),
        "POST",
        "/v1/context-pack-assignments",
        Some(&owner_token),
        Some(&trace_id),
        serde_json::json!({
            "pack_id": pack_id,
            "user_id": developer_user_id,
            "workspace_id": workspace_id,
            "assignment_order": 10,
            "required_session": true,
            "required_worker": true
        }),
    )
    .await;
    assert_eq!(assignment.status, StatusCode::CREATED);
    assert_eq!(
        assignment.json["assignment"]["assignment_source"],
        "user_workspace"
    );

    let login = json_request(
        router.clone(),
        "POST",
        "/v1/auth/login",
        None,
        Some(&trace_id),
        serde_json::json!({
            "email": "developer@example.com",
            "password": "developer password"
        }),
    )
    .await;
    assert_eq!(login.status, StatusCode::OK);
    let developer_token = login.json["api_token"]
        .as_str()
        .expect("developer token")
        .to_string();
    assert_eq!(login.json["role"], "developer");

    let session = json_request(
        router.clone(),
        "POST",
        "/v1/threads",
        Some(&developer_token),
        Some(&trace_id),
        serde_json::json!({
            "workspace_path": project,
            "title": "Vertical MVP"
        }),
    )
    .await;
    assert_eq!(session.status, StatusCode::CREATED);
    let session_id = session.json["session"]["session_id"]
        .as_str()
        .expect("session id")
        .to_string();

    let worker = json_request(
        router.clone(),
        "POST",
        "/v1/workers",
        Some(&developer_token),
        Some(&trace_id),
        serde_json::json!({
            "workspace_path": project,
            "session_id": session_id
        }),
    )
    .await;
    assert_eq!(worker.status, StatusCode::CREATED);
    let worker_id = worker.json["worker"]["worker_id"]
        .as_str()
        .expect("worker id")
        .to_string();

    let handoff = empty_request(
        router.clone(),
        "POST",
        &format!("/v1/workers/{worker_id}/handoffs"),
        Some(&developer_token),
        Some(&trace_id),
    )
    .await;
    assert_eq!(handoff.status, StatusCode::CREATED);
    let handoff_token = handoff.json["handoff_token"]
        .as_str()
        .expect("handoff token")
        .to_string();

    let audit = empty_request(
        router.clone(),
        "GET",
        &format!("/v1/evidence-records?trace_id={trace_id}"),
        Some(&owner_token),
        Some(&trace_id),
    )
    .await;
    assert_eq!(audit.status, StatusCode::OK);
    let receipt_events = audit.json["execution_receipts"]
        .as_array()
        .expect("execution receipts")
        .iter()
        .map(|receipt| receipt["event_type"].as_str().expect("event type"))
        .collect::<Vec<_>>();
    assert!(receipt_events.contains(&"session.create"));
    assert!(receipt_events.contains(&"context_pack.session_load"));
    assert!(receipt_events.contains(&"context_pack.worker_load"));
    assert!(receipt_events.contains(&"worker.start"));
    assert!(receipt_events.contains(&"worker.handoff.issue"));

    let context_receipts = audit.json["context_receipts"]
        .as_array()
        .expect("context receipts");
    assert!(context_receipts.iter().any(|receipt| {
        receipt["trace_id"] == trace_id
            && receipt["pack_id"] == pack_id
            && receipt["document_id"] == document_id
            && receipt["load_order"] == 10
            && receipt["assignment_source"] == "user_workspace"
            && receipt["session_id"] == session_id
            && receipt["worker_id"] == worker_id
    }));

    let durable_records = serde_json::to_string(&audit.json).expect("audit json");
    assert!(!durable_records.contains("developer password"));
    assert!(!durable_records.contains(&developer_token));
    assert!(!durable_records.contains(&owner_token));
    assert!(!durable_records.contains(&handoff_token));
    assert!(!durable_records.contains("Use the project test suite"));
}

#[tokio::test]
async fn context_pack_assignments_reject_ambiguous_load_order_for_same_user_workspace() {
    let temp = tempfile::tempdir().expect("temp dir");
    let workspace_root = temp.path().join("workspaces");
    std::fs::create_dir_all(&workspace_root).expect("workspace root");
    let router = api::build_test_router();
    let trace_id = Uuid::new_v4().to_string();

    let setup = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        Some(&trace_id),
        serde_json::json!({
            "owner_email": "owner@example.com",
            "owner_password": "correct horse battery staple",
            "workspace_roots": [workspace_root]
        }),
    )
    .await;
    let token = setup.json["api_token"].as_str().expect("token").to_string();
    let user_id = setup.json["owner_user_id"]
        .as_str()
        .expect("owner user id")
        .to_string();

    let first = create_minimal_context_pack(router.clone(), &token, &trace_id, "First Pack").await;
    let second =
        create_minimal_context_pack(router.clone(), &token, &trace_id, "Second Pack").await;

    let assign_first = json_request(
        router.clone(),
        "POST",
        "/v1/context-pack-assignments",
        Some(&token),
        Some(&trace_id),
        serde_json::json!({
            "pack_id": first,
            "user_id": user_id,
            "workspace_id": workspace_root.canonicalize().expect("canonical root"),
            "assignment_order": 10,
            "required_session": true,
            "required_worker": true
        }),
    )
    .await;
    assert_eq!(assign_first.status, StatusCode::CREATED);

    let assign_second = json_request(
        router,
        "POST",
        "/v1/context-pack-assignments",
        Some(&token),
        Some(&trace_id),
        serde_json::json!({
            "pack_id": second,
            "user_id": user_id,
            "workspace_id": workspace_root.canonicalize().expect("canonical root"),
            "assignment_order": 10,
            "required_session": true,
            "required_worker": true
        }),
    )
    .await;
    assert_eq!(assign_second.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn context_pack_assignment_admin_flow_lists_bulk_assigns_and_removes_without_manual_ids() {
    let temp = tempfile::tempdir().expect("temp dir");
    let workspace_root = temp.path().join("workspaces");
    let project = workspace_root.join("project");
    std::fs::create_dir_all(&project).expect("project workspace");
    let router = api::build_test_router();
    let trace_id = Uuid::new_v4().to_string();

    let setup = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        Some(&trace_id),
        serde_json::json!({
            "owner_email": "owner@example.com",
            "owner_password": "correct horse battery staple",
            "workspace_roots": [workspace_root]
        }),
    )
    .await;
    let token = setup.json["api_token"].as_str().expect("token").to_string();

    let user = json_request(
        router.clone(),
        "POST",
        "/v1/users",
        Some(&token),
        Some(&trace_id),
        serde_json::json!({
            "email": "developer@example.com",
            "password": "developer password",
            "role": "developer"
        }),
    )
    .await;
    assert_eq!(user.status, StatusCode::CREATED);
    let user_id = user.json["user"]["user_id"]
        .as_str()
        .expect("user id")
        .to_string();

    let pack =
        create_minimal_context_pack(router.clone(), &token, &trace_id, "Assignable Pack").await;

    let users = empty_request(
        router.clone(),
        "GET",
        "/v1/users",
        Some(&token),
        Some(&trace_id),
    )
    .await;
    assert_eq!(users.status, StatusCode::OK);
    assert!(users.text.contains("developer@example.com"));

    let packs = empty_request(
        router.clone(),
        "GET",
        "/v1/context-packs",
        Some(&token),
        Some(&trace_id),
    )
    .await;
    assert_eq!(packs.status, StatusCode::OK);
    assert!(packs.text.contains("Assignable Pack"));

    let bulk = json_request(
        router.clone(),
        "POST",
        "/v1/context-pack-assignment-batches",
        Some(&token),
        Some(&trace_id),
        serde_json::json!({
            "pack_ids": [pack],
            "user_ids": [user_id],
            "workspace_ids": [project],
            "assignment_order": 10,
            "required_session": true,
            "required_worker": true
        }),
    )
    .await;
    assert_eq!(bulk.status, StatusCode::CREATED);
    let assignment_id = bulk.json["assignments"][0]["assignment_id"]
        .as_str()
        .expect("assignment id")
        .to_string();

    let assignments = empty_request(
        router.clone(),
        "GET",
        "/v1/context-pack-assignments",
        Some(&token),
        Some(&trace_id),
    )
    .await;
    assert_eq!(assignments.status, StatusCode::OK);
    assert!(assignments.text.contains(&assignment_id));

    let removed = empty_request(
        router.clone(),
        "DELETE",
        &format!("/v1/context-pack-assignments/{assignment_id}"),
        Some(&token),
        Some(&trace_id),
    )
    .await;
    assert_eq!(removed.status, StatusCode::OK);

    let assignments = empty_request(
        router,
        "GET",
        "/v1/context-pack-assignments",
        Some(&token),
        Some(&trace_id),
    )
    .await;
    assert_eq!(assignments.status, StatusCode::OK);
    assert!(!assignments.text.contains(&assignment_id));
}

#[tokio::test]
async fn demo_data_is_opt_in_idempotent_and_keeps_passwords_out_of_audit() {
    let temp = tempfile::tempdir().expect("temp dir");
    let workspace_root = temp.path().join("workspaces");
    std::fs::create_dir_all(&workspace_root).expect("workspace root");
    let router = api::build_test_router();
    let trace_id = Uuid::new_v4().to_string();

    let setup = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        Some(&trace_id),
        serde_json::json!({
            "owner_email": "owner@example.com",
            "owner_password": "owner-password",
            "workspace_roots": [workspace_root]
        }),
    )
    .await;
    assert_eq!(setup.status, StatusCode::CREATED);
    let token = setup.json["api_token"].as_str().expect("token").to_string();

    let admin_page = empty_request(router.clone(), "GET", "/admin", Some(&token), None).await;
    assert_eq!(admin_page.status, StatusCode::OK);
    assert!(admin_page.text.contains("Demo Data"));
    assert!(admin_page.text.contains("Load Demo Data"));
    assert!(admin_page.text.contains("/v1/demo-data"));

    let status = empty_request(
        router.clone(),
        "GET",
        "/v1/demo-data",
        Some(&token),
        Some(&trace_id),
    )
    .await;
    assert_eq!(status.status, StatusCode::OK);
    assert_eq!(status.json["installed"], false);

    let seeded = json_request(
        router.clone(),
        "POST",
        "/v1/demo-data",
        Some(&token),
        Some(&trace_id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(seeded.status, StatusCode::CREATED);
    assert_eq!(seeded.json["installed"], true);
    assert!(seeded.text.contains("demo.developer@example.test"));
    assert!(seeded.text.contains("Demo Engineering Context Pack"));
    for filename in [
        "PACK.md",
        "CALIBRATION.md",
        "OPERATING-INSTRUCTIONS.md",
        "PROJECT-RULES.md",
        "HANDOFF.md",
        "VERIFICATION.md",
        "ESCALATION.md",
        "CONTEXT.md",
    ] {
        assert!(seeded.text.contains(filename), "{filename}");
    }
    let developer_password = seeded.json["users"][0]["password"]
        .as_str()
        .expect("one-time demo password")
        .to_string();

    let users = empty_request(
        router.clone(),
        "GET",
        "/v1/users",
        Some(&token),
        Some(&trace_id),
    )
    .await;
    assert_eq!(users.status, StatusCode::OK);
    assert!(users.text.contains("demo.developer@example.test"));
    assert!(users.text.contains("demo.viewer@example.test"));

    let packs = empty_request(
        router.clone(),
        "GET",
        "/v1/context-packs",
        Some(&token),
        Some(&trace_id),
    )
    .await;
    assert_eq!(packs.status, StatusCode::OK);
    assert!(packs.text.contains("Demo Engineering Context Pack"));

    let assignments = empty_request(
        router.clone(),
        "GET",
        "/v1/context-pack-assignments",
        Some(&token),
        Some(&trace_id),
    )
    .await;
    assert_eq!(assignments.status, StatusCode::OK);
    assert!(assignments.text.contains("user_workspace"));

    let seeded_again = json_request(
        router.clone(),
        "POST",
        "/v1/demo-data",
        Some(&token),
        Some(&trace_id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(seeded_again.status, StatusCode::OK);
    assert_eq!(seeded_again.json["installed"], true);
    assert!(
        seeded_again.json["users"]
            .as_array()
            .expect("users")
            .iter()
            .all(|user| user["password"].is_null())
    );

    let audit = empty_request(
        router,
        "GET",
        &format!("/v1/evidence-records?trace_id={trace_id}"),
        Some(&token),
        Some(&trace_id),
    )
    .await;
    assert_eq!(audit.status, StatusCode::OK);
    assert!(audit.text.contains("demo_data.seed"));
    assert!(!audit.text.contains(&developer_password));
}

#[tokio::test]
async fn admin_assigns_user_scoped_outputs_that_users_can_view_and_download_read_only() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let output_root = temp_dir.path().join("output-artifacts");
    std::fs::create_dir_all(&output_root).expect("output root");
    let mut config = EnterpriseConfig::default();
    config.output_artifact_root = output_root.to_string_lossy().to_string();
    let router = api::build_router_with_store(InMemoryEnterpriseStore::default(), config);
    let workspace_root = temp_dir.path().join("workspaces");
    std::fs::create_dir_all(&workspace_root).expect("workspace root");
    let bootstrap = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        None,
        serde_json::json!({
            "owner_email": "owner@example.com",
            "owner_password": "owner-password",
            "workspace_roots": [workspace_root],
        }),
    )
    .await;
    assert_eq!(bootstrap.status, StatusCode::CREATED);
    let owner_token = bootstrap.json["api_token"]
        .as_str()
        .expect("token")
        .to_string();

    let user = json_request(
        router.clone(),
        "POST",
        "/v1/users",
        Some(&owner_token),
        None,
        serde_json::json!({
            "email": "developer@example.com",
            "password": "developer-password",
            "role": "developer",
        }),
    )
    .await;
    assert_eq!(user.status, StatusCode::CREATED);

    let developer_login = json_request(
        router.clone(),
        "POST",
        "/v1/auth/login",
        None,
        None,
        serde_json::json!({
            "email": "developer@example.com",
            "password": "developer-password",
        }),
    )
    .await;
    assert_eq!(developer_login.status, StatusCode::OK);
    let developer_token = developer_login.json["api_token"]
        .as_str()
        .expect("developer token")
        .to_string();
    let developer_id = developer_login.json["user_id"]
        .as_str()
        .expect("developer id")
        .to_string();

    let manager = json_request(
        router.clone(),
        "POST",
        "/v1/users",
        Some(&owner_token),
        None,
        serde_json::json!({
            "email": "manager@example.com",
            "password": "manager-password",
            "role": "manager",
        }),
    )
    .await;
    assert_eq!(manager.status, StatusCode::CREATED);
    let manager_login = json_request(
        router.clone(),
        "POST",
        "/v1/auth/login",
        None,
        None,
        serde_json::json!({
            "email": "manager@example.com",
            "password": "manager-password",
        }),
    )
    .await;
    assert_eq!(manager_login.status, StatusCode::OK);
    let manager_token = manager_login.json["api_token"]
        .as_str()
        .expect("manager token")
        .to_string();

    let user_artifact_dir = output_root.join(&developer_id).join("outputs");
    std::fs::create_dir_all(&user_artifact_dir).expect("user output dir");
    std::fs::write(
        user_artifact_dir.join("vp-report.md"),
        "# Delivery Report\n\nSynthetic report for local review.\n",
    )
    .expect("write report");

    let report = json_request(
        router.clone(),
        "POST",
        "/v1/outputs",
        Some(&manager_token),
        None,
        serde_json::json!({
            "owner_user_id": developer_id,
            "category": "deliverable",
            "output_type": "end_report",
            "title": "VP Delivery Report",
            "artifact_path": "outputs/vp-report.md",
            "status": "completed",
            "metadata_json": {"source": "admin"}
        }),
    )
    .await;
    assert_eq!(report.status, StatusCode::CREATED);
    assert_eq!(report.json["output"]["owner_user_id"], developer_id);
    assert_eq!(report.json["output"]["category"], "deliverable");
    assert_eq!(report.json["output"]["output_type"], "end_report");
    let output_id = report.json["output"]["output_id"]
        .as_str()
        .expect("output id")
        .to_string();

    let user_cannot_create = json_request(
        router.clone(),
        "POST",
        "/v1/outputs",
        Some(&developer_token),
        None,
        serde_json::json!({
            "owner_user_id": developer_id,
            "category": "deliverable",
            "output_type": "end_report",
            "title": "Unauthorized Report",
            "artifact_path": "outputs/vp-report.md",
            "status": "completed",
            "metadata_json": {"source": "developer"}
        }),
    )
    .await;
    assert_eq!(user_cannot_create.status, StatusCode::FORBIDDEN);

    let outputs = empty_request(
        router.clone(),
        "GET",
        "/v1/outputs",
        Some(&developer_token),
        None,
    )
    .await;
    assert_eq!(outputs.status, StatusCode::OK);
    assert_eq!(
        outputs.json["outputs"].as_array().expect("outputs").len(),
        1
    );
    assert!(outputs.text.contains("VP Delivery Report"));
    assert!(!outputs.text.contains("Project Roadmap"));

    let manager_outputs = empty_request(
        router.clone(),
        "GET",
        "/v1/outputs",
        Some(&manager_token),
        None,
    )
    .await;
    assert_eq!(manager_outputs.status, StatusCode::OK);
    assert_eq!(
        manager_outputs.json["outputs"]
            .as_array()
            .expect("outputs")
            .len(),
        0
    );

    let download = empty_request(
        router.clone(),
        "GET",
        &format!("/v1/outputs/{output_id}/download"),
        Some(&developer_token),
        None,
    )
    .await;
    assert_eq!(download.status, StatusCode::OK);
    assert!(download.text.contains("Delivery Report"));

    let manager_cannot_download_user_output = empty_request(
        router.clone(),
        "GET",
        &format!("/v1/outputs/{output_id}/download"),
        Some(&manager_token),
        None,
    )
    .await;
    assert_eq!(
        manager_cannot_download_user_output.status,
        StatusCode::NOT_FOUND
    );

    let outputs_page =
        empty_request(router, "GET", "/app/outputs", Some(&developer_token), None).await;
    assert_eq!(outputs_page.status, StatusCode::OK);
    assert!(outputs_page.text.contains("My Outputs"));
    assert!(outputs_page.text.contains("Operational outputs"));
    assert!(outputs_page.text.contains("End outputs"));
    assert!(outputs_page.text.contains("output-index"));
    assert!(!outputs_page.text.contains("Record Output Metadata"));
}

#[tokio::test]
async fn rbac_page_explains_permissions_and_exposes_assignment_crud() {
    let router = api::build_test_router();
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let bootstrap = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        None,
        serde_json::json!({
            "owner_email": "owner@example.com",
            "owner_password": "owner-password",
            "workspace_roots": [temp_dir.path()],
        }),
    )
    .await;
    assert_eq!(bootstrap.status, StatusCode::CREATED);
    let token = bootstrap.json["api_token"]
        .as_str()
        .expect("token")
        .to_string();

    let rbac_page = empty_request(router, "GET", "/admin/rbac", Some(&token), None).await;
    assert_eq!(rbac_page.status, StatusCode::OK);
    assert!(rbac_page.text.contains("RBAC Role Assignment CRUD"));
    assert!(rbac_page.text.contains("Effective Permission Matrix"));
    for role in ["admin", "manager", "developer", "viewer"] {
        assert!(rbac_page.text.contains(role), "{role}");
    }
    for action in [
        "administer_users",
        "assign_roles",
        "manage_workspaces",
        "manage_context_packs",
        "manage_outputs",
        "manage_own_context_packs",
        "start_worker",
        "read_threads",
        "read_audit",
    ] {
        assert!(rbac_page.text.contains(action), "{action}");
    }
    assert!(rbac_page.text.contains("Create assignment"));
    assert!(rbac_page.text.contains("Update assignment"));
    assert!(rbac_page.text.contains("Remove assignment"));
    assert!(
        !rbac_page
            .text
            .contains("Custom role and policy editing is deferred")
    );
}

#[tokio::test]
async fn terminal_page_uses_browser_login_and_documents_cli_login() {
    let router = api::build_test_router();
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let bootstrap = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        None,
        serde_json::json!({
            "owner_email": "owner@example.com",
            "owner_password": "owner-password",
            "workspace_roots": [temp_dir.path()],
        }),
    )
    .await;
    assert_eq!(bootstrap.status, StatusCode::CREATED);
    let token = bootstrap.json["api_token"]
        .as_str()
        .expect("token")
        .to_string();

    let unauthenticated = empty_request(router.clone(), "GET", "/app/terminal", None, None).await;
    assert_eq!(unauthenticated.status, StatusCode::SEE_OTHER);
    assert_eq!(unauthenticated.location.as_deref(), Some("/login"));

    let terminal = empty_request(router.clone(), "GET", "/app/terminal", Some(&token), None).await;
    assert_eq!(terminal.status, StatusCode::OK);
    assert!(terminal.text.contains("Terminal Login Instructions"));
    assert!(terminal.text.contains("/v1/auth/login"));
    assert!(terminal.text.contains("LOCAL_CODEX_ENTERPRISE_TOKEN"));
    assert!(terminal.text.contains("Use Browser Workbench"));
    assert!(!terminal.text.contains(r#"id="workbench-chat""#));
    assert!(!terminal.text.contains("1. Session"));
    assert!(!terminal.text.contains("2. Worker"));
    assert!(!terminal.text.contains("3. Connect"));

    let chat = empty_request(router, "GET", "/chat", Some(&token), None).await;
    assert_eq!(chat.status, StatusCode::OK);
    assert!(chat.text.contains("Local Codex Chat"));
    assert!(chat.text.contains("workbench-chat"));
    assert!(chat.text.contains("composer-input"));
    assert!(!chat.text.contains("Open thread"));
    assert!(chat.text.contains("ensureWorkbenchConnected"));
    assert!(chat.text.contains("sendWorkbenchRpcMessage"));
    assert!(chat.text.contains("initializeWorkbenchRpc"));
    assert!(chat.text.contains("retryWorkbenchMessageOnce"));
    assert!(chat.text.contains("thread/start"));
    assert!(chat.text.contains("turn/start"));
    assert!(!chat.text.contains("Codex app-server connection is ready."));
    assert!(
        !chat
            .text
            .contains("Browser handoff is active. Messages are sent through the worker websocket.")
    );
    assert!(!chat.text.contains("workbench.socket.send(text)"));
    assert!(chat.text.contains("Projects"));
    assert!(chat.text.contains("openProjectMenu"));
    assert!(chat.text.contains("Create project"));
    assert!(chat.text.contains("Add repository"));
    assert!(!chat.text.contains("Clone into workspace"));
    assert!(chat.text.contains("chat-shell-fullscreen"));
    assert!(
        !chat
            .text
            .contains("Self-hosted control plane for governed local Codex sessions")
    );
}

#[tokio::test]
async fn developer_workspaces_are_assigned_per_user_not_global_roots() {
    let router = api::build_test_router();
    let temp = tempfile::tempdir().expect("temp dir");
    let workspace_root = temp.path().join("workspaces");
    let project = workspace_root.join("project");
    std::fs::create_dir_all(&project).expect("project workspace");

    let setup = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        None,
        serde_json::json!({
            "owner_email": "owner@example.com",
            "owner_password": "owner-password",
            "workspace_roots": [workspace_root],
        }),
    )
    .await;
    assert_eq!(setup.status, StatusCode::CREATED);
    let admin_token = setup.json["api_token"]
        .as_str()
        .expect("admin token")
        .to_string();
    let (developer_token, developer_id) = create_and_login_user(
        router.clone(),
        &admin_token,
        "assigned-dev@example.com",
        "developer-password",
        "developer",
    )
    .await;
    let me = empty_request(
        router.clone(),
        "GET",
        "/v1/auth/me",
        Some(&developer_token),
        None,
    )
    .await;
    assert_eq!(me.status, StatusCode::OK, "{}", me.text);
    assert_eq!(me.json["role"], "developer");

    let denied = json_request(
        router.clone(),
        "POST",
        "/v1/threads",
        Some(&developer_token),
        None,
        serde_json::json!({
            "workspace_path": project,
            "title": "Unassigned workspace"
        }),
    )
    .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);

    let assigned = json_request(
        router.clone(),
        "POST",
        "/v1/user-workspace-access-grants",
        Some(&admin_token),
        None,
        serde_json::json!({
            "user_id": developer_id,
            "workspace_root": project
        }),
    )
    .await;
    assert_eq!(assigned.status, StatusCode::CREATED);

    let listed = empty_request(
        router.clone(),
        "GET",
        "/v1/user-workspaces",
        Some(&developer_token),
        None,
    )
    .await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "developer_id={developer_id} token_prefix={} body={}",
        &developer_token[..developer_token.len().min(10)],
        listed.text
    );
    assert!(listed.text.contains("project"));
    assert!(!listed.text.contains("owner@example.com"));

    let unsafe_clone = json_request(
        router.clone(),
        "POST",
        "/v1/projects/PROJECT_ID/repositories/clone",
        Some(&developer_token),
        None,
        serde_json::json!({
            "repo_url": "file:///tmp/repo.git",
            "destination_name": "repo"
        }),
    )
    .await;
    assert_eq!(unsafe_clone.status, StatusCode::BAD_REQUEST);
    assert!(
        unsafe_clone.text.contains("repo URL must use https"),
        "{}",
        unsafe_clone.text
    );
    let selected_workspace_clone = json_request(
        router.clone(),
        "POST",
        "/v1/projects/PROJECT_ID/repositories/clone",
        Some(&developer_token),
        None,
        serde_json::json!({
            "repo_url": "file:///tmp/repo.git",
            "destination_name": "repo",
            "workspace_root": project
        }),
    )
    .await;
    assert_eq!(selected_workspace_clone.status, StatusCode::BAD_REQUEST);
    assert!(
        selected_workspace_clone
            .text
            .contains("repo URL must use https"),
        "{}",
        selected_workspace_clone.text
    );
    let unassigned_workspace_clone = json_request(
        router.clone(),
        "POST",
        "/v1/projects/PROJECT_ID/repositories/clone",
        Some(&developer_token),
        None,
        serde_json::json!({
            "repo_url": "https://example.com/org/repo.git",
            "destination_name": "repo",
            "workspace_root": workspace_root
        }),
    )
    .await;
    assert_eq!(unassigned_workspace_clone.status, StatusCode::NOT_FOUND);
    assert!(
        unassigned_workspace_clone
            .text
            .contains("project not found"),
        "{}",
        unassigned_workspace_clone.text
    );

    let app = empty_request(router.clone(), "GET", "/app", Some(&developer_token), None).await;
    assert_eq!(app.status, StatusCode::OK);
    assert!(app.text.contains("Browser Workbench"));
    assert!(app.text.contains("/chat"));

    let allowed = json_request(
        router,
        "POST",
        "/v1/threads",
        Some(&developer_token),
        None,
        serde_json::json!({
            "workspace_path": project,
            "title": "Assigned workspace"
        }),
    )
    .await;
    assert_eq!(allowed.status, StatusCode::CREATED);
}

#[tokio::test]
async fn workspace_access_can_be_delegated_by_a_user_who_has_that_workspace() {
    let router = api::build_test_router();
    let temp = tempfile::tempdir().expect("temp dir");
    let workspace_root = temp.path().join("workspaces");
    let project = workspace_root.join("project");
    std::fs::create_dir_all(&project).expect("project workspace");

    let setup = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        None,
        serde_json::json!({
            "owner_email": "admin@example.com",
            "owner_password": "admin-password",
            "workspace_roots": [workspace_root],
        }),
    )
    .await;
    assert_eq!(setup.status, StatusCode::CREATED);
    let admin_token = setup.json["api_token"]
        .as_str()
        .expect("admin token")
        .to_string();
    let (workspace_owner_token, workspace_owner_id) = create_and_login_user(
        router.clone(),
        &admin_token,
        "workspace-owner@example.com",
        "developer-password",
        "developer",
    )
    .await;
    let (guest_token, guest_id) = create_and_login_user(
        router.clone(),
        &admin_token,
        "workspace-guest@example.com",
        "developer-password",
        "developer",
    )
    .await;

    let assigned = json_request(
        router.clone(),
        "POST",
        "/v1/user-workspace-access-grants",
        Some(&admin_token),
        None,
        serde_json::json!({
            "user_id": workspace_owner_id,
            "workspace_root": project
        }),
    )
    .await;
    assert_eq!(assigned.status, StatusCode::CREATED, "{}", assigned.text);

    let guest_denied = json_request(
        router.clone(),
        "POST",
        "/v1/threads",
        Some(&guest_token),
        None,
        serde_json::json!({
            "workspace_path": project,
            "title": "No grant yet"
        }),
    )
    .await;
    assert_eq!(guest_denied.status, StatusCode::FORBIDDEN);

    let delegated = json_request(
        router.clone(),
        "POST",
        "/v1/user-workspace-access-grants",
        Some(&workspace_owner_token),
        None,
        serde_json::json!({
            "user_id": guest_id,
            "workspace_root": project
        }),
    )
    .await;
    assert_eq!(delegated.status, StatusCode::CREATED, "{}", delegated.text);

    let guest_allowed = json_request(
        router,
        "POST",
        "/v1/threads",
        Some(&guest_token),
        None,
        serde_json::json!({
            "workspace_path": project,
            "title": "Delegated workspace"
        }),
    )
    .await;
    assert_eq!(
        guest_allowed.status,
        StatusCode::CREATED,
        "{}",
        guest_allowed.text
    );
}

#[tokio::test]
async fn enterprise_web_pages_render_without_private_example_content() {
    let router = api::build_test_router();
    for uri in ["/setup"] {
        let response = Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .expect("request");
        let response = router.clone().oneshot(response).await.expect("response");
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let body = String::from_utf8(body.to_vec()).expect("utf8");
        assert!(body.contains("Local Codex for Enterprise"));
        assert!(!body.contains("OPENAI_API_KEY"));
        assert!(!body.contains("GITHUB_TOKEN"));
        assert!(!body.contains(r#"<textarea id="setup-roots">/enterprise-workspaces"#));
    }
}

#[tokio::test]
async fn web_pages_follow_setup_then_login_lifecycle() {
    let mut config = EnterpriseConfig::default();
    config.default_workspace_root = Some("/enterprise-workspaces".to_string());
    let router = api::build_router_with_store(InMemoryEnterpriseStore::default(), config);

    let root_before_setup = empty_request(router.clone(), "GET", "/", None, None).await;
    assert_eq!(root_before_setup.status, StatusCode::SEE_OTHER);
    assert_eq!(root_before_setup.location.as_deref(), Some("/setup"));

    let login_before_setup = empty_request(router.clone(), "GET", "/login", None, None).await;
    assert_eq!(login_before_setup.status, StatusCode::SEE_OTHER);
    assert_eq!(login_before_setup.location.as_deref(), Some("/setup"));

    let setup_before_setup = empty_request(router.clone(), "GET", "/setup", None, None).await;
    assert_eq!(setup_before_setup.status, StatusCode::OK);
    assert!(setup_before_setup.text.contains("Bootstrap Admin"));
    assert!(setup_before_setup.text.contains("Initial Admin Account"));
    assert!(
        setup_before_setup
            .text
            .contains("Initial allowed workspace roots")
    );
    assert!(setup_before_setup.text.contains("/enterprise-workspaces"));
    assert!(
        setup_before_setup
            .text
            .contains("Docker Compose: use /enterprise-workspaces")
    );
    assert!(
        setup_before_setup
            .text
            .contains("Local install: use a path that exists on the server")
    );
    assert!(!setup_before_setup.text.contains("Create User"));
    assert!(!setup_before_setup.text.contains("Register Workspace"));

    let temp_dir = tempfile::tempdir().expect("temp dir");
    let bootstrap = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        None,
        serde_json::json!({
            "owner_email": "owner@example.com",
            "owner_password": "owner-password",
            "workspace_roots": [temp_dir.path()],
        }),
    )
    .await;
    assert_eq!(bootstrap.status, StatusCode::CREATED);
    let token = bootstrap.json["api_token"]
        .as_str()
        .expect("token")
        .to_string();

    let root_after_setup = empty_request(router.clone(), "GET", "/", None, None).await;
    assert_eq!(root_after_setup.status, StatusCode::SEE_OTHER);
    assert_eq!(root_after_setup.location.as_deref(), Some("/login"));

    let root_after_setup_with_auth =
        empty_request(router.clone(), "GET", "/", Some(&token), None).await;
    assert_eq!(root_after_setup_with_auth.status, StatusCode::SEE_OTHER);
    assert_eq!(
        root_after_setup_with_auth.location.as_deref(),
        Some("/chat")
    );

    let admin_after_setup_without_auth =
        empty_request(router.clone(), "GET", "/admin", None, None).await;
    assert_eq!(admin_after_setup_without_auth.status, StatusCode::SEE_OTHER);
    assert_eq!(
        admin_after_setup_without_auth.location.as_deref(),
        Some("/login")
    );

    let setup_after_setup = empty_request(router.clone(), "GET", "/setup", None, None).await;
    assert_eq!(setup_after_setup.status, StatusCode::SEE_OTHER);
    assert_eq!(setup_after_setup.location.as_deref(), Some("/login"));

    let login_after_setup = empty_request(router, "GET", "/login", None, None).await;
    assert_eq!(login_after_setup.status, StatusCode::OK);
    assert!(login_after_setup.text.contains("Sign In"));
    assert!(login_after_setup.text.contains("Credentials"));
    assert!(!login_after_setup.text.contains("Bootstrap Admin"));
}

#[tokio::test]
async fn login_page_redirects_home_and_nav_says_home_when_authenticated() {
    let router = api::build_test_router();
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let bootstrap = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        None,
        serde_json::json!({
            "owner_email": "owner@example.com",
            "owner_password": "owner-password",
            "workspace_roots": [temp_dir.path()],
        }),
    )
    .await;
    assert_eq!(bootstrap.status, StatusCode::CREATED);
    let token = bootstrap.json["api_token"]
        .as_str()
        .expect("token")
        .to_string();

    let login_with_auth = empty_request(router.clone(), "GET", "/login", Some(&token), None).await;
    assert_eq!(login_with_auth.status, StatusCode::SEE_OTHER);
    assert_eq!(login_with_auth.location.as_deref(), Some("/chat"));

    let chat_with_auth = empty_request(router, "GET", "/chat", Some(&token), None).await;
    assert_eq!(chat_with_auth.status, StatusCode::OK);
    assert!(chat_with_auth.text.contains("account-menu"));
    assert!(chat_with_auth.text.contains("owner@example.com (admin)"));
    assert!(chat_with_auth.text.contains(r#"<a href="/chat">Chat</a>"#));
    assert!(
        chat_with_auth
            .text
            .contains(r#"<a href="/admin">Admin</a>"#)
    );
    assert!(
        !chat_with_auth
            .text
            .contains(r#"<a href="/login">Login</a>"#)
    );
    assert!(
        chat_with_auth
            .text
            .contains("Local Codex for Enterprise v0.0.1-beta.4")
    );
    assert!(chat_with_auth.text.contains("Made with Codex"));
    assert!(
        chat_with_auth
            .text
            .contains(r#"<div class="chat-brand-corner"><strong>Local Codex for Enterprise</strong><div class="chat-brand-meta"><span class="chat-brand-motto">Made with Codex</span><span class="chat-brand-version">v0.0.1-beta.4</span></div></div>"#)
    );
    let footer_index = chat_with_auth
        .text
        .find(r#"<div class="chat-rail-footer">"#)
        .expect("rail footer");
    let brand_index = chat_with_auth
        .text
        .find(r#"<div class="chat-brand-corner">"#)
        .expect("rail brand");
    assert!(brand_index > footer_index);
    assert!(
        chat_with_auth
            .text
            .contains("Ask Codex to inspect, edit, test, or explain this project.")
    );
    assert!(
        !chat_with_auth
            .text
            .contains("Ask Codex to inspect, edit, test, or explain this workspace.")
    );
}

#[tokio::test]
async fn visible_navigation_matches_role_permissions() {
    let router = api::build_test_router();
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let bootstrap = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        None,
        serde_json::json!({
            "owner_email": "owner@example.com",
            "owner_password": "owner-password",
            "workspace_roots": [temp_dir.path()],
        }),
    )
    .await;
    assert_eq!(bootstrap.status, StatusCode::CREATED);
    let admin_token = bootstrap.json["api_token"]
        .as_str()
        .expect("token")
        .to_string();
    let (manager_token, _) = create_and_login_user(
        router.clone(),
        &admin_token,
        "manager@example.com",
        "manager-password",
        "manager",
    )
    .await;
    let (viewer_token, _) = create_and_login_user(
        router.clone(),
        &admin_token,
        "viewer@example.com",
        "viewer-password",
        "viewer",
    )
    .await;

    let viewer_chat =
        empty_request(router.clone(), "GET", "/chat", Some(&viewer_token), None).await;
    assert_eq!(viewer_chat.status, StatusCode::OK);
    assert!(viewer_chat.text.contains("viewer@example.com (viewer)"));
    assert!(viewer_chat.text.contains(r#"<a href="/chat">Chat</a>"#));
    assert!(!viewer_chat.text.contains(r#"<a href="/admin">Admin</a>"#));
    assert!(!viewer_chat.text.contains("Admin / settings"));
    assert!(!viewer_chat.text.contains("Admin Console"));

    let manager_chat =
        empty_request(router.clone(), "GET", "/chat", Some(&manager_token), None).await;
    assert_eq!(manager_chat.status, StatusCode::OK);
    assert!(manager_chat.text.contains("manager@example.com (manager)"));
    assert!(
        manager_chat
            .text
            .contains(r#"<a href="/admin">Admin / settings</a>"#)
    );
    assert!(!manager_chat.text.contains("Users index"));
    assert!(!manager_chat.text.contains("Context pack index"));

    let manager_admin =
        empty_request(router.clone(), "GET", "/admin", Some(&manager_token), None).await;
    assert_eq!(manager_admin.status, StatusCode::OK);
    assert!(manager_admin.text.contains("Manager Console"));
    assert!(manager_admin.text.contains("Reports and outputs"));
    assert!(manager_admin.text.contains("Audit"));
    assert!(!manager_admin.text.contains("Users index"));
    assert!(!manager_admin.text.contains("RBAC role assignment"));
    assert!(!manager_admin.text.contains("Workspace index"));
    assert!(!manager_admin.text.contains("Context pack index"));
    let manager_users = empty_request(
        router.clone(),
        "GET",
        "/admin/users",
        Some(&manager_token),
        None,
    )
    .await;
    assert_eq!(manager_users.status, StatusCode::FORBIDDEN);

    let admin_admin = empty_request(router, "GET", "/admin", Some(&admin_token), None).await;
    assert_eq!(admin_admin.status, StatusCode::OK);
    assert!(admin_admin.text.contains("Admin Console"));
    assert!(admin_admin.text.contains("Users index"));
    assert!(admin_admin.text.contains("RBAC role assignment"));
    assert!(admin_admin.text.contains("Workspace index"));
    assert!(admin_admin.text.contains("Context pack index"));
}

#[test]
fn worker_rpc_proxy_uses_app_server_unix_rpc_endpoint() {
    let api_source = include_str!("../src/api.rs");
    assert!(api_source.contains("client_async(\"ws://localhost/rpc\""));
    assert!(!api_source.contains("client_async(\"ws://localhost/\""));
}

#[test]
fn enterprise_domain_contract_defines_workspace_project_repo_thread_hierarchy() {
    let contract = include_str!("../../../docs/enterprise-domain-contract.md");
    for expected in [
        "Workspace root",
        "User workspace",
        "Project",
        "Repository",
        "Thread",
        "workspace root -> user workspace -> project -> repository -> thread",
        "Workspace is not a project",
        "Projects are not filesystem security boundaries",
        "Repositories are not user workspaces",
        "Threads are not login sessions",
        "GET /v1/user-workspaces",
        "POST /v1/user-workspaces/{user_workspace_id}/projects",
        "POST /v1/projects/{project_id}/repositories/clone",
        "GET /v1/threads/{thread_id}",
        "POST /v1/threads/{thread_id}/workers",
        "POST /v1/workers/{worker_id}/handoffs",
    ] {
        assert!(contract.contains(expected), "{expected}");
    }
}

#[tokio::test]
async fn chat_rail_groups_threads_under_workspace_project_headers() {
    let router = api::build_test_router();
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let workspace_root = temp_dir.path().join("workspaces");
    let project_a = workspace_root.join("project-a");
    let project_b = workspace_root.join("project-b");
    std::fs::create_dir_all(&project_a).expect("project a");
    std::fs::create_dir_all(&project_b).expect("project b");
    let bootstrap = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        None,
        serde_json::json!({
            "owner_email": "owner@example.com",
            "owner_password": "owner-password",
            "workspace_roots": [project_a, project_b],
        }),
    )
    .await;
    assert_eq!(bootstrap.status, StatusCode::CREATED);
    let token = bootstrap.json["api_token"]
        .as_str()
        .expect("token")
        .to_string();
    let developer = json_request(
        router.clone(),
        "POST",
        "/v1/users",
        Some(&token),
        None,
        serde_json::json!({
            "email": "developer@example.com",
            "password": "developer-password",
            "role": "developer",
            "workspace_roots": [project_a, project_b],
        }),
    )
    .await;
    assert_eq!(developer.status, StatusCode::CREATED);
    let login = json_request(
        router.clone(),
        "POST",
        "/v1/auth/login",
        None,
        None,
        serde_json::json!({
            "email": "developer@example.com",
            "password": "developer-password",
        }),
    )
    .await;
    assert_eq!(login.status, StatusCode::OK);
    let developer_token = login.json["api_token"]
        .as_str()
        .expect("developer token")
        .to_string();

    let first_thread = json_request(
        router.clone(),
        "POST",
        "/v1/threads",
        Some(&developer_token),
        None,
        serde_json::json!({
            "workspace_path": project_a,
            "title": "Inspect project A"
        }),
    )
    .await;
    assert_eq!(
        first_thread.status,
        StatusCode::CREATED,
        "{}",
        first_thread.text
    );
    let second_thread = json_request(
        router.clone(),
        "POST",
        "/v1/threads",
        Some(&developer_token),
        None,
        serde_json::json!({
            "workspace_path": project_b,
            "title": "Review project B"
        }),
    )
    .await;
    assert_eq!(
        second_thread.status,
        StatusCode::CREATED,
        "{}",
        second_thread.text
    );

    let chat = empty_request(router, "GET", "/chat", Some(&developer_token), None).await;
    assert_eq!(chat.status, StatusCode::OK);
    assert!(chat.text.contains("Projects"));
    assert!(chat.text.contains("toggleChatRail"));
    assert!(chat.text.contains("chat-rail-collapsed"));
    assert!(chat.text.contains("workbench-thread-title"));
    assert!(chat.text.contains("setWorkbenchThreadTitle"));
    assert!(chat.text.contains("thread-title-edit-button"));
    assert!(chat.text.contains("beginWorkbenchTitleEdit"));
    assert!(chat.text.contains("saveWorkbenchTitleEdit"));
    assert!(chat.text.contains("thread-context-menu"));
    assert!(chat.text.contains("openThreadContextMenu"));
    assert!(chat.text.contains("renameThreadFromContextMenu"));
    assert!(chat.text.contains("thread-rename-modal"));
    assert!(chat.text.contains("async function patchJsonPayload"));
    assert!(chat.text.contains("removeWorkbenchThread"));
    assert!(chat.text.contains("Remove thread"));
    assert!(chat.text.contains("Rename thread"));
    assert!(chat.text.contains("oncontextmenu=\"openThreadContextMenu"));
    assert!(chat.text.contains("deleteJsonPayload('/v1/threads/'"));
    assert!(chat.text.contains("send-button"));
    assert!(chat.text.contains(".chat-composer .send-button"));
    assert!(
        chat.text
            .contains("/v1/threads/'+encodeURIComponent(workbench.sessionId)")
    );
    assert!(chat.text.contains("Local Codex for Enterprise"));
    assert!(chat.text.contains("Made with Codex"));
    assert!(chat.text.contains("chat-brand-version"));
    assert!(chat.text.contains("chat-rail-footer"));
    assert!(chat.text.contains("chat-account-menu"));
    assert!(chat.text.contains("account-menu-panel"));
    assert!(chat.text.contains("Terminal instructions"));
    assert!(!chat.text.contains(">+ New chat<"));
    assert!(chat.text.contains("composer-hidden"));
    assert!(chat.text.contains("setComposerVisible"));
    assert!(chat.text.contains("selectLastWorkbenchThread"));
    assert!(
        chat.text
            .contains("Please select a thread to start chatting")
    );
    assert!(chat.text.contains("Write a message to create this thread."));
    assert!(!chat.text.contains(">Open thread<"));
    assert!(!chat.text.contains("Start or resume a chat thread"));
    assert!(chat.text.contains("ensureWorkbenchConnected"));
    assert!(chat.text.contains("sendWorkbenchRpcMessage"));
    assert!(chat.text.contains("initializeWorkbenchRpc"));
    assert!(chat.text.contains("retryWorkbenchMessageOnce"));
    assert!(!chat.text.contains("await retryWorkbenchMessageOnce(text, firstError);\n          await retryWorkbenchMessageOnce(text, firstError);"));
    assert!(chat.text.contains("thread/start"));
    assert!(chat.text.contains("buildWorkbenchUserPrompt"));
    assert!(chat.text.contains("Conceptual planning request"));
    assert!(
        chat.text
            .contains("do not inspect the repository unless the user explicitly asks")
    );
    assert!(chat.text.contains("workbenchTurnGuidance"));
    assert!(chat.text.contains("chat-turn-guidance"));
    assert!(
        chat.text
            .contains("Before using repository tools, decide whether")
    );
    assert!(chat.text.contains("business planning, architecture advice"));
    assert!(
        chat.text
            .contains("Do not print raw HTML, JSON, or full tool output")
    );
    assert!(chat.text.contains("business goal"));
    assert!(chat.text.contains("users/stakeholders"));
    assert!(chat.text.contains("decisions the system must support"));
    assert!(chat.text.contains("turn/start"));
    assert!(chat.text.contains("item/agentMessage/delta"));
    assert!(chat.text.contains("handleWorkbenchCommandOutputDelta"));
    assert!(chat.text.contains("Raw tool output is collapsed"));
    assert!(
        !chat
            .text
            .contains("workbenchMessage('assistant', 'Command', params.delta)")
    );
    assert!(!chat.text.contains("Codex app-server connection is ready."));
    assert!(!chat.text.contains("Worker ready"));
    assert!(!chat.text.contains("Thread ready"));
    assert!(
        !chat
            .text
            .contains("Browser handoff is active. Messages are sent through the worker websocket.")
    );
    assert!(!chat.text.contains("workbench.socket.send(text)"));
    assert!(!chat.text.contains("Start / Resume Thread"));
    assert!(chat.text.contains("workbench-project-list"));
    assert!(chat.text.contains("project-header"));
    assert!(chat.text.contains("projects-menu-button"));
    assert!(chat.text.contains("projects-new-button"));
    assert!(chat.text.contains("setWorkbenchProjectCreationAvailable"));
    assert!(chat.text.contains("showNoUserWorkspaceState"));
    assert!(
        chat.text
            .contains("New projects are unavailable until a user workspace is assigned")
    );
    assert!(chat.text.contains("project-menu-button"));
    assert!(chat.text.contains("project-new-thread-button"));
    assert!(chat.text.contains("chat-bubble-icon"));
    assert!(chat.text.contains("workbenchIcon"));
    assert!(chat.text.contains("lucide-icon"));
    assert!(chat.text.contains("'message-circle'"));
    assert!(chat.text.contains("data-icon=\"more-horizontal\""));
    assert!(chat.text.contains("data-icon=\"pencil\""));
    assert!(chat.text.contains("data-icon=\"send\""));
    assert!(chat.text.contains("AccentColor"));
    assert!(chat.text.contains("--lc-accent"));
    assert!(chat.text.contains("@media (max-width: 480px)"));
    assert!(
        chat.text
            .contains("@media (min-width: 481px) and (max-width: 820px)")
    );
    assert!(chat.text.contains("@media (min-width: 1440px)"));
    assert!(!chat.text.contains(">＋<"));
    assert!(!chat.text.contains(">⋯<"));
    assert!(!chat.text.contains(">✎<"));
    assert!(chat.text.contains("project-menu-dropdown"));
    assert!(chat.text.contains("Add repository"));
    assert!(chat.text.contains("Remove project"));
    assert!(chat.text.contains("removeWorkbenchProject"));
    assert!(chat.text.contains("openNewProjectModal"));
    assert!(chat.text.contains("createWorkbenchProject"));
    assert!(chat.text.contains("addRepositoryToSelectedProject"));
    assert!(!chat.text.contains("New workspace/project"));
    assert!(!chat.text.contains("Clone into workspace"));
    assert!(chat.text.contains("openProjectMenu"));
    assert!(chat.text.contains("repository-selected-project"));
    assert!(chat.text.contains("My outputs"));
    assert!(chat.text.contains("openWorkbenchOutputs"));
    assert!(chat.text.contains("workbench-output-list"));
    assert!(chat.text.contains("chooseWorkbenchProject"));
    assert!(chat.text.contains("selectWorkbenchThread"));
    assert!(chat.text.contains("clearWorkbenchTranscript"));
    assert!(chat.text.contains("loadWorkbenchThreadHistory"));
    assert!(chat.text.contains("recordWorkbenchThreadMessage"));
    assert!(
        chat.text
            .contains("autoLabelWorkbenchThreadAfterFirstAiTurn")
    );
    assert!(chat.text.contains("deriveWorkbenchThreadTitle"));
    assert!(chat.text.contains("isDefaultWorkbenchThreadTitle"));
    assert!(chat.text.contains("normalized === 'New chat thread'"));
    assert!(chat.text.contains("firstUserTurn"));
    assert!(chat.text.contains("firstAssistantTurn"));
    assert!(
        chat.text
            .contains("if (!isDefaultWorkbenchThreadTitle(workbench.threadTitle)) return")
    );
    assert!(
        chat.text
            .contains("await recordWorkbenchThreadMessage('assistant', 'Codex', assistantText)")
    );
    assert!(
        chat.text
            .contains("await autoLabelWorkbenchThreadAfterFirstAiTurn()")
    );
    assert!(
        chat.text
            .contains("renameWorkbenchThread(workbench.sessionId, title)")
    );
    assert!(chat.text.contains("composerKeydown"));
    assert!(chat.text.contains("showWorkbenchEmptyState"));
    assert!(chat.text.contains("clearEphemeralMessages"));
    assert!(chat.text.contains("appendWorkbenchAssistantPending"));
    assert!(chat.text.contains("replacePendingAssistantDelta"));
    assert!(chat.text.contains("appendWorkbenchAssistantDelta"));
    assert!(chat.text.contains("completeWorkbenchAssistantTurn"));
    assert!(chat.text.contains("completeLastMessage"));
    assert!(chat.text.contains("copyWorkbenchMessage"));
    assert!(chat.text.contains("resubmitWorkbenchMessage"));
    assert!(chat.text.contains("beginEditResubmitWorkbenchMessage"));
    assert!(chat.text.contains("submitEditedWorkbenchMessage"));
    assert!(chat.text.contains("data-resubmit-index"));
    assert!(chat.text.contains("data-begin-edit-index"));
    assert!(chat.text.contains("data-submit-edit-index"));
    assert!(chat.text.contains("inline-turn-editor"));
    assert!(chat.text.contains("data-edit-value-index"));
    assert!(chat.text.contains("title=\"Resubmit turn\""));
    assert!(chat.text.contains("title=\"Edit turn\""));
    assert!(chat.text.contains("message.kind === 'user'"));
    assert!(
        chat.text
            .contains("this.setMessages(this.messages.slice(0, index + 1))")
    );
    assert!(
        !chat
            .text
            .contains("window.prompt('Edit and resubmit this turn', text)")
    );
    assert!(chat.text.contains("workbench.appThreadId = null"));
    assert!(
        chat.text
            .contains("await sendWorkbenchRpcMessage(text, {recordUser:false, appendUser:false})")
    );
    assert!(chat.text.contains("isSocialWorkbenchMessage"));
    assert!(chat.text.contains("isPlanningWorkbenchMessage"));
    assert!(chat.text.contains("Conversational acknowledgement"));
    assert!(chat.text.contains("Reply briefly and naturally"));
    assert!(chat.text.contains("hello"));
    assert!(chat.text.contains("Do not start planning"));
    assert!(chat.text.contains("Do not mention unavailable tools"));
    assert!(chat.text.contains("General chat request"));
    assert!(chat.text.contains("Conceptual planning request"));
    assert!(chat.text.contains("copyable-message"));
    assert!(chat.text.contains("message-actions"));
    assert!(chat.text.contains("last-completed-assistant"));
    assert!(chat.text.contains(".message:hover .message-actions"));
    assert!(chat.text.contains(".message:focus-within .message-actions"));
    assert!(chat.text.contains("opacity:0"));
    assert!(chat.text.contains("color:#9aa3b2"));
    assert!(chat.text.contains("formatAssistantMessage(message)"));
    assert!(chat.text.contains("body+actions"));
    assert!(chat.text.contains("formatWorkbenchTurnTime"));
    assert!(chat.text.contains("11:31 pm"));
    assert!(chat.text.contains("Jun 8, 11:31 pm"));
    assert!(chat.text.contains("sameDay"));
    assert!(chat.text.contains("toLocaleDateString"));
    assert!(chat.text.contains("message-timestamp"));
    assert!(chat.text.contains("message-actions-left"));
    assert!(
        chat.text
            .contains("createdAt:message.created_at || message.createdAt")
    );
    assert!(
        chat.text
            .contains("message.created_at || message.createdAt || new Date().toISOString()")
    );
    assert!(chat.text.contains("new Date()"));
    assert!(chat.text.contains("findLastIndex"));
    assert!(!chat.text.contains(
        "message-header\"><small>'+workbenchEscape(message.label)+'</small>'+copyButton"
    ));
    assert!(chat.text.contains("thinking-dots"));
    assert!(chat.text.contains("@keyframes thinkingPulse"));
    assert!(
        chat.text
            .contains("message.pending && message.kind === 'assistant'")
    );
    assert!(chat.text.contains("data-copy-index"));
    assert!(chat.text.contains("'copy'"));
    assert!(chat.text.contains("Copy turn"));
    assert!(
        chat.text
            .contains("message.kind === 'user' || message.kind === 'assistant'")
    );
    assert!(chat.text.contains("!message.streaming"));
    assert!(chat.text.contains("reverse().find"));
    assert!(chat.text.contains("renderWorkbenchMarkdown"));
    assert!(chat.text.contains("formatAssistantMessage"));
    assert!(chat.text.contains("scrollToBottom()"));
    assert!(chat.text.contains("this.stickToBottom"));
    assert!(chat.text.contains("overflow-y:auto"));
    assert!(
        chat.text
            .contains("position: fixed; inset: 0; overflow: hidden")
    );
    assert!(chat.text.contains("height: 100%; overflow: hidden"));
    assert!(
        chat.text
            .contains("/v1/threads/'+encodeURIComponent(workbench.sessionId)+'/messages")
    );
    assert!(!chat.text.contains(
        "Loaded chat thread '+(session.title || session.session_id)+' into this conversation area."
    ));
    assert!(chat.text.contains("<workbench-transcript"));
    assert!(chat.text.contains("attachShadow"));
    assert!(chat.text.contains("maxRenderedMessages"));
    assert!(!chat.text.contains("thread-meta"));
    assert!(!chat.text.contains(r#"id="workbench-session-title""#));
    assert!(!chat.text.contains(r#"<select id="workbench-workspace""#));
}

#[tokio::test]
async fn projects_are_created_inside_user_workspaces_and_own_threads_and_repositories() {
    let router = api::build_test_router();
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let workspace_root = temp_dir.path().join("enterprise-workspaces");
    std::fs::create_dir_all(&workspace_root).expect("workspace root");

    let bootstrap = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        None,
        serde_json::json!({
            "owner_email": "owner@example.com",
            "owner_password": "owner-password",
            "workspace_roots": [workspace_root],
        }),
    )
    .await;
    assert_eq!(bootstrap.status, StatusCode::CREATED, "{}", bootstrap.text);
    let admin_token = bootstrap.json["api_token"]
        .as_str()
        .expect("admin token")
        .to_string();

    let user_workspace = workspace_root.join("project-dev");
    std::fs::create_dir_all(&user_workspace).expect("user workspace");
    let (developer_token, _developer_id) = create_and_login_user_with_workspace(
        router.clone(),
        &admin_token,
        "project-dev@example.com",
        "developer-password",
        "developer",
        &user_workspace,
    )
    .await;

    let user_workspaces = empty_request(
        router.clone(),
        "GET",
        "/v1/user-workspaces",
        Some(&developer_token),
        None,
    )
    .await;
    assert_eq!(
        user_workspaces.status,
        StatusCode::OK,
        "{}",
        user_workspaces.text
    );
    let user_workspace_id = user_workspaces.json["user_workspaces"][0]["user_workspace_id"]
        .as_str()
        .expect("user workspace id");
    assert!(
        user_workspaces.json["user_workspaces"][0]["path"]
            .as_str()
            .expect("user workspace path")
            .ends_with("/project-dev"),
        "{}",
        user_workspaces.json["user_workspaces"][0]["path"]
    );

    let created_project = json_request(
        router.clone(),
        "POST",
        &format!("/v1/user-workspaces/{user_workspace_id}/projects"),
        Some(&developer_token),
        None,
        serde_json::json!({
            "name": "Client Portal"
        }),
    )
    .await;
    assert_eq!(
        created_project.status,
        StatusCode::CREATED,
        "{}",
        created_project.text
    );
    let project = &created_project.json["project"];
    let project_id = project["project_id"].as_str().expect("project id");
    assert_eq!(project["name"], "Client Portal");
    assert!(
        project["project_path"]
            .as_str()
            .expect("project path")
            .ends_with("/project-dev/projects/client-portal"),
        "{}",
        project["project_path"]
    );

    let listed = empty_request(
        router.clone(),
        "GET",
        &format!("/v1/user-workspaces/{user_workspace_id}/projects"),
        Some(&developer_token),
        None,
    )
    .await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    assert_eq!(listed.json["projects"][0]["project_id"], project_id);
    assert_eq!(
        listed.json["projects"][0]["repositories"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        listed.json["projects"][0]["threads"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    let created_thread = json_request(
        router.clone(),
        "POST",
        &format!("/v1/projects/{project_id}/threads"),
        Some(&developer_token),
        None,
        serde_json::json!({
            "title": "Inspect landing flow"
        }),
    )
    .await;
    assert_eq!(
        created_thread.status,
        StatusCode::CREATED,
        "{}",
        created_thread.text
    );
    assert_eq!(created_thread.json["session"]["project_id"], project_id);
    assert_eq!(
        created_thread.json["session"]["workspace_path"],
        project["project_path"]
    );
    let thread_id = created_thread.json["session"]["session_id"]
        .as_str()
        .expect("thread id");
    let fetched_thread = empty_request(
        router.clone(),
        "GET",
        &format!("/v1/threads/{thread_id}"),
        Some(&developer_token),
        None,
    )
    .await;
    assert_eq!(
        fetched_thread.status,
        StatusCode::OK,
        "{}",
        fetched_thread.text
    );
    assert_eq!(fetched_thread.json["session"]["project_id"], project_id);

    let unsafe_clone = json_request(
        router.clone(),
        "POST",
        &format!("/v1/projects/{project_id}/repositories/clone"),
        Some(&developer_token),
        None,
        serde_json::json!({
            "repo_url": "file:///tmp/repo.git",
            "destination_name": "api"
        }),
    )
    .await;
    assert_eq!(unsafe_clone.status, StatusCode::BAD_REQUEST);
    assert!(
        unsafe_clone.text.contains("repo URL must use https"),
        "{}",
        unsafe_clone.text
    );

    let chat = empty_request(router, "GET", "/chat", Some(&developer_token), None).await;
    assert_eq!(chat.status, StatusCode::OK);
    assert!(chat.text.contains("/v1/user-workspaces"));
    assert!(chat.text.contains("/repositories/clone"));
    assert!(chat.text.contains("/threads"));
    assert!(chat.text.contains("project_id"));
    assert!(chat.text.contains("repository_id"));
    assert!(!chat.text.contains("assigned workspace</span>"));
}

#[tokio::test]
async fn admin_can_crud_projects_under_a_selected_user_workspace() {
    let router = api::build_test_router();
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let workspace_root = temp_dir.path().join("enterprise-workspaces");
    std::fs::create_dir_all(&workspace_root).expect("workspace root");

    let bootstrap = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        None,
        serde_json::json!({
            "owner_email": "owner@example.com",
            "owner_password": "owner-password",
            "workspace_roots": [workspace_root],
        }),
    )
    .await;
    assert_eq!(bootstrap.status, StatusCode::CREATED, "{}", bootstrap.text);
    let admin_token = bootstrap.json["api_token"]
        .as_str()
        .expect("admin token")
        .to_string();

    let user_workspace = workspace_root.join("analyst");
    std::fs::create_dir_all(&user_workspace).expect("user workspace");
    let (_developer_token, developer_id) = create_and_login_user_with_workspace(
        router.clone(),
        &admin_token,
        "analyst@example.com",
        "developer-password",
        "developer",
        &user_workspace,
    )
    .await;

    let user_workspaces = empty_request(
        router.clone(),
        "GET",
        "/v1/user-workspaces",
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(
        user_workspaces.status,
        StatusCode::OK,
        "{}",
        user_workspaces.text
    );
    let selected_workspace = user_workspaces.json["user_workspaces"]
        .as_array()
        .expect("user workspaces")
        .iter()
        .find(|workspace| workspace["owner_user_id"] == developer_id)
        .expect("developer workspace");
    let user_workspace_id = selected_workspace["user_workspace_id"]
        .as_str()
        .expect("user workspace id");

    let created = json_request(
        router.clone(),
        "POST",
        &format!("/v1/user-workspaces/{user_workspace_id}/projects"),
        Some(&admin_token),
        None,
        serde_json::json!({ "name": "Quarterly Reporting" }),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text);
    let project_id = created.json["project"]["project_id"]
        .as_str()
        .expect("project id");
    assert_eq!(created.json["project"]["owner_user_id"], developer_id);
    assert_eq!(created.json["project"]["name"], "Quarterly Reporting");

    let fetched = empty_request(
        router.clone(),
        "GET",
        &format!("/v1/projects/{project_id}"),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(fetched.status, StatusCode::OK, "{}", fetched.text);
    assert_eq!(fetched.json["project"]["project_id"], project_id);

    let renamed = json_request(
        router.clone(),
        "PATCH",
        &format!("/v1/projects/{project_id}"),
        Some(&admin_token),
        None,
        serde_json::json!({ "name": "Monthly Reporting" }),
    )
    .await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.text);
    assert_eq!(renamed.json["project"]["name"], "Monthly Reporting");
    assert!(
        renamed.json["project"]["project_path"]
            .as_str()
            .expect("project path")
            .ends_with("/analyst/projects/quarterly-reporting"),
        "{}",
        renamed.json["project"]["project_path"]
    );

    let listed_for_user = empty_request(
        router.clone(),
        "GET",
        &format!("/v1/projects?user_id={developer_id}"),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(
        listed_for_user.status,
        StatusCode::OK,
        "{}",
        listed_for_user.text
    );
    assert_eq!(
        listed_for_user.json["projects"][0]["project_id"],
        project_id
    );

    let deleted = empty_request(
        router.clone(),
        "DELETE",
        &format!("/v1/projects/{project_id}"),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.text);
    assert_eq!(deleted.json["project"]["project_id"], project_id);
    assert!(
        deleted.json["project"]["deleted_at"].is_string(),
        "{}",
        deleted.text
    );

    let after_delete = empty_request(
        router.clone(),
        "GET",
        &format!("/v1/projects/{project_id}"),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(after_delete.status, StatusCode::NOT_FOUND);

    let hidden_from_default_list = empty_request(
        router.clone(),
        "GET",
        &format!("/v1/projects?user_id={developer_id}"),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(
        hidden_from_default_list.status,
        StatusCode::OK,
        "{}",
        hidden_from_default_list.text
    );
    assert!(
        hidden_from_default_list.json["projects"]
            .as_array()
            .expect("default projects")
            .is_empty(),
        "{}",
        hidden_from_default_list.text
    );

    let archived = empty_request(
        router.clone(),
        "GET",
        &format!("/v1/projects?user_id={developer_id}&include_deleted=true"),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(archived.status, StatusCode::OK, "{}", archived.text);
    assert_eq!(archived.json["projects"][0]["project_id"], project_id);
    assert!(archived.json["projects"][0]["deleted_at"].is_string());

    let restored = empty_request(
        router.clone(),
        "POST",
        &format!("/v1/projects/{project_id}/restorations"),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(restored.status, StatusCode::OK, "{}", restored.text);
    assert!(restored.json["project"]["deleted_at"].is_null());

    let visible_after_restore = empty_request(
        router,
        "GET",
        &format!("/v1/projects?user_id={developer_id}"),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(
        visible_after_restore.json["projects"][0]["project_id"],
        project_id
    );
}

#[tokio::test]
async fn admin_pages_offer_dropdown_assignment_controls_instead_of_manual_ids() {
    let router = api::build_test_router();
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let bootstrap = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        None,
        serde_json::json!({
            "owner_email": "owner@example.com",
            "owner_password": "owner-password",
            "workspace_roots": [temp_dir.path()],
        }),
    )
    .await;
    assert_eq!(bootstrap.status, StatusCode::CREATED);
    let token = bootstrap.json["api_token"]
        .as_str()
        .expect("token")
        .to_string();

    let users_page = empty_request(router.clone(), "GET", "/admin/users", Some(&token), None).await;
    assert_eq!(users_page.status, StatusCode::OK);
    assert!(users_page.text.contains("Admin Console"));
    assert!(users_page.text.contains("Identity"));
    assert!(users_page.text.contains("Index"));
    assert!(users_page.text.contains("user-index"));
    assert!(users_page.text.contains("/admin/users/new"));
    assert!(users_page.text.contains("/admin/users/context-packs"));
    assert!(users_page.text.contains("/admin/users/projects"));
    assert!(!users_page.text.contains("<h2>Create User</h2>"));
    assert!(!users_page.text.contains("Refresh Users"));
    assert!(
        users_page
            .text
            .contains("Manage users from their action pages")
    );
    assert!(!users_page.text.contains("<label>Pack ID"));
    assert!(!users_page.text.contains("<label>User ID"));

    let user_pack_page = empty_request(
        router.clone(),
        "GET",
        "/admin/users/context-packs",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(user_pack_page.status, StatusCode::OK);
    assert!(user_pack_page.text.contains("Assign Context Packs"));
    assert!(user_pack_page.text.contains("select multiple"));
    assert!(user_pack_page.text.contains("pack-empty-state"));
    assert!(user_pack_page.text.contains("workspace-empty-state"));
    assert!(user_pack_page.text.contains("/admin/context-packs/new"));

    let projects_page = empty_request(
        router.clone(),
        "GET",
        "/admin/users/projects",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(projects_page.status, StatusCode::OK);
    assert!(projects_page.text.contains("User Projects"));
    assert!(projects_page.text.contains("project-user-select"));
    assert!(projects_page.text.contains("project-user-workspace-select"));
    assert!(projects_page.text.contains("project-index"));
    assert!(projects_page.text.contains("include_deleted=true"));
    assert!(projects_page.text.contains("restoreSelectedProject"));
    assert!(projects_page.text.contains("Archived projects"));
    assert!(
        projects_page
            .text
            .contains("/v1/projects/'+encodeURIComponent(projectId)+'/restorations")
    );
    assert!(projects_page.text.contains("/v1/user-workspaces"));
    assert!(projects_page.text.contains("/v1/projects?user_id="));
    assert!(
        projects_page
            .text
            .contains("/v1/projects/'+encodeURIComponent")
    );
    assert!(!projects_page.text.contains("<label>Project ID"));
    assert!(user_pack_page.text.contains("/admin/workspaces/register"));
    assert!(user_pack_page.text.contains("user-pack-select"));
    assert!(user_pack_page.text.contains("user-pack-assign-submit"));
    assert!(!user_pack_page.text.contains("<label>Pack ID"));

    let pack_create_page = empty_request(
        router.clone(),
        "GET",
        "/admin/context-packs/new",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(pack_create_page.status, StatusCode::OK);
    for filename in [
        "PACK.md",
        "CALIBRATION.md",
        "OPERATING-INSTRUCTIONS.md",
        "PROJECT-RULES.md",
        "WORKFLOWS.md",
        "HANDOFF.md",
        "VERIFICATION.md",
        "ESCALATION.md",
        "CONTEXT.md",
        "PROMPTS.md",
    ] {
        assert!(pack_create_page.text.contains(filename), "{filename}");
    }
    assert!(
        pack_create_page
            .text
            .contains("Context Packs are versioned operating packages")
    );
    assert!(pack_create_page.text.contains("They are not Codex skills"));
    assert!(
        pack_create_page
            .text
            .contains("Context Packs do not execute workflows")
    );
    assert!(pack_create_page.text.contains("isContextPackMarkdownFile"));
    assert!(pack_create_page.text.contains("CUSTOM-STANDARD.md"));

    let packs_page = empty_request(
        router.clone(),
        "GET",
        "/admin/context-packs",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(packs_page.status, StatusCode::OK);
    assert!(packs_page.text.contains("Admin Console"));
    assert!(packs_page.text.contains("Governed Context"));
    assert!(packs_page.text.contains("Context Pack Index"));
    assert!(packs_page.text.contains("versioned operating packages"));
    assert!(packs_page.text.contains("not Codex skills"));
    assert!(packs_page.text.contains("context-pack-index"));
    assert!(packs_page.text.contains("/admin/context-packs/new"));
    assert!(packs_page.text.contains("/admin/context-packs/assignments"));
    assert!(!packs_page.text.contains("<h2>Create Context Pack</h2>"));
    assert!(!packs_page.text.contains("Refresh Context Packs"));

    let pack_assignment_page = empty_request(
        router,
        "GET",
        "/admin/context-packs/assignments",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(pack_assignment_page.status, StatusCode::OK);
    assert!(pack_assignment_page.text.contains("Assign To Users"));
    assert!(
        pack_assignment_page
            .text
            .contains("context-pack-empty-state")
    );
    assert!(
        pack_assignment_page
            .text
            .contains("pack-workspace-empty-state")
    );
    assert!(
        pack_assignment_page
            .text
            .contains("/admin/context-packs/new")
    );
    assert!(
        pack_assignment_page
            .text
            .contains("/admin/workspaces/register")
    );
    assert!(pack_assignment_page.text.contains("pack-user-select"));
    assert!(pack_assignment_page.text.contains("pack-assign-submit"));
    assert!(pack_assignment_page.text.contains("Remove Assignment"));
    assert!(!pack_assignment_page.text.contains("<label>User ID"));
    assert!(!pack_assignment_page.text.contains("<label>Workspace ID"));
}

#[tokio::test]
async fn admin_forms_explain_empty_prerequisites_and_link_to_create_resources() {
    let router = api::build_test_router();
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let bootstrap = json_request(
        router.clone(),
        "POST",
        "/v1/setup/enterprise",
        None,
        None,
        serde_json::json!({
            "owner_email": "owner@example.com",
            "owner_password": "owner-password",
            "workspace_roots": [temp_dir.path()],
        }),
    )
    .await;
    assert_eq!(bootstrap.status, StatusCode::CREATED);
    let token = bootstrap.json["api_token"]
        .as_str()
        .expect("token")
        .to_string();

    let context_pack_assignment = empty_request(
        router.clone(),
        "GET",
        "/admin/context-packs/assignments",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(context_pack_assignment.status, StatusCode::OK);
    assert!(
        context_pack_assignment
            .text
            .contains("No context packs exist yet")
    );
    assert!(
        context_pack_assignment
            .text
            .contains("Create a Context Pack before assigning it")
    );
    assert!(
        context_pack_assignment
            .text
            .contains("No registered workspaces are available")
    );
    assert!(
        context_pack_assignment
            .text
            .contains("disabled = packs.length === 0 || workspaces.length === 0")
    );

    let users_context_packs = empty_request(
        router,
        "GET",
        "/admin/users/context-packs",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(users_context_packs.status, StatusCode::OK);
    assert!(
        users_context_packs
            .text
            .contains("No context packs exist yet")
    );
    assert!(
        users_context_packs
            .text
            .contains("No registered workspaces are available")
    );
    assert!(users_context_packs.text.contains(
        "disabled = packs.length === 0 || users.length === 0 || workspaces.length === 0"
    ));
}

struct JsonTestResponse {
    status: StatusCode,
    location: Option<String>,
    json: serde_json::Value,
    text: String,
}

async fn json_request(
    router: axum::Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    trace_id: Option<&str>,
    body: serde_json::Value,
) -> JsonTestResponse {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    if let Some(trace_id) = trace_id {
        request = request.header("x-trace-id", trace_id);
    }
    let response = router
        .oneshot(
            request
                .body(Body::from(body.to_string()))
                .expect("json request"),
        )
        .await
        .expect("json response");
    json_response(response).await
}

async fn empty_request(
    router: axum::Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    trace_id: Option<&str>,
) -> JsonTestResponse {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    if let Some(trace_id) = trace_id {
        request = request.header("x-trace-id", trace_id);
    }
    let response = router
        .oneshot(request.body(Body::empty()).expect("empty request"))
        .await
        .expect("empty response");
    json_response(response).await
}

async fn json_response(response: axum::response::Response) -> JsonTestResponse {
    let status = response.status();
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let text = String::from_utf8(body.to_vec()).expect("utf8");
    let json = if body.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({ "body": text }))
    };
    JsonTestResponse {
        status,
        location,
        json,
        text,
    }
}

async fn create_and_login_user(
    router: axum::Router,
    admin_token: &str,
    email: &str,
    password: &str,
    role: &str,
) -> (String, String) {
    let created = json_request(
        router.clone(),
        "POST",
        "/v1/users",
        Some(admin_token),
        None,
        serde_json::json!({
            "email": email,
            "password": password,
            "role": role,
        }),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);

    let login = json_request(
        router,
        "POST",
        "/v1/auth/login",
        None,
        None,
        serde_json::json!({
            "email": email,
            "password": password,
        }),
    )
    .await;
    assert_eq!(login.status, StatusCode::OK);
    (
        login.json["api_token"]
            .as_str()
            .expect("api token")
            .to_string(),
        login.json["user_id"].as_str().expect("user id").to_string(),
    )
}

async fn create_and_login_user_with_workspace(
    router: axum::Router,
    admin_token: &str,
    email: &str,
    password: &str,
    role: &str,
    workspace_root: &std::path::Path,
) -> (String, String) {
    let created = json_request(
        router.clone(),
        "POST",
        "/v1/users",
        Some(admin_token),
        None,
        serde_json::json!({
            "email": email,
            "password": password,
            "role": role,
            "workspace_roots": [workspace_root.to_string_lossy()],
        }),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text);

    let login = json_request(
        router,
        "POST",
        "/v1/auth/login",
        None,
        None,
        serde_json::json!({
            "email": email,
            "password": password,
        }),
    )
    .await;
    assert_eq!(login.status, StatusCode::OK);
    (
        login.json["api_token"]
            .as_str()
            .expect("api token")
            .to_string(),
        login.json["user_id"].as_str().expect("user id").to_string(),
    )
}

async fn create_minimal_context_pack(
    router: axum::Router,
    token: &str,
    trace_id: &str,
    name: &str,
) -> String {
    let response = json_request(
        router,
        "POST",
        "/v1/context-packs",
        Some(token),
        Some(trace_id),
        serde_json::json!({
            "name": name,
            "documents": [
                {
                    "filename": "PACK.md",
                    "content": format!("name: {name}\nversion: 1\nload_order:\n- CONTEXT.md\n")
                },
                {
                    "filename": "CONTEXT.md",
                    "content": "Follow repository instructions."
                }
            ]
        }),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED);
    response.json["pack"]["pack_id"]
        .as_str()
        .expect("pack id")
        .to_string()
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
