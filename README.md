# Local Codex for Enterprise

Local Codex for Enterprise is an experimental private enterprise extension of Local Codex for Docker. It is not affiliated with, endorsed by, or supported by OpenAI.

This repository starts from the Local Codex for Docker codebase and will add self-hosted enterprise controls for multi-user local Codex deployments: authentication, Postgres-backed state, RBAC, workspace allowlisting, worker supervision, audit trails, and terminal remote access. The current baseline still retains the single-user Docker Model Runner behavior while the enterprise server is under development.

## Status

- Local Docker Model Runner execution has been smoke-tested.
- Large-prompt handling has been smoke-tested with a Docker Model context-size variant.
- Container packaging is present. The optimized `release` image path and Compose release override have been validated against a responsive local Docker daemon; Compose still defaults to the faster `dev` build profile for iteration.
- Enterprise server scaffold is now runnable as `codex-enterprise-server`.
- Enterprise server currently supports health/config endpoints, first-run owner setup, password login with opaque API-token issuance, token-authenticated worker start/list/stop, role-enforced worker routes, HTTPS-only repo clone onboarding into allowlisted roots, short-lived single-use worker handoff tokens, an initial websocket tunnel to supervised worker Unix sockets, Postgres migrations, workspace allowlist enforcement for worker launch, supervised worker process launch, initial audit events for auth/RBAC/workspace/worker/handoff decisions, Argon2 password hashing, Casbin RBAC policy checks, and Utoipa OpenAPI generation.
- Enterprise server does not yet persist chat/thread history, provide admin user/role management APIs, reconcile workers after server restart, or provide audit query/export APIs.

## Enterprise Server Smoke

Run the enterprise control plane against Postgres:

```sh
DATABASE_URL="postgres://codex:codex@127.0.0.1:5432/codex_enterprise" \
  cargo run -p codex-enterprise-server -- \
  --bind-addr 127.0.0.1:8787
```

By default, workers launch:

```sh
codex-app-server --listen unix://{socket_path}
```

For smoke tests, override the worker command:

```sh
--worker-command /bin/sh --worker-arg=-c --worker-arg 'sleep 30'
```

First-run setup:

```sh
curl -X POST http://127.0.0.1:8787/v1/setup/enterprise \
  -H 'content-type: application/json' \
  -d '{
    "owner_email": "owner@example.com",
    "owner_password": "change-me",
    "workspace_roots": ["/srv/workspaces"]
  }'
```

Workspace roots must exist on the enterprise server host. Worker launch
canonicalizes the requested workspace path and rejects paths outside the
registered roots.

The setup response returns the owner API token once. Use it with:

```sh
curl http://127.0.0.1:8787/v1/workers \
  -H "authorization: Bearer $LOCAL_CODEX_ENTERPRISE_TOKEN"
```

After bootstrap, sign in with the owner password to issue a fresh API token:

```sh
curl -X POST http://127.0.0.1:8787/v1/auth/login \
  -H 'content-type: application/json' \
  -d '{
    "email": "owner@example.com",
    "password": "change-me"
  }'
```

Clone a repository into an allowlisted workspace root:

```sh
curl -X POST http://127.0.0.1:8787/v1/workspaces/clone \
  -H "authorization: Bearer $LOCAL_CODEX_ENTERPRISE_TOKEN" \
  -H 'content-type: application/json' \
  -d '{
    "repo_url": "https://example.com/org/repo.git",
    "workspace_root": "/srv/workspaces",
    "destination_name": "repo"
  }'
```

Clone intake is intentionally narrow in v1: HTTPS only, no embedded
credentials, no localhost/private/link-local targets, and a destination name
directly under a registered workspace root.

Start and stop a worker:

```sh
curl -X POST http://127.0.0.1:8787/v1/workers \
  -H "authorization: Bearer $LOCAL_CODEX_ENTERPRISE_TOKEN" \
  -H 'content-type: application/json' \
  -d '{
    "workspace_path": "/srv/workspaces/project-a",
    "session_id": "session-1"
  }'

curl -X DELETE http://127.0.0.1:8787/v1/workers/$WORKER_ID \
  -H "authorization: Bearer $LOCAL_CODEX_ENTERPRISE_TOKEN"
```

Issue and consume a worker handoff token:

```sh
curl -X POST http://127.0.0.1:8787/v1/workers/$WORKER_ID/handoff \
  -H "authorization: Bearer $LOCAL_CODEX_ENTERPRISE_TOKEN"

curl -X POST http://127.0.0.1:8787/v1/worker-handoffs/$JTI/consume \
  -H 'content-type: application/json' \
  -d "{\"handoff_token\":\"$HANDOFF_TOKEN\"}"
```

Handoff tokens are purpose-bound to a worker, owner user, workspace, and
session. They are short-lived and single-use. This is the control-plane contract
that the remote TUI/websocket broker consumes before tunneling frames to the
worker socket.

Connect to a worker websocket tunnel:

```text
GET /v1/workers/$WORKER_ID/rpc?handoff_token=$HANDOFF_TOKEN
```

The tunnel relays websocket frames to the worker's private
`codex-app-server --listen unix://...` socket. It keeps the app-server JSON-RPC
protocol opaque to the enterprise control plane.

## Local-Only Contract

- No OpenAI API key is required by default.
- No outbound model calls are made by default.
- There is no automatic cloud fallback.
- Cloud providers remain available only when explicitly configured or selected.
- Docker/local execution remains the default behavior.

## Defaults

The bootstrap default config is:

```toml
model_provider = "docker-model-runner"
model = "ai/qwen3-coder"
```

`ai/qwen3-coder` is only the starter model. Change `model` through the normal Codex config paths to use newer Docker Models as they become available.

