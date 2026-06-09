# Local Codex for Enterprise

Local Codex for Enterprise is a community fork and self-hosted enterprise extension of Local Codex for Docker. It keeps the local-first Docker Model Runner direction and adds an enterprise control plane for teams that want governed access to local coding agents.

This repository is not affiliated with, endorsed by, or supported by OpenAI. It is derived from the open-source OpenAI Codex project and preserves the upstream Apache-2.0 license and attribution.

## What This Is

- A local-first Codex fork for Docker Model Runner and Docker Model Gateway.
- A self-hosted enterprise control plane with Postgres-backed state.
- A managed path for users, seeded RBAC roles, workspace root allowlisting, user workspaces, projects, repositories, threads, workers, handoff tokens, trace-aware audit events, and execution receipts.
- A Context Pack system for versioned Markdown operating packages.
- A release candidate foundation for a public community project.

## What This Is Not

- It is not an official OpenAI product.
- It is not a cloud-hosted Codex service.
- It is not a governance reasoning engine.
- It does not include SSO, custom RBAC policy editing, approval workflows, Fernain integration, or full browser IDE polish.
- It does not store prompts, model outputs, auth headers, handoff tokens, passwords, API tokens, repo credentials, or private real-life examples in receipts or audit metadata.

## Current Status

The enterprise server currently supports health/config endpoints, first-run owner setup, password login, browser cookie auth, minimal user management, seeded RBAC role assignment, workspace root registration/validation, HTTPS-only repository clone intake, Context Pack upload/assignment/receipts, thread/session records, workers, short-lived handoff tokens, initial websocket tunneling to worker sockets, trace-aware audit events, execution receipts, audit query APIs, and Docker Compose local evaluation.

The product domain contract is defined in [docs/enterprise-domain-contract.md](docs/enterprise-domain-contract.md). In short: workspace roots are server allowlist boundaries, user workspaces are per-user filesystem spaces, projects are human work containers, repositories are cloned checkouts inside projects, and threads are chat histories attached to projects/repositories.

Default user workspaces are generated as `<workspace root>/user/<user-email>`, for example `/enterprise-workspaces/user/alex@example.com`. Admins can override this default by assigning explicit allowed workspace paths to a user.

