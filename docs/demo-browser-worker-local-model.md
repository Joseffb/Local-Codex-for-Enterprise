# Demo: Browser, Worker, and Local Docker Model

This guide verifies the release-readiness path without storing prompts, model outputs, auth headers, handoff tokens, passwords, API tokens, repository credentials, or private examples in receipts, logs, or docs.

## Scope

This demo proves:

- Docker Compose stack starts.
- `/healthz` responds.
- Browser login works.
- Session creation works.
- Worker start works.
- Handoff issuance works.
- Audit/receipt query works.
- Local Docker model can perform a code-change task through the Codex runtime.

Current limitation: the browser UI is a control-plane shell, not a finished browser IDE. The app-server websocket tunnel is available through the worker handoff route, but the browser shell does not yet include a built-in RPC client.

Known release blocker: app-server `model/list` can still return upstream model catalog text even when the worker process is configured with the Docker Model Runner provider. Use the worker command line and local model code-change proof to verify local execution until that catalog surface is cleaned up.

## Prerequisites

- Docker Desktop or Docker Engine.
- Docker Model Runner enabled.
- A local model pulled:

  ```sh
  docker model pull ai/qwen3-coder
  ```

## Start The Stack

```sh
LOCAL_CODEX_ENTERPRISE_WORKSPACES="$PWD" \
  docker compose -f compose.enterprise.yaml up --build
```

Health check:

```sh
curl -fsS http://127.0.0.1:8787/healthz
```

Expected:

```json
{"product":"Local Codex for Enterprise","status":"ok"}
```

## Browser Control-Plane Path

Open:

```text
http://127.0.0.1:8787/setup
```

Bootstrap with synthetic values:

- Email: `owner@example.com`
- Password: use a local throwaway test password.
- Workspace root: `/enterprise-workspaces`.

Open:

```text
http://127.0.0.1:8787/login
```

Sign in as the owner, then use the admin pages:

- `/admin/users`
- `/admin/rbac`
- `/admin/workspaces`
- `/admin/context-packs`
- `/app`
- `/admin/audit`

Create a thread/session for a project or transitional workspace path under `/enterprise-workspaces`, start a worker, issue a handoff, and query audit by trace. The intended domain hierarchy is workspace root -> user workspace -> project -> repository -> thread.

Do not paste real secrets, private examples, private prompts, model outputs, or private repository URLs into the UI during public demo capture.

## API Evidence Path

Use a temporary cookie jar so token values are not printed:

```sh
COOKIE_JAR="$(mktemp)"

curl -fsS -c "$COOKIE_JAR" \
  -H 'content-type: application/json' \
  -d '{"email":"owner@example.com","password":"replace-with-local-test-password"}' \
  http://127.0.0.1:8787/v1/auth/browser-login
```

Create a session:

```sh
curl -fsS -b "$COOKIE_JAR" \
  -H 'content-type: application/json' \
  -H 'x-trace-id: 00000000-0000-4000-8000-000000000001' \
  -d '{"workspace_path":"/enterprise-workspaces","title":"Release demo"}' \
  http://127.0.0.1:8787/v1/sessions
```

Start a worker with the returned `session_id`:

```sh
curl -fsS -b "$COOKIE_JAR" \
  -H 'content-type: application/json' \
  -H 'x-trace-id: 00000000-0000-4000-8000-000000000001' \
  -d '{"workspace_path":"/enterprise-workspaces","session_id":"REPLACE_WITH_SESSION_ID"}' \
  http://127.0.0.1:8787/v1/workers
```

Issue a handoff without logging the token:

```sh
curl -fsS -b "$COOKIE_JAR" \
  -X POST \
  -H 'x-trace-id: 00000000-0000-4000-8000-000000000001' \
  http://127.0.0.1:8787/v1/workers/REPLACE_WITH_WORKER_ID/handoff \
  >/tmp/local-codex-enterprise-handoff-redacted.json
```

Query audit evidence:

```sh
curl -fsS -b "$COOKIE_JAR" \
  'http://127.0.0.1:8787/v1/admin/audit?trace_id=00000000-0000-4000-8000-000000000001'
```

Review the response for event names, IDs, results, and redacted metadata only.

## Worker RPC And Local Model Code-Change Proof

Each websocket connection consumes a handoff token. Mint a fresh handoff for each RPC connection, and never print or commit token values.

Drive the app-server through the enterprise tunnel with the local test client from another shell:

```sh
cd codex-rs
WS_URL="ws://127.0.0.1:8787/v1/workers/REPLACE_WITH_WORKER_ID/rpc?handoff_token=REPLACE_WITH_HANDOFF_TOKEN"

cargo run -p codex-app-server-test-client -- --url "$WS_URL" model-list \
  2> >(sed 's/handoff_token=[^& ]*/handoff_token=<redacted>/g' >&2)
```

Create a disposable demo workspace under the mounted root and make a small local-only change request through the app-server tunnel. Keep the request synthetic, and do not use private project files or private prompts for public release validation.

```sh
mkdir -p demo-workspace
printf 'hello\n' > demo-workspace/message.txt

cargo run -p codex-app-server-test-client -- --url "$WS_URL" send-message-v2 \
  '<synthetic local file-edit request>' \
  2> >(sed 's/handoff_token=[^& ]*/handoff_token=<redacted>/g' >&2)

cat demo-workspace/message.txt
```

The expected result is that the disposable file changes as requested. Do not publish the prompt text or model output from a real private validation run.

## Cleanup

```sh
docker compose -f compose.enterprise.yaml down --volumes
rm -rf demo-workspace
rm -f "$COOKIE_JAR" /tmp/local-codex-enterprise-handoff-redacted.json
```
