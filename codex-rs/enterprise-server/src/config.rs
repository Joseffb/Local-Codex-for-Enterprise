use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerMode {
    Enterprise,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseConfig {
    pub mode: ServerMode,
    pub default_model_provider: String,
    pub default_model: String,
}

impl Default for EnterpriseConfig {
    fn default() -> Self {
        Self {
            mode: ServerMode::Enterprise,
            default_model_provider: "docker-model-runner".to_string(),
            default_model: "ai/qwen3-coder".to_string(),
        }
    }
}
