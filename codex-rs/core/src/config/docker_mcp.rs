use codex_config::types::McpServerConfig;
use codex_config::types::McpServerTransportConfig;
use std::collections::BTreeMap;
use std::process::Command;

pub const DOCKER_MCP_SERVER_NAME: &str = "docker";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockerMcpSetupChoice {
    Accept,
    Decline,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DockerMcpSetupEdits {
    pub docker_mcp_auto_configure: Option<bool>,
    pub mcp_servers: Option<BTreeMap<String, McpServerConfig>>,
}

pub fn should_prompt_docker_mcp_setup(
    docker_mcp_auto_configure: Option<bool>,
    mcp_servers: &BTreeMap<String, McpServerConfig>,
    toolkit_available: bool,
) -> bool {
    toolkit_available
        && docker_mcp_auto_configure.is_none()
        && !mcp_servers.contains_key(DOCKER_MCP_SERVER_NAME)
}

pub fn docker_mcp_setup_edits(
    current_servers: &BTreeMap<String, McpServerConfig>,
    choice: DockerMcpSetupChoice,
) -> DockerMcpSetupEdits {
    match choice {
        DockerMcpSetupChoice::Decline => DockerMcpSetupEdits {
            docker_mcp_auto_configure: Some(false),
            mcp_servers: None,
        },
        DockerMcpSetupChoice::Accept => {
            let mut mcp_servers = None;
            if !current_servers.contains_key(DOCKER_MCP_SERVER_NAME) {
                let mut servers = current_servers.clone();
                servers.insert(
                    DOCKER_MCP_SERVER_NAME.to_string(),
                    docker_mcp_server_config(),
                );
                mcp_servers = Some(servers);
            }

            DockerMcpSetupEdits {
                docker_mcp_auto_configure: Some(true),
                mcp_servers,
            }
        }
    }
}

pub fn docker_mcp_server_config() -> McpServerConfig {
    McpServerConfig {
        transport: McpServerTransportConfig::Stdio {
            command: "docker".to_string(),
            args: vec!["mcp".to_string(), "gateway".to_string(), "run".to_string()],
            env: None,
            env_vars: Vec::new(),
            cwd: None,
        },
        environment_id: "local".to_string(),
        enabled: true,
        required: false,
        supports_parallel_tool_calls: false,
        disabled_reason: None,
        startup_timeout_sec: None,
        tool_timeout_sec: None,
        default_tools_approval_mode: None,
        enabled_tools: None,
        disabled_tools: None,
        scopes: None,
        oauth: None,
        oauth_resource: None,
        tools: Default::default(),
    }
}

pub fn docker_mcp_toolkit_available() -> bool {
    Command::new("docker")
        .args(["mcp", "--help"])
        .output()
        .is_ok_and(|output| {
            output.status.success()
                && docker_mcp_help_has_gateway(&format!(
                    "{}\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                ))
        })
}

pub(crate) fn docker_mcp_help_has_gateway(help: &str) -> bool {
    help.lines().any(|line| {
        line.trim_start()
            .split_whitespace()
            .next()
            .is_some_and(|command| command == "gateway")
    })
}

#[cfg(test)]
#[path = "docker_mcp_tests.rs"]
mod tests;
