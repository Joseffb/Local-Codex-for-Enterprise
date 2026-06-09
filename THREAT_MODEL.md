# Local Codex for Enterprise Threat Model

This document tracks the security boundaries and known risks for the enterprise server mode while it is under active development.

Local Codex for Enterprise is a community fork. It is not affiliated with, endorsed by, or supported by OpenAI.

## Trust Boundaries

- The enterprise server is trusted to authenticate users, enforce authorization, launch workers, and audit decisions.
- Codex workers are scoped to a user, user workspace, project/repository working path, thread/session, and socket.
- Coding threads are persisted as server-side chat histories bound to a user and, after the project/repository migration, a project plus optional repository. Transitional session records may still carry a canonical workspace path.
- Workspace roots are server allowlist boundaries. User workspaces are per-user filesystem spaces. Projects are human work containers. Repositories are cloned checkouts inside projects. Threads are chat histories, not login sessions.
- Context Packs are trusted only as versioned Markdown operating packages.
- Context Packs must not execute code, trigger schedules, call MCP tools, create sessions, create workers, create agents, alter RBAC, dispatch agents, act as workflow engines, or perform governance reasoning.
- Context Packs are not Codex skills; they use enterprise assignment, receipt, audit, tenancy, and RBAC controls instead of local runtime skill installation.
- Trace records and execution receipts are evidence only. They must not become a governance reasoning, authority, or orchestration runtime.
- The websocket tunnel is trusted only to authenticate/authorize connection setup and relay frames to the correct worker socket; app-server JSON-RPC remains opaque.
- Docker Model Runner, Docker Model Gateway, and MCP gateways are external local services and must be explicitly configured.
- The terminal client is not trusted to choose arbitrary server paths or worker targets.

## Workspace Path Risks

- Workspace allowlisting is the primary security boundary.
- Workspace roots and user workspaces are security boundaries; projects and repositories are resolved inside those boundaries.
- Requested paths must be canonicalized before authorization.
- Symlink escapes, nested symlink escapes, `..` traversal, deleted/moved workspaces, hidden/system directories, and Docker socket paths must be denied unless explicitly allowed.
- Repo clone destinations must resolve under a selected project `repos/` directory, never arbitrary paths.
- Repo clone URLs must not use local file, SSH, scp-like, localhost, private, link-local, metadata-service, or credential-bearing targets.
- Audit logs must record resolved paths, not only requested paths.
- Public documentation and examples must not contain private local machine paths.

## Token Risks

- API tokens must be opaque, random, and hash-stored.
- Browser login uses a same-origin HttpOnly cookie that authenticates API calls without storing API tokens in local browser script state.
- Password login must verify stored Argon2 hashes and return API tokens only once.
- Websocket handoff tokens must be short-lived, purpose-bound, audience-bound, and single-use.
- Handoff token records must bind the `jti` to the worker, owner user, user workspace/project/repository context where available, session/thread, socket path, expiry, and consumed timestamp.
- Handoff consumers must reject token replay, expired tokens, and JWT claims that do not match the stored handoff record.
- Token issue, use, expiry, and revocation events must be audited.

## Worker Lifecycle Risks

- Workers must be tracked from startup through shutdown or failure.
- The supervisor must record owner user, user workspace/project/repository context where available, session/thread, PID/container ID, state, heartbeat time, logs pointer, and cleanup policy.
- Abandoned or failed workers must be cleaned up deterministically.
- Worker process launch must use canonicalized working paths derived from a selected project or repository and authorized against the user's workspace boundary.
- Worker command and argument templates must not allow user-controlled shell expansion.

## Audit Requirements

- Audit authentication events, authorization decisions, workspace root/user workspace/project/repository access, thread/session lifecycle events, token lifecycle events, worker lifecycle events, setup/bootstrap actions, and administrative changes.
- Audit, security, and receipt records must include trace ID, actor, applicable workspace/session/worker context, event type, result, redacted metadata, and creation time.
- Audit records must avoid storing plaintext secrets, bearer headers, handoff JWTs, credential-bearing repo URLs, private/runtime prompts, raw model outputs, or environment secrets.
- Audit records, receipts, logs, seeded docs, and example docs must avoid storing private/runtime prompts, model outputs, auth headers, handoff tokens, passwords, API tokens, repo credentials, or private real-life examples. Synthetic reusable prompt templates are allowed in Context Pack examples only as inert text assets.
- Context Pack receipts must record pack ID, document ID, content hash, load order, assignment source, actor, workspace, session, optional worker, phase, trace ID, and creation time. They must not store Markdown document bodies.
- Initial trace coverage records bootstrap, login success/failure, RBAC denial, workspace clone, session create/list/get, worker start/stop, handoff issue/consume, and websocket tunnel connection events.
- Execution receipts are emitted for session creation, context pack session/worker loading, repo clone attempts, worker start/stop, handoff issue/consume, and websocket tunnel connection.

See [docs/enterprise-domain-contract.md](docs/enterprise-domain-contract.md) for the authoritative workspace root -> user workspace -> project -> repository -> thread vocabulary.

## Known Unsafe/Incomplete Areas

- Enterprise mode is MVP work and not yet ready for production use.
- The public-ready browser coding client is not complete; the current browser shell manages setup, admin, sessions, workers, and audit evidence.
- OIDC is not implemented in v1.
- Cedar/ABAC policy packs are reserved for future work.
- Password login, browser login, minimal user management, seeded role assignment, workspace registration/validation, Context Pack upload/assignment/receipts, session ledger persistence, trace-aware audit events, execution receipts, audit query APIs, worker process launch, HTTPS-only repo clone intake, handoff token issue/consume, and the initial remote TUI websocket tunnel are implemented.
- Project/repository domain tables, model/tool invocation capture from worker boundaries, worker restart reconciliation, custom RBAC policy editing, and full audit export/reporting are still incomplete.
- Dashboards, SIEM export, Fernain bridge, approval workflow engine, resolver graphs, cognition phases, and generalized governance orchestration are intentionally out of this slice.
- Argon2, Casbin, and Utoipa are wired for password hashing, seeded RBAC policy evaluation, and OpenAPI generation. Custom policy persistence and full route documentation are still incomplete.