Known current limits are listed in [Current Limitations](#current-limitations).

## Install Modes

Local Codex for Enterprise supports both containerized and locally installed server deployments.

- **Docker Compose local evaluation**: Compose starts Postgres and the enterprise server. This is the fastest path for a single-machine demo.
- **Containerized enterprise server with external services**: Run the enterprise app in a container, but point it at an existing Postgres service and host/container Docker Model Runner.
- **Local/server install**: Install the binaries directly on a server, point them at an external Postgres database, and configure workspace roots that exist on that server.

In all modes, the browser `/setup` flow initializes the application owner and initial workspace allowlist. It does not provision Postgres. The database must already be available unless you use the included Compose stack.

## Deployment Profiles And Hardware Guidance

Local Codex for Enterprise has two different hardware concerns:

- The enterprise control plane: browser UI, API server, Postgres, workspace metadata, workers, handoffs, audit, and receipts.
- Inference: the model runtime that actually runs the LLM.

The enterprise control plane itself is not GPU-heavy. GPU requirements depend on where inference runs. Small deployments can use external/API model providers or an existing model server. Larger deployments should separate the control plane, Postgres, workspace storage, and inference hardware.

Model size, context length, concurrency, and whether inference is local or external matter more than the control-plane requirements.

### Local Evaluation

Use this profile for one person testing the project on their own machine.

Minimum:

- 8-core CPU.
- 32GB RAM.
- 100GB SSD.
- Remote/API model provider recommended for smoother evaluation.

Recommended:

- 12+ CPU cores.
- 64GB RAM.
- 1TB NVMe.
- Apple Silicon M2 Max/M3 Max or NVIDIA RTX 3060 12GB+.

Best local experience:

- 16+ CPU cores.
- 128GB RAM.
- NVIDIA RTX 4090/5090 or Apple Silicon with 128GB unified memory.

### Enterprise Server

Use this profile for a self-hosted team deployment.

Pilot / small team:

- 16+ CPU cores.
- 128GB RAM.
- 2TB NVMe.
- Dedicated inference GPU optional when using external/API models or an existing model server.
- Suggested GPU class for local inference: NVIDIA RTX 4090/5090, L40S, or RTX PRO 6000.

Production / serious internal AI platform:

- AMD EPYC or Intel Xeon server.
- 256GB-512GB ECC RAM.
- 4TB-16TB enterprise NVMe.
- 2x L40S or 2x RTX PRO 6000-class GPUs for centralized local inference.
- Postgres on reliable storage.
- Separate backup and disaster-recovery plan.

Clean positioning:

- Runs locally for evaluation.
- Deploys to enterprise hardware for shared team use.
- Inference can be local, remote, or centralized.

## Quick Start With Docker Compose

Prerequisites:

- Docker Desktop or Docker Engine.
- Docker Model Runner enabled.
- A local Docker model pulled, for example:

  ```sh
  docker model pull ai/qwen3-coder
  ```

Start the enterprise stack:

```sh
LOCAL_CODEX_ENTERPRISE_WORKSPACES="$PWD" \
LOCAL_CODEX_ENTERPRISE_HANDOFF_TOKEN_SECRET="change-me-for-real-use" \
  docker compose -f compose.enterprise.yaml up --build
```

Open the web shell:

```text
http://127.0.0.1:8787
```

Check health:

```sh
curl -fsS http://127.0.0.1:8787/healthz
```

Expected response:

```json
{"product":"Local Codex for Enterprise","status":"ok"}
```

Then open `/setup` and bootstrap the owner account.

For the initial workspace root:

- Docker Compose users should use `/enterprise-workspaces`.
- Local/server installs should use a real path visible to the server process.
- Do not enter a host-only path such as a desktop home directory when the server is running inside a container unless that exact path is mounted inside the container.

## Docker Compose Setup

The enterprise Compose stack is defined in [compose.enterprise.yaml](compose.enterprise.yaml).

It starts:

- `postgres`: Postgres 17 with a named volume.
- `enterprise`: Local Codex for Enterprise server.

The enterprise container:

- Builds `codex`, `codex-enterprise-server`, and `codex-app-server`.
- Mounts `LOCAL_CODEX_ENTERPRISE_WORKSPACES` at `/enterprise-workspaces`.
- Sets `LOCAL_CODEX_ENTERPRISE_DEFAULT_WORKSPACE_ROOT=/enterprise-workspaces` so the setup UI shows the container-visible workspace path.
- Mounts the host Docker socket for local Docker services.
- Points Docker Model Runner traffic at `http://host.docker.internal:12434/engines/v1`.
- Launches workers through `codex-container-entrypoint app-server --listen unix://{socket_path}` so worker processes receive the same local Docker model provider config as the container CLI path.
- Sets the default container model from `CODEX_MODEL`, defaulting to `ai/qwen3-coder`.
- Uses container `/tmp` for ephemeral worker sockets and logs.

For a clean reset:

```sh
docker compose -f compose.enterprise.yaml down --volumes
```

### Docker Compose With External Workspace Path

Mount the directory you want Codex workers to access:

```sh
LOCAL_CODEX_ENTERPRISE_WORKSPACES="/srv/engineering-workspaces" \
LOCAL_CODEX_ENTERPRISE_HANDOFF_TOKEN_SECRET="change-me-for-real-use" \
  docker compose -f compose.enterprise.yaml up --build
```

Inside the web setup form, still use:

```text
/enterprise-workspaces
```

That is the path visible from inside the enterprise container. The host path is only the source of the bind mount.

### Containerized App With External Postgres

For production-like container deployments, you can run the enterprise app container while using an external Postgres service instead of the Compose-managed `postgres` service. This is the main path for teams that want the app containerized but database storage managed by a platform such as a database VM, RDS, Cloud SQL, Neon, Supabase, or an internal Postgres service.

In this mode:

- The enterprise app is containerized.
- Postgres is external to the app container.
- Migrations still run automatically when `codex-enterprise-server` starts.
- Workspace paths entered in the browser must be paths visible inside the app container.

Required inputs:

- `DATABASE_URL`: Postgres connection string reachable from the enterprise container.
- `LOCAL_CODEX_ENTERPRISE_HANDOFF_TOKEN_SECRET`: long random secret used to sign short-lived worker handoff tokens.
- `LOCAL_CODEX_ENTERPRISE_DEFAULT_WORKSPACE_ROOT`: the path users should enter during setup, as seen by the container.
- Workspace bind mount: the host or volume path mounted into the container at the same runtime-visible workspace root.
- Docker Model Runner or Docker Model Gateway endpoint reachable from the container.

Example:

```sh
docker run --rm \
  --name local-codex-enterprise \
  --entrypoint codex-enterprise-server \
  -p 8787:8787 \
  -e DATABASE_URL="postgres://codex:REPLACE_ME@postgres.internal:5432/codex_enterprise" \
  -e LOCAL_CODEX_ENTERPRISE_HANDOFF_TOKEN_SECRET="REPLACE_WITH_LONG_RANDOM_SECRET" \
  -e LOCAL_CODEX_ENTERPRISE_DEFAULT_WORKSPACE_ROOT="/enterprise-workspaces" \
  -e CODEX_CONTAINER_DEFAULT_PROVIDER_CONFIG=1 \
  -e CODEX_MODEL="ai/qwen3-coder" \
  -e CODEX_MODEL_PROVIDER_ID="docker-model-runner-container" \
  -e CODEX_MODEL_PROVIDER_NAME="Docker Model Runner" \
  -e CODEX_MODEL_PROVIDER_BASE_URL="http://host.docker.internal:12434/engines/v1" \
  -v "/srv/engineering-workspaces:/enterprise-workspaces" \
  -v "$HOME/.docker/run/docker.sock:/docker.sock" \
  local-codex-enterprise:dev \
  --bind-addr 0.0.0.0:8787 \
  --worker-command /usr/local/bin/codex-container-entrypoint \
  --worker-arg app-server \
  --worker-arg --listen \
  --worker-arg 'unix://{socket_path}'
```

Adapt the Docker socket mount and model endpoint for your server. On Linux, `host.docker.internal` may require explicit host-gateway configuration if you are not using Compose.

If the external Postgres service is another container on a Docker network, attach this app container to that network and use the Postgres container or service DNS name in `DATABASE_URL`.

## Local Server Install With External Postgres

Use this mode when Local Codex for Enterprise is installed directly on a server rather than run as the enterprise container.

### Local Prerequisites

- Rust toolchain matching the repository toolchain.
- Postgres reachable from the server.
- `git`, `rg`, and normal build prerequisites for the Codex Rust workspace.
- Docker Model Runner or Docker Model Gateway reachable from the server if you want the default local model path.
- A dedicated workspace root on the server, for example `/srv/local-codex/workspaces`.
- A dedicated Codex home, for example `/var/lib/local-codex-enterprise/codex-home`.

### Create The External Database

Create a database and application user in your Postgres service. Example SQL:

```sql
CREATE USER codex_enterprise WITH PASSWORD 'REPLACE_WITH_STRONG_PASSWORD';
CREATE DATABASE codex_enterprise OWNER codex_enterprise;
GRANT ALL PRIVILEGES ON DATABASE codex_enterprise TO codex_enterprise;
```

The enterprise server runs its embedded migrations automatically on startup. The browser setup page does not create the database.

### Build The Server Binaries

From the repository root:

```sh
cd codex-rs
cargo build --locked \
  -p codex-enterprise-server --bin codex-enterprise-server \
  -p codex-app-server --bin codex-app-server \
  -p codex-cli --bin codex
```

Install or reference the resulting binaries from `codex-rs/target/debug` or `codex-rs/target/release`, depending on your build profile.

### Configure Local Runtime

Create server-owned directories:

```sh
sudo mkdir -p /srv/local-codex/workspaces
sudo mkdir -p /var/lib/local-codex-enterprise/codex-home
sudo mkdir -p /var/log/local-codex-enterprise
```

Set environment variables for the server process:

```sh
export DATABASE_URL="postgres://codex_enterprise:REPLACE_WITH_STRONG_PASSWORD@127.0.0.1:5432/codex_enterprise"
export CODEX_HOME="/var/lib/local-codex-enterprise/codex-home"
export LOCAL_CODEX_ENTERPRISE_BIND="0.0.0.0:8787"
export LOCAL_CODEX_ENTERPRISE_HANDOFF_TOKEN_SECRET="REPLACE_WITH_LONG_RANDOM_SECRET"
export LOCAL_CODEX_ENTERPRISE_DEFAULT_WORKSPACE_ROOT="/srv/local-codex/workspaces"
```

If `codex-app-server` is not on `PATH`, pass its absolute path:

```sh
codex-enterprise-server \
  --database-url "$DATABASE_URL" \
  --bind-addr "$LOCAL_CODEX_ENTERPRISE_BIND" \
  --worker-command /absolute/path/to/codex-app-server \
  --worker-arg --listen \
  --worker-arg 'unix://{socket_path}'
```

If `codex-app-server` is on `PATH`, the default worker command can be used:

```sh
codex-enterprise-server \
  --database-url "$DATABASE_URL" \
  --bind-addr "$LOCAL_CODEX_ENTERPRISE_BIND"
```

The server will:

1. Connect to Postgres.
2. Run enterprise migrations.
3. Start the web UI.
4. Launch supervised workers as needed.

Open:

```text
http://SERVER_HOST:8787/setup
```

For the initial workspace root in local/server install mode, enter a real path visible to the server process, such as:

```text
/srv/local-codex/workspaces
```

### Local Model Configuration

For local installs, configure Codex to use Docker Model Runner or Docker Model Gateway through the normal Local Codex configuration path. A typical local Docker Model Runner endpoint is:

```text
http://localhost:12434/engines/v1
```

If the enterprise server launches workers on the same host, workers inherit the server process environment and use the configured `CODEX_HOME`. Make sure the service user can read the relevant Codex config and can access the configured workspace roots.

### Local Install Health Check

```sh
curl -fsS http://SERVER_HOST:8787/healthz
```

Expected response:

```json
{"product":"Local Codex for Enterprise","status":"ok"}
```

## Architecture Overview

```mermaid
flowchart LR
    Browser["Browser UI"] --> Enterprise["Enterprise Server"]
    CLI["CLI/API Client"] --> Enterprise
    Enterprise --> Postgres["Postgres"]
    Enterprise --> Worker["Supervised Codex Worker"]
    Worker --> AppServer["codex-app-server"]
    AppServer --> Codex["Codex Runtime"]
    Codex --> DockerModel["Docker Model Runner or Gateway"]
    Enterprise --> Audit["Audit Events and Receipts"]
```

Core boundaries:

- The enterprise server owns identity, RBAC checks, workspace root validation, user workspace/project/repository/thread records, worker supervision, handoff issuance, and audit evidence.
- Workers are scoped to an actor, user workspace, project/repository working path, thread/session, and socket.
- The app-server protocol remains opaque to the enterprise control plane.
- Context packs are loaded as operating material only.
- Receipts are evidence, not reasoning.

Domain hierarchy:

```text
workspace root -> user workspace -> project -> repository -> thread
```

Workspace is not a project. Projects are not filesystem security boundaries. Repositories are not user workspaces. Threads are not login sessions.

The default per-user path is `<workspace root>/user/<user-email>`. Setup assigns this default to the bootstrap admin, and user creation assigns it when admins leave the per-user workspace field blank.

## Local-Only Contract

- No OpenAI API key is required by default.
- No outbound model calls are made by default.
- There is no automatic cloud fallback.
- Cloud providers remain available only when explicitly configured or selected.
- Docker/local execution remains the default behavior.

Default model config:

```toml
model_provider = "docker-model-runner"
model = "ai/qwen3-coder"
```

`ai/qwen3-coder` is a starter default, not a permanent product assumption. Change `model` through normal Codex config paths.

Built-in local providers:

- `docker-model-runner`: `http://localhost:12434/engines/v1`
- `docker-model-gateway`: `http://localhost:4000/v1`

Both use `wire_api = "chat_completions"`.

## Context Packs

Context Packs are versioned operating packages that bundle project knowledge, operating instructions, workflows, procedures, checklists, calibration, handoffs, verification guidance, escalation guidance, and reusable prompt templates. They are loaded into sessions to guide Codex behavior and produce receipts proving which operating package influenced the run.

Context Packs are not Codex skills. Codex skills are runtime capability packages for agent behavior or tool use. Context Packs are enterprise-owned operating packages with user, workspace, project, assignment, receipt, audit, and RBAC surfaces.

See [docs/context-pack-contract.md](docs/context-pack-contract.md) for the authoritative Context Pack contract.

Standard filenames:

- `PACK.md`
- `CALIBRATION.md`
- `OPERATING-INSTRUCTIONS.md`
- `PROJECT-RULES.md`
- `WORKFLOWS.md`
- `HANDOFF.md`
- `VERIFICATION.md`
- `ESCALATION.md`
- `CONTEXT.md`
- `PROMPTS.md`

Only `PACK.md` is universally required. Other canonical documents are optional and can be required by the pack manifest. Custom uppercase Markdown files are allowed.

Hard boundary:

- Packs are loaded as operating material only.
- Packs do not execute code.
- Packs do not trigger actions.
- Packs do not call MCP tools.
- Packs do not create sessions or workers.
- Packs do not create agents.
- Packs do not alter RBAC.
- Packs do not perform governance reasoning.
- Packs do not function as workflow engines or Codex skills.

See [docs/examples/context-pack-basic](docs/examples/context-pack-basic) for a safe synthetic example.

## Example Context Pack

Minimal `PACK.md`:

```markdown
# Basic Engineering Pack

required_documents:
- OPERATING-INSTRUCTIONS.md
- VERIFICATION.md

load_order:
- PACK.md
- OPERATING-INSTRUCTIONS.md
- VERIFICATION.md

This pack provides basic operating material for a generic engineering project.
```

The server validates that required documents are present and that load order is deterministic. If multiple assignments create an ambiguous order for the same user/project context, the request is rejected.

## Example Receipt

Receipts record what context was loaded without storing document bodies, prompts, model outputs, tokens, or credentials.

```json
{
  "receipt_id": "receipt_01J00000000000000000000000",
  "trace_id": "00000000-0000-4000-8000-000000000001",
  "actor_user_id": "user_owner",
  "workspace_id": "workspace_01J0000000000000000000000",
  "session_id": "session_01J0000000000000000000000",
  "worker_id": "worker_01J0000000000000000000000",
  "event_type": "context_pack.worker_load",
  "result": "completed",
  "metadata_json": {
    "pack_id": "pack_basic",
    "document_id": "doc_verification",
    "content_hash": "sha256:9e1f6f64b49a2f4fd8365f2fb2c8d872f6bb84e6f3ce8f08f3d6f6c3f2d5b1a0",
    "load_order": 30,
    "assignment_source": "workspace",
    "phase": "worker_start"
  },
  "created_at": "2026-01-01T00:00:00Z"
}
```

See [docs/examples/receipts](docs/examples/receipts) for additional examples.

## Demo

The release-readiness demo is documented in [docs/demo-browser-worker-local-model.md](docs/demo-browser-worker-local-model.md).

The demo covers Compose startup, `/healthz`, browser login, session creation, worker launch, handoff issuance, audit/receipt query, and a local Docker model code-change proof.

## Roadmap

See [docs/ROADMAP.md](docs/ROADMAP.md) for planned work. Scheduled sessions are listed there as deferred functionality that reuses the existing session, worker, trace, audit, receipt, and Context Pack loading lifecycle. Context Packs remain operating packages only; they are not executable workflow definitions, schedulers, governance runtimes, or Codex skills.

## Current Limitations

- Browser UI is a functional server-served control-plane shell, not a polished IDE.
- The public-ready browser-to-worker coding client is not complete.
- The app-server `model/list` response can still expose upstream model catalog text even when the worker runtime is configured for the local Docker model.
- Project/repository domain tables are the next migration slice; current scaffold still has transitional workspace-path session fields.
- Worker restart reconciliation is not implemented.
- Custom RBAC policy editing is intentionally deferred.
- SSO/SAML/OIDC is intentionally deferred.
- Groups, teams, and org hierarchy are intentionally deferred.
- Full audit export/reporting dashboards are intentionally deferred.
- Approval workflows and Fernain bridge are intentionally deferred.
- Model/tool invocation capture from worker boundaries is schema groundwork only.

## Security

See [SECURITY.md](SECURITY.md) and [THREAT_MODEL.md](THREAT_MODEL.md).

Security-sensitive release rule:

Do not commit private/runtime prompts, model outputs, auth headers, handoff tokens, passwords, API tokens, repo credentials, screenshots with private data, local machine paths, or private real-life examples. Synthetic reusable prompt templates are allowed only as Context Pack template examples.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

This repository is licensed under the [Apache-2.0 License](LICENSE). It is derived from OpenAI Codex and keeps upstream attribution in [NOTICE](NOTICE).
