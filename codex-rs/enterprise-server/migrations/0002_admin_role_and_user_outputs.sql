-- Normalize pre-MVP owner role rows to the admin model and add user-scoped
-- output metadata for databases created before these tables existed.
UPDATE enterprise_users
SET role = 'admin', updated_at = now()
WHERE role = 'owner';

CREATE TABLE IF NOT EXISTS enterprise_outputs (
    output_id UUID PRIMARY KEY,
    owner_user_id UUID NOT NULL REFERENCES enterprise_users(user_id) ON DELETE CASCADE,
    workspace_id TEXT,
    session_id TEXT,
    worker_id UUID REFERENCES enterprise_workers(worker_id) ON DELETE SET NULL,
    category TEXT NOT NULL CHECK (category IN ('operational', 'deliverable')),
    output_type TEXT NOT NULL,
    title TEXT NOT NULL,
    artifact_path TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('draft', 'active', 'completed', 'archived')),
    metadata_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS enterprise_outputs_owner_idx
    ON enterprise_outputs(owner_user_id, category, updated_at DESC);

CREATE TABLE IF NOT EXISTS enterprise_workspace_assignments (
    assignment_id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES enterprise_users(user_id) ON DELETE CASCADE,
    workspace_id UUID REFERENCES enterprise_workspaces(workspace_id) ON DELETE CASCADE,
    workspace_root TEXT NOT NULL,
    assigned_by UUID NOT NULL REFERENCES enterprise_users(user_id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, workspace_root)
);
