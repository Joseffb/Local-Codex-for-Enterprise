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
}

impl Default for EnterpriseConfig {
    fn default() -> Self {
        Self {
            mode: ServerMode::Enterprise,
            default_model_provider: "docker-model-runner".to_string(),
            default_model: "ai/qwen3-coder".to_string(),
            bind_addr: "127.0.0.1:8787".to_string(),
            database_url: None,
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
