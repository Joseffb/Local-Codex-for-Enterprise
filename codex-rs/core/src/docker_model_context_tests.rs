use pretty_assertions::assert_eq;
use serde_json::json;

use super::DockerModelContextPlan;
use super::is_docker_model_context_base_url;
use super::native_context_length;
use super::plan_context_matched_model;
use super::variant_model_name;

#[test]
fn native_context_length_uses_any_gguf_architecture_key() {
    let inspect = json!({
        "config": {
            "context_size": 4096,
            "gguf": {
                "qwen3moe.context_length": "262144",
                "general.name": "Qwen3 Coder"
            }
        }
    });

    assert_eq!(native_context_length(&inspect), Some(262_144));
}

#[test]
fn plan_uses_original_when_packaged_context_matches_native_context() {
    let inspect = json!({
        "config": {
            "context_size": 32768,
            "gguf": {
                "llama.context_length": "32768"
            }
        }
    });

    assert_eq!(
        plan_context_matched_model("ai/example-model", &inspect, None),
        DockerModelContextPlan::UseOriginal
    );
}

#[test]
fn plan_creates_variant_when_packaged_context_is_smaller_than_native_context() {
    let inspect = json!({
        "config": {
            "context_size": 4096,
            "gguf": {
                "qwen3moe.context_length": "262144"
            }
        }
    });

    assert_eq!(
        plan_context_matched_model("ai/qwen3-coder:latest", &inspect, None),
        DockerModelContextPlan::CreateVariant {
            model: "codex-for-docker/ai-qwen3-coder-latest:ctx262144".to_string(),
            context_size: 262_144,
        }
    );
}

#[test]
fn plan_creates_variant_for_arbitrary_selected_model() {
    let inspect = json!({
        "config": {
            "context_size": 8192,
            "gguf": {
                "llama.context_length": "131072"
            }
        }
    });

    assert_eq!(
        plan_context_matched_model("registry.example.com/Team/Custom-Coder:q8", &inspect, None),
        DockerModelContextPlan::CreateVariant {
            model: "codex-for-docker/registry.example.com-team-custom-coder-q8:ctx131072"
                .to_string(),
            context_size: 131_072,
        }
    );
}

#[test]
fn plan_reuses_existing_matching_variant() {
    let source = json!({
        "config": {
            "context_size": 4096,
            "gguf": {
                "qwen3moe.context_length": "262144"
            }
        }
    });
    let variant = json!({
        "config": {
            "context_size": 262144,
            "gguf": {
                "qwen3moe.context_length": "262144"
            }
        }
    });

    assert_eq!(
        plan_context_matched_model("ai/qwen3-coder:latest", &source, Some(&variant)),
        DockerModelContextPlan::UseExistingVariant(
            "codex-for-docker/ai-qwen3-coder-latest:ctx262144".to_string()
        )
    );
}

#[test]
fn variant_model_name_is_stable_for_registry_qualified_models() {
    assert_eq!(
        variant_model_name("registry.example.com/Org/Model.Name:Q4_K_M", 131_072),
        "codex-for-docker/registry.example.com-org-model.name-q4_k_m:ctx131072"
    );
}

#[test]
fn detects_docker_model_context_base_urls_with_optional_trailing_slash() {
    assert!(is_docker_model_context_base_url(
        "http://localhost:12434/engines/v1/"
    ));
    assert!(is_docker_model_context_base_url("http://localhost:4000/v1"));
    assert!(is_docker_model_context_base_url(
        "http://host.docker.internal:12434/engines/v1/"
    ));
    assert!(is_docker_model_context_base_url(
        "http://host.docker.internal:4000/v1"
    ));
    assert!(!is_docker_model_context_base_url(
        "http://localhost:8080/v1"
    ));
}
