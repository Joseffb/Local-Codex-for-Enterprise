-- Add the v0 enterprise domain hierarchy:
-- workspace root -> user workspace -> project -> repository -> thread.

CREATE TABLE IF NOT EXISTS enterprise_user_workspaces (
    user_workspace_id UUID PRIMARY KEY,
    owner_user_id UUID NOT NULL REFERENCES enterprise_users(user_id) ON DELETE CASCADE,
    workspace_root_id UUID REFERENCES enterprise_workspaces(workspace_id) ON DELETE SET NULL,
    path TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (owner_user_id, path)
);

CREATE INDEX IF NOT EXISTS enterprise_user_workspaces_owner_idx
    ON enterprise_user_workspaces(owner_user_id, path);

CREATE TABLE IF NOT EXISTS enterprise_projects (
    project_id UUID PRIMARY KEY,
    owner_user_id UUID NOT NULL REFERENCES enterprise_users(user_id) ON DELETE CASCADE,
    user_workspace_id UUID NOT NULL REFERENCES enterprise_user_workspaces(user_workspace_id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    slug TEXT NOT NULL,
    project_path TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_workspace_id, slug)
);

CREATE INDEX IF NOT EXISTS enterprise_projects_owner_idx
    ON enterprise_projects(owner_user_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS enterprise_repositories (
    repository_id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES enterprise_projects(project_id) ON DELETE CASCADE,
    repo_url TEXT,
    name TEXT NOT NULL,
    repository_path TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, name)
);

CREATE INDEX IF NOT EXISTS enterprise_repositories_project_idx
    ON enterprise_repositories(project_id, name);

ALTER TABLE enterprise_sessions
    ADD COLUMN IF NOT EXISTS project_id UUID REFERENCES enterprise_projects(project_id) ON DELETE SET NULL;

ALTER TABLE enterprise_sessions
    ADD COLUMN IF NOT EXISTS repository_id UUID REFERENCES enterprise_repositories(repository_id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS enterprise_sessions_project_idx
    ON enterprise_sessions(project_id, updated_at DESC);
