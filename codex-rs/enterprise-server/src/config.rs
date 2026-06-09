use serde::Deserialize;
use serde::Serialize;
use std::env;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerMode {
    Enterprise,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseConfig {
    pub mode: ServerMode,
    pub default_model_provider: String,
    pub default_model: String,
    pub bind_addr: String,
    pub database_url: Option<String>,
    pub worker_command: String,
    pub worker_args: Vec<String>,
    pub worker_socket_dir: String,
    pub worker_log_dir: String,
    pub handoff_token_secret: String,
    pub handoff_token_ttl_seconds: u64,
    pub default_workspace_root: Option<String>,
    pub output_artifact_root: String,
}

impl Default for EnterpriseConfig {
    fn default() -> Self {
        Self {
            mode: ServerMode::Enterprise,
            default_model_provider: "docker-model-runner".to_string(),
            default_model: "ai/qwen3-coder".to_string(),
            bind_addr: "127.0.0.1:8787".to_string(),
            database_url: None,
            worker_command: "codex-app-server".to_string(),
            worker_args: vec!["--listen".to_string(), "unix://{socket_path}".to_string()],
            worker_socket_dir: "/tmp/local-codex-enterprise/workers".to_string(),
            worker_log_dir: "/tmp/local-codex-enterprise/logs".to_string(),
            handoff_token_secret: "local-codex-enterprise-dev-handoff-secret".to_string(),
            handoff_token_ttl_seconds: 120,
            default_workspace_root: env::var("LOCAL_CODEX_ENTERPRISE_DEFAULT_WORKSPACE_ROOT").ok(),
            output_artifact_root: env::var("LOCAL_CODEX_ENTERPRISE_OUTPUT_ARTIFACT_ROOT")
                .unwrap_or_else(|_| "/tmp/local-codex-enterprise/outputs".to_string()),
        }
    }
}

impl EnterpriseConfig {
    pub fn from_runtime_parts(
        bind_addr: impl Into<String>,
        database_url: impl Into<Option<String>>,
    ) -> Self {
        Self {
            bind_addr: bind_addr.into(),
            database_url: database_url.into(),
            ..Self::default()
        }
    }

    pub fn from_env() -> Self {
        Self::from_runtime_parts(
            env::var("LOCAL_CODEX_ENTERPRISE_BIND")
                .unwrap_or_else(|_| "127.0.0.1:8787".to_string()),
            env::var("DATABASE_URL").ok(),
        )
    }
}
