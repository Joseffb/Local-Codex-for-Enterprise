-- Projects are archived via soft delete so owners can remove them from chat
-- while admins can still inspect and restore them.

ALTER TABLE enterprise_projects
    ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS enterprise_projects_active_owner_idx
    ON enterprise_projects(owner_user_id, updated_at DESC)
    WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS enterprise_projects_deleted_idx
    ON enterprise_projects(deleted_at)
    WHERE deleted_at IS NOT NULL;
