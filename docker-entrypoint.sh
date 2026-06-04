#!/bin/sh
set -eu

if [ -n "${CODEX_DOCKER_MODEL_CONTEXT_HOST:-}" ] && command -v docker >/dev/null 2>&1; then
    context_name="${CODEX_DOCKER_MODEL_CONTEXT_NAME:-codex-host}"

    if ! docker model context inspect "$context_name" >/dev/null 2>&1; then
        docker model context create "$context_name" \
            --host "$CODEX_DOCKER_MODEL_CONTEXT_HOST" >/dev/null 2>&1 || true
    fi

    docker model context use "$context_name" >/dev/null 2>&1 || true
fi

if [ "${CODEX_CONTAINER_DEFAULT_PROVIDER_CONFIG:-1}" != "0" ]; then
    provider_id="${CODEX_MODEL_PROVIDER_ID:-docker-model-runner-container}"
    provider_name="${CODEX_MODEL_PROVIDER_NAME:-Docker Model Runner}"
    provider_base_url="${CODEX_MODEL_PROVIDER_BASE_URL:-http://host.docker.internal:12434/engines/v1}"

    set -- \
        -c "model_provider=\"$provider_id\"" \
        -c "model_providers.$provider_id.name=\"$provider_name\"" \
        -c "model_providers.$provider_id.base_url=\"$provider_base_url\"" \
        -c "model_providers.$provider_id.wire_api=\"chat_completions\"" \
        -c "model_providers.$provider_id.requires_openai_auth=false" \
        "$@"
fi

exec codex "$@"
