-- Backfill v0 default user workspaces for databases created before the
-- /user/<email> namespace became the default.
--
-- This is intentionally non-destructive:
-- - existing user workspace records are preserved
-- - existing explicit workspace assignments are preserved
-- - users are assigned under the first registered workspace root only when
--   they do not already have a user workspace

CREATE EXTENSION IF NOT EXISTS pgcrypto;

WITH first_workspace_root AS (
    SELECT workspace_id, root_path
    FROM enterprise_workspaces
    ORDER BY created_at ASC, root_path ASC
    LIMIT 1
),
missing_user_workspaces AS (
    SELECT
        u.user_id,
        COALESCE(
            NULLIF(
                trim(BOTH '-.@_' FROM regexp_replace(lower(u.email), '[^a-z0-9@._-]+', '-', 'g')),
                ''
            ),
            'user'
        ) AS email_namespace
    FROM enterprise_users u
    WHERE NOT EXISTS (
        SELECT 1
        FROM enterprise_user_workspaces uw
        WHERE uw.owner_user_id = u.user_id
    )
),
inserted_user_workspaces AS (
    INSERT INTO enterprise_user_workspaces
        (user_workspace_id, owner_user_id, workspace_root_id, path)
    SELECT
        gen_random_uuid(),
        m.user_id,
        r.workspace_id,
        r.root_path || '/user/' || m.email_namespace
    FROM missing_user_workspaces m
    CROSS JOIN first_workspace_root r
    ON CONFLICT (owner_user_id, path) DO NOTHING
    RETURNING owner_user_id, workspace_root_id, path
)
INSERT INTO enterprise_workspace_assignments
    (assignment_id, user_id, workspace_id, workspace_root, assigned_by)
SELECT
    gen_random_uuid(),
    uw.owner_user_id,
    uw.workspace_root_id,
    uw.path,
    COALESCE(
        (SELECT owner_user_id FROM enterprise_bootstrap ORDER BY created_at ASC LIMIT 1),
        uw.owner_user_id
    )
FROM inserted_user_workspaces uw
ON CONFLICT (user_id, workspace_root) DO NOTHING;
