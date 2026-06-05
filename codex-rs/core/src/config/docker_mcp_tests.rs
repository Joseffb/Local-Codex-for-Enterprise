use super::*;
use codex_config::types::McpServerConfig;
use codex_config::types::McpServerTransportConfig;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;

#[test]
fn accept_adds_docker_mcp_server_and_persists_decision() {
    let edits = docker_mcp_setup_edits(&BTreeMap::new(), DockerMcpSetupChoice::Accept);

    let expected_server = docker_mcp_server_config();
    assert_eq!(
        edits,
        DockerMcpSetupEdits {
            docker_mcp_auto_configure: Some(true),
            mcp_servers: Some(BTreeMap::from([("docker".to_string(), expected_server)])),
        }
    );
}

#[test]
fn decline_persists_opt_out_without_adding_server() {
    let edits = docker_mcp_setup_edits(&BTreeMap::new(), DockerMcpSetupChoice::Decline);

    assert_eq!(
        edits,
        DockerMcpSetupEdits {
            docker_mcp_auto_configure: Some(false),
            mcp_servers: None,
        }
    );
}

#[test]
fn accept_preserves_existing_docker_mcp_server() {
    let existing = McpServerConfig {
        transport: McpServerTransportConfig::Stdio {
            command: "custom-docker".to_string(),
            args: vec!["mcp".to_string()],
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
    };
    let servers = BTreeMap::from([("docker".to_string(), existing)]);

    let edits = docker_mcp_setup_edits(&servers, DockerMcpSetupChoice::Accept);

    assert_eq!(
        edits,
        DockerMcpSetupEdits {
            docker_mcp_auto_configure: Some(true),
            mcp_servers: None,
        }
    );
}

#[test]
fn should_prompt_only_when_available_unconfigured_and_no_decision() {
    assert!(should_prompt_docker_mcp_setup(
        /*docker_mcp_auto_configure*/ None,
        &BTreeMap::new(),
        /*toolkit_available*/ true,
    ));
    assert!(!should_prompt_docker_mcp_setup(
        Some(false),
        &BTreeMap::new(),
        /*toolkit_available*/ true,
    ));
    assert!(!should_prompt_docker_mcp_setup(
        /*docker_mcp_auto_configure*/ None,
        &BTreeMap::from([("docker".to_string(), docker_mcp_server_config())]),
        /*toolkit_available*/ true,
    ));
    assert!(!should_prompt_docker_mcp_setup(
        /*docker_mcp_auto_configure*/ None,
        &BTreeMap::new(),
        /*toolkit_available*/ false,
    ));
}

#[test]
fn docker_mcp_help_requires_gateway_command() {
    let generic_docker_help = r#"
Usage:  docker [OPTIONS] COMMAND

Management Commands:
  model*      Docker Model Runner
"#;
    let docker_mcp_help = r#"
Usage:  docker mcp COMMAND

Commands:
  gateway     Run the Docker MCP gateway
"#;

    assert!(!docker_mcp_help_has_gateway(generic_docker_help));
    assert!(docker_mcp_help_has_gateway(docker_mcp_help));
}