Built-in local providers:

- `docker-model-runner`: `http://localhost:12434/engines/v1`
- `docker-model-gateway`: `http://localhost:4000/v1`

Both use `wire_api = "chat_completions"`. The legacy `wire_api = "chat"` value remains rejected.

## Docker MCP Toolkit

On first interactive startup, Codex for Docker checks for Docker MCP Toolkit. If it is available and no `docker` MCP server is already configured, it prompts:

```text
Docker MCP Toolkit detected. Configure automatically? [Y/n]
```

Accepting persists:

```toml
[mcp_servers.docker]
command = "docker"
args = ["mcp", "gateway", "run"]
```

Declining persists:

```toml
docker_mcp_auto_configure = false
```

Existing user-defined `docker` MCP servers are preserved.

## Runtime Order

Build and validate locally first:

1. Enable Docker Model Runner.
2. Pull the bootstrap model:

   ```sh
   docker model pull ai/qwen3-coder
   ```

3. Run Codex for Docker locally against Docker Model Runner.
4. Verify a coding-agent turn and Docker MCP tool discovery.

For large prompts, Codex for Docker inspects the selected Docker Model and creates/reuses a `codex-for-docker/...:ctxN` variant with the model's native context size when Docker exposes that metadata.

## Container Runtime

The v1 container image packages the Codex for Docker CLI/runtime and a Docker CLI. It does not start or bundle Docker Model Runner, Docker Model Gateway, or a separate Docker MCP Gateway service. Those stay on the host, or must otherwise be reachable from inside the container.

Build the image:

```sh
docker build -t codex-for-docker:local .
```

The default release container build still uses Cargo's release profile, but overrides the repo's fat-LTO defaults with Docker-friendly settings:

```text
CARGO_PROFILE_RELEASE_LTO=thin
CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
```

This keeps the image on an optimized release build while avoiding Docker Desktop memory failures during Rust's final link step. On machines with more builder memory, you can override those build args.

For a faster local smoke build, use:

```sh
docker build --build-arg BUILD_PROFILE=dev -t codex-for-docker:dev .
```

Run with Compose from this repository:

```sh
docker compose run --build --rm codex
```

Run from any project directory by pointing Docker Compose at this repo's Compose file:

```sh
docker compose -f /path/to/Local-Codex-for-Docker/compose.yaml run --build --rm codex
```

By default, Compose mounts the shell's current working directory at `/workspace` and uses the faster `dev` build profile. To launch Codex for Docker against another codebase explicitly, point `CODEX_WORKSPACE` at that folder:

```sh
CODEX_WORKSPACE="/path/to/your/project" docker compose run --rm codex
```

Run from the project root when possible. If your shell is inside a subdirectory, set `CODEX_WORKSPACE` to the repository root so Codex can see the project metadata:

```sh
CODEX_WORKSPACE="$(git rev-parse --show-toplevel)" docker compose run --rm codex
```

On Linux Docker Engine, use the Linux socket path:

```sh
CODEX_WORKSPACE="$PWD" DOCKER_HOST_SOCKET=/var/run/docker.sock docker compose run --rm codex
```

To force the optimized release build through Compose:

```sh
CODEX_BUILD_PROFILE=release docker compose build codex
```

Release build tuning can also be overridden:

```sh
CODEX_BUILD_PROFILE=release \
CODEX_CARGO_PROFILE_RELEASE_LTO=fat \
CODEX_CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
docker compose build codex
```

The Compose example:

- Mounts `CODEX_WORKSPACE` at `/workspace`, defaulting to the shell's current working directory.
- Persists container Codex state in the `codex-home` volume.
- Mounts the host Docker socket at `/docker.sock` so Docker CLI commands can talk to the host Docker engine. On Docker Desktop, this defaults to `${HOME}/.docker/run/docker.sock`; on Linux, run with `DOCKER_HOST_SOCKET=/var/run/docker.sock`.
- Points the container provider at `http://host.docker.internal:12434/engines/v1`.
- Installs the Docker Model CLI plugin and configures a container-local Docker Model context for `http://host.docker.internal:12434`, so dynamic context matching can call `docker model inspect` and `docker model package` from inside the container without trying to start a second standalone Model Runner.
- Injects the Docker provider config in the container entrypoint so normal Codex arguments still work, for example `docker compose run --rm codex exec "summarize this repo"`.
- Defaults Codex's inner sandbox to `danger-full-access` because Docker is the outer sandbox boundary. Many Docker runtimes do not allow an unprivileged container process to create the nested Linux namespaces that bubblewrap needs. To opt back into nested Codex sandboxing in a privileged/container-runtime-specific setup, set `CODEX_CONTAINER_SANDBOX_MODE=workspace-write`; set it to an empty value to skip the container entrypoint sandbox override entirely.

To use Docker Model Gateway instead, change the Compose provider URL to `http://host.docker.internal:4000/v1`.

To use a different Docker Model, set `model` through normal Codex config, or add another Compose `-c` override such as:

```yaml
- -c
- 'model="ai/your-model"'
```

Inside the container, dynamic context matching requires Docker socket access plus the Docker Model CLI plugin, which this image installs from Docker's Debian package repository.

Docker MCP Toolkit is different: recent Docker Desktop installs the `docker mcp` CLI plugin on the host, while Docker Engine/Linux users may need to install the MCP Gateway plugin separately. The v1 image does not bundle `docker-mcp`; run Docker MCP Gateway on the host, or mount/provide a Linux-compatible `docker-mcp` CLI plugin inside the container if you want Codex's first-run Docker MCP auto-configuration to run from inside the container.

This repository is licensed under the [Apache-2.0 License](LICENSE).
