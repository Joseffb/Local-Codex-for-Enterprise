# Local Codex for Enterprise Threat Model

This document tracks the security boundaries and known risks for the enterprise server mode while it is still private and under active development.

## Trust Boundaries

- The enterprise server is trusted to authenticate users, enforce authorization, launch workers, and audit decisions.
- Codex workers are scoped to a user, workspace, and session.
- The websocket tunnel is trusted only to authenticate/authorize connection setup and relay frames to the correct worker socket; app-server JSON-RPC remains opaque.
- Docker Model Runner, Docker Model Gateway, and MCP gateways are external local services and must be explicitly configured.
- The terminal client is not trusted to choose arbitrary server paths or worker targets.

## Workspace Path Risks

- Workspace allowlisting is the primary security boundary.
- Requested paths must be canonicalized before authorization.
- Symlink escapes, nested symlink escapes, `..` traversal, deleted/moved workspaces, hidden/system directories, and Docker socket paths must be denied unless explicitly allowed.
- Repo clone destinations must be names directly under an allowlisted root, never arbitrary paths.
- Repo clone URLs must not use local file, SSH, scp-like, localhost, private, link-local, metadata-service, or credential-bearing targets.
- Audit logs must record resolved paths, not only requested paths.

## Token Risks

- API tokens must be opaque, random, and hash-stored.
- Password login must verify stored Argon2 hashes and return API tokens only once.
- Websocket handoff tokens must be short-lived, purpose-bound, audience-bound, and single-use.
- Handoff token records must bind the `jti` to the worker, owner user, workspace, session, socket path, expiry, and consumed timestamp.
- Handoff consumers must reject token replay, expired tokens, and JWT claims that do not match the stored handoff record.
- Token issue, use, expiry, and revocation events must be audited.

## Worker Lifecycle Risks

- Workers must be tracked from startup through shutdown or failure.
- The supervisor must record owner user, workspace, session, PID/container ID, state, heartbeat time, logs pointer, and cleanup policy.
- Abandoned or failed workers must be cleaned up deterministically.
- Worker process launch must use canonicalized, allowlisted workspace paths as the process working directory.
- Worker command and argument templates must not allow user-controlled shell expansion.

## Audit Requirements

- Audit authentication events, authorization decisions, workspace access, token lifecycle events, worker lifecycle events, setup/bootstrap actions, and administrative changes.
- Audit records must avoid storing plaintext secrets.
- Initial audit coverage records bootstrap, login success/failure, RBAC denial, workspace clone, worker start/stop, handoff issue/consume, and websocket tunnel connection events.

## Known Unsafe/Incomplete Areas

- Enterprise mode is private MVP work and not yet ready for public use.
- OIDC is not implemented in v1.
- Cedar/ABAC policy packs are reserved for future work.
- Password login, worker process launch, HTTPS-only repo clone intake, handoff token issue/consume, and the initial remote TUI websocket tunnel are implemented.
- Persisted chat/thread history, worker restart reconciliation, audit query/export APIs, and admin user/role management are still incomplete.
- Argon2, Casbin, and Utoipa are wired at scaffold level for password hashing,
  RBAC policy evaluation, and OpenAPI generation; production database adapters,
  persistent policy loading, and full route coverage are still incomplete.
