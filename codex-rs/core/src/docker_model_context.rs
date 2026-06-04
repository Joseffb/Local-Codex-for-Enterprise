use std::process::Output;
use std::time::Duration;

use codex_model_provider_info::DOCKER_MODEL_GATEWAY_DEFAULT_BASE_URL;
use codex_model_provider_info::DOCKER_MODEL_RUNNER_DEFAULT_BASE_URL;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result;
use serde_json::Value;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::debug;
use tracing::info;
use tracing::warn;

const DOCKER_CONTEXT_VARIANT_NAMESPACE: &str = "codex-for-docker";
const DOCKER_MODEL_COMMAND_TIMEOUT: Duration = Duration::from_secs(600);
const DOCKER_MODEL_RUNNER_CONTAINER_BASE_URL: &str = "http://host.docker.internal:12434/engines/v1";
const DOCKER_MODEL_GATEWAY_CONTAINER_BASE_URL: &str = "http://host.docker.internal:4000/v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DockerModelContextPlan {
    UseOriginal,
    UseExistingVariant(String),
    CreateVariant { model: String, context_size: u64 },
}

pub(crate) async fn resolve_model_for_native_context(model: &str) -> Result<String> {
    let Some(source_inspect) = inspect_model(model).await? else {
        warn!(
            model,
            "could not inspect Docker model context metadata; using selected model"
        );
        return Ok(model.to_string());
    };

    let Some(native_context) = native_context_length(&source_inspect) else {
        debug!(
            model,
            "Docker model inspect did not include native context metadata; using selected model"
        );
        return Ok(model.to_string());
    };

    if packaged_context_size(&source_inspect).is_some_and(|packaged| packaged >= native_context) {
        return Ok(model.to_string());
    }

    let target_model = variant_model_name(model, native_context);
    let variant_inspect = inspect_model(&target_model).await?;
    match plan_context_matched_model(model, &source_inspect, variant_inspect.as_ref()) {
        DockerModelContextPlan::UseOriginal => Ok(model.to_string()),
        DockerModelContextPlan::UseExistingVariant(model) => Ok(model),
        DockerModelContextPlan::CreateVariant {
            model: variant_model,
            context_size,
        } => {
            create_context_variant(model, &variant_model, context_size).await?;
            Ok(variant_model)
        }
    }
}

pub(crate) fn is_docker_model_context_base_url(base_url: &str) -> bool {
    let base_url = base_url.trim_end_matches('/');
    base_url == DOCKER_MODEL_RUNNER_DEFAULT_BASE_URL
        || base_url == DOCKER_MODEL_GATEWAY_DEFAULT_BASE_URL
        || base_url == DOCKER_MODEL_RUNNER_CONTAINER_BASE_URL
        || base_url == DOCKER_MODEL_GATEWAY_CONTAINER_BASE_URL
}

pub(crate) fn plan_context_matched_model(
    model: &str,
    source_inspect: &Value,
    variant_inspect: Option<&Value>,
) -> DockerModelContextPlan {
    let Some(native_context) = native_context_length(source_inspect) else {
        return DockerModelContextPlan::UseOriginal;
    };
    if packaged_context_size(source_inspect).is_some_and(|packaged| packaged >= native_context) {
        return DockerModelContextPlan::UseOriginal;
    }

    let variant_model = variant_model_name(model, native_context);
    if let Some(variant_inspect) = variant_inspect
        && packaged_context_size(variant_inspect) == Some(native_context)
    {
        return DockerModelContextPlan::UseExistingVariant(variant_model);
    }

    DockerModelContextPlan::CreateVariant {
        model: variant_model,
        context_size: native_context,
    }
}

pub(crate) fn native_context_length(inspect: &Value) -> Option<u64> {
    let gguf = inspect.pointer("/config/gguf").and_then(Value::as_object)?;

    gguf.iter()
        .filter(|(key, _)| key.ends_with(".context_length"))
        .filter_map(|(_, value)| value_as_u64(value))
        .max()
}

pub(crate) fn variant_model_name(model: &str, context_size: u64) -> String {
    let mut previous_dash = false;
    let slug = model
        .to_ascii_lowercase()
        .chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                previous_dash = false;
                Some(ch)
            } else if previous_dash {
                None
            } else {
                previous_dash = true;
                Some('-')
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let slug = if slug.is_empty() { "model" } else { &slug };
    format!("{DOCKER_CONTEXT_VARIANT_NAMESPACE}/{slug}:ctx{context_size}")
}

fn packaged_context_size(inspect: &Value) -> Option<u64> {
    inspect
        .pointer("/config/context_size")
        .and_then(value_as_u64)
}

fn value_as_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

async fn inspect_model(model: &str) -> Result<Option<Value>> {
    let output = run_docker_model_command(vec!["inspect".to_string(), model.to_string()]).await?;
    if !output.status.success() {
        debug!(
            model,
            stderr = %command_stderr(&output),
            "docker model inspect failed"
        );
        return Ok(None);
    }

    serde_json::from_slice(&output.stdout)
        .map(Some)
        .map_err(CodexErr::from)
}

async fn create_context_variant(
    source_model: &str,
    target_model: &str,
    context_size: u64,
) -> Result<()> {
    info!(
        source_model,
        target_model, context_size, "creating Docker model context-size variant"
    );
    let output = run_docker_model_command(vec![
        "package".to_string(),
        "--from".to_string(),
        source_model.to_string(),
        "--context-size".to_string(),
        context_size.to_string(),
        target_model.to_string(),
    ])
    .await?;

    if output.status.success() {
        Ok(())
    } else {
        Err(CodexErr::Fatal(format!(
            "failed to create Docker model context variant `{target_model}` from `{source_model}`: {}",
            command_stderr(&output)
        )))
    }
}

async fn run_docker_model_command(args: Vec<String>) -> Result<Output> {
    let mut command = Command::new("docker");
    command.arg("model").args(args);
    timeout(DOCKER_MODEL_COMMAND_TIMEOUT, command.output())
        .await
        .map_err(|_| {
            CodexErr::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "docker model command timed out",
            ))
        })?
        .map_err(CodexErr::from)
}

fn command_stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

#[cfg(test)]
#[path = "docker_model_context_tests.rs"]
mod tests;
