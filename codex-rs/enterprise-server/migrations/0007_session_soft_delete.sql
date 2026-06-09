ALTER TABLE enterprise_sessions
    ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS enterprise_sessions_active_owner_idx
    ON enterprise_sessions(owner_user_id, updated_at DESC)
    WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS enterprise_sessions_active_project_idx
    ON enterprise_sessions(project_id, updated_at DESC)
    WHERE deleted_at IS NULL;
