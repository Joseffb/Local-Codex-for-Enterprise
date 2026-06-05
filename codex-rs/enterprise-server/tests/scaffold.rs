use codex_enterprise_server::api;
use codex_enterprise_server::auth;
use codex_enterprise_server::config::EnterpriseConfig;
use codex_enterprise_server::config::ServerMode;
use codex_enterprise_server::rbac;
use codex_enterprise_server::rbac::EnterpriseAction;
use codex_enterprise_server::rbac::EnterpriseRole;
use codex_enterprise_server::setup::BootstrapReceipt;
use codex_enterprise_server::setup::SetupMode;
use codex_enterprise_server::worker::WorkerState;
use codex_enterprise_server::worker::WorkerSupervisor;
use codex_enterprise_server::workspace::WorkspacePolicy;
use std::time::Duration;

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

    assert_ne!(hash, "correct horse battery staple");
    assert!(auth::verify_password("correct horse battery staple", &hash).expect("verify password"));
    assert!(!auth::verify_password("wrong", &hash).expect("reject wrong password"));
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
