ALTER TABLE enterprise_sessions
    ADD COLUMN IF NOT EXISTS session_type TEXT NOT NULL DEFAULT 'interactive'
    CHECK (session_type IN ('interactive', 'scheduled'));

CREATE TABLE IF NOT EXISTS enterprise_schedules (
    schedule_id UUID PRIMARY KEY,
    owner_user_id UUID NOT NULL REFERENCES enterprise_users(user_id) ON DELETE CASCADE,
    created_by_user_id UUID NOT NULL REFERENCES enterprise_users(user_id) ON DELETE RESTRICT,
    project_id UUID NOT NULL REFERENCES enterprise_projects(project_id) ON DELETE CASCADE,
    repository_id UUID REFERENCES enterprise_repositories(repository_id) ON DELETE SET NULL,
    name TEXT NOT NULL,
    description TEXT,
    cron_expression TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    task_prompt TEXT NOT NULL,
    prompt_template_ref TEXT,
    runner_mode TEXT NOT NULL DEFAULT 'smoke' CHECK (runner_mode IN ('smoke', 'app_server_rpc')),
    next_run_at TIMESTAMPTZ NOT NULL,
    last_run_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS enterprise_schedules_due_idx
    ON enterprise_schedules(enabled, next_run_at)
    WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS enterprise_schedules_owner_idx
    ON enterprise_schedules(owner_user_id, updated_at DESC)
    WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS enterprise_schedule_context_packs (
    schedule_id UUID NOT NULL REFERENCES enterprise_schedules(schedule_id) ON DELETE CASCADE,
    pack_id UUID NOT NULL REFERENCES enterprise_context_packs(pack_id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (schedule_id, pack_id)
);

CREATE TABLE IF NOT EXISTS enterprise_schedule_runs (
    run_id UUID PRIMARY KEY,
    schedule_id UUID NOT NULL REFERENCES enterprise_schedules(schedule_id) ON DELETE CASCADE,
    owner_user_id UUID NOT NULL REFERENCES enterprise_users(user_id) ON DELETE CASCADE,
    session_id TEXT REFERENCES enterprise_sessions(session_id) ON DELETE SET NULL,
    worker_id UUID REFERENCES enterprise_workers(worker_id) ON DELETE SET NULL,
    output_id UUID REFERENCES enterprise_outputs(output_id) ON DELETE SET NULL,
    trace_id UUID NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'failed', 'cancelled')),
    runner_mode TEXT NOT NULL CHECK (runner_mode IN ('smoke', 'app_server_rpc')),
    metadata_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS enterprise_schedule_runs_schedule_idx
    ON enterprise_schedule_runs(schedule_id, started_at DESC);

CREATE INDEX IF NOT EXISTS enterprise_schedule_runs_trace_idx
    ON enterprise_schedule_runs(trace_id);
