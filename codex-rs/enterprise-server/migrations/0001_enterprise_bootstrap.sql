-- Local Codex for Enterprise bootstrap schema.
CREATE TABLE IF NOT EXISTS enterprise_users (
    user_id UUID PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS enterprise_api_tokens (
    token_id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES enterprise_users(user_id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS enterprise_workspaces (
    workspace_id UUID PRIMARY KEY,
    root_path TEXT NOT NULL UNIQUE,
    created_by UUID NOT NULL REFERENCES enterprise_users(user_id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS enterprise_bootstrap (
    bootstrap_id UUID PRIMARY KEY,
    owner_user_id UUID NOT NULL REFERENCES enterprise_users(user_id),
    owner_email TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS enterprise_sessions (
    session_id TEXT PRIMARY KEY,
    owner_user_id UUID NOT NULL REFERENCES enterprise_users(user_id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    workspace_path TEXT NOT NULL,
    title TEXT,
    last_worker_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS enterprise_sessions_owner_idx
    ON enterprise_sessions(owner_user_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS enterprise_workers (
    worker_id UUID PRIMARY KEY,
    owner_user_id UUID NOT NULL REFERENCES enterprise_users(user_id),
    workspace_id TEXT NOT NULL,
    workspace_path TEXT NOT NULL,
    session_id TEXT NOT NULL,
    state TEXT NOT NULL,
    pid BIGINT,
    socket_path TEXT,
    log_path TEXT,
    last_heartbeat_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS enterprise_workers_owner_idx
    ON enterprise_workers(owner_user_id, created_at DESC);

CREATE TABLE IF NOT EXISTS enterprise_worker_handoffs (
    jti TEXT PRIMARY KEY,
    worker_id UUID NOT NULL REFERENCES enterprise_workers(worker_id) ON DELETE CASCADE,
    owner_user_id UUID NOT NULL REFERENCES enterprise_users(user_id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    socket_path TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS enterprise_worker_handoffs_worker_idx
    ON enterprise_worker_handoffs(worker_id, created_at DESC);

CREATE INDEX IF NOT EXISTS enterprise_worker_handoffs_owner_idx
    ON enterprise_worker_handoffs(owner_user_id, created_at DESC);

CREATE TABLE IF NOT EXISTS enterprise_audit_events (
    event_id UUID PRIMARY KEY,
    actor_user_id UUID REFERENCES enterprise_users(user_id),
    event_type TEXT NOT NULL,
    event_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
