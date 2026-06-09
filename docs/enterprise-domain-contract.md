# Enterprise Domain Contract

This contract fixes the product language and data-model hierarchy for Local Codex for Enterprise. Use these terms consistently in UI, API design, migrations, tests, documentation, and audit evidence.

## Canonical Hierarchy

```text
workspace root -> user workspace -> project -> repository -> thread
```

Example layout:

```text
/enterprise-workspaces/
  user/
    alex@example.com/
    projects/
      leira-ai/
        repos/
          api/
          web/
        outputs/
        threads/
```

## Terms

### Workspace root

A Workspace root is a server-visible filesystem allowlist root. It is configured by setup/admin flows and is the outer security boundary for user workspaces.

Examples:

- `/enterprise-workspaces` inside the Docker Compose container.
- `/srv/local-codex/workspaces` for a local server install.

Rules:

- Workspace roots are canonicalized before use.
- Workspace roots are not shown as user projects in the chat sidebar.
- Workspace roots are not direct coding targets except during transitional compatibility.
- Admin setup and infrastructure docs may use the term Workspace root.

### User workspace

A User workspace is one user's filesystem space under a Workspace root.

Example:

```text
/enterprise-workspaces/user/alex@example.com
```

Rules:

- User workspaces are the per-user path boundary.
- Default user workspaces are generated as `<workspace root>/user/<sanitized user email>`.
- The bootstrap admin receives this same default user workspace during setup.
- Admins may override defaults by assigning one or more explicit allowed workspace paths to a user.
- A user can create projects only inside their own user workspace unless access is explicitly granted.
- User workspace assignment is a security and tenancy concern, not a project-management concern.
- Workspace is not a project.

### Project

A Project is a human work container inside a User workspace.

Example:

```text
/enterprise-workspaces/user/alex@example.com/projects/leira-ai
```

Rules:

- Projects are the primary objects in the chat sidebar.
- Projects contain repositories, outputs, and threads.
- Projects may have access grants so another user can collaborate.
- Projects are not filesystem security boundaries; their resolved paths must still stay inside an authorized user workspace.

### Repository

A Repository is one cloned source checkout inside a Project.

Example:

```text
/enterprise-workspaces/user/alex@example.com/projects/leira-ai/repos/api
```

Rules:

- A project can contain multiple repositories.
- Clone actions create repositories under a selected project.
- Repository URLs must be HTTPS-only and must not contain credentials.
- Repositories are not user workspaces.

### Thread

A Thread is a chat history attached to a Project and optionally scoped to one Repository or working directory.

Rules:

- Threads are user-visible conversation histories.
- Threads are not login sessions.
- A browser login session can resume an existing thread.
- A worker may be started for a thread, but a thread outlives any single worker process.

## Permission Contract

- Admins manage workspace roots, users, roles, and global safety settings.
- Managers may create user outputs and manage allowed surfaces granted by RBAC.
- Users own their user workspace and may create projects/repositories inside it when permitted.
- A project owner, manager, or admin can grant another user access to a project.
- Repository access is inherited from project access unless a later explicit repository access model is added.

## Worker Contract

Workers launch with a resolved working path derived from a selected project or repository.

The working path must satisfy all of these:

- It is canonicalized.
- It is inside an authorized user workspace.
- If repository-scoped, it is inside the selected project's `repos/` directory.
- It is not a symlink escape, Docker socket path, hidden/system root, or arbitrary user-supplied path.

## Trace And Receipt Contract

Audit and receipt records should preserve the hierarchy without storing prompts or model output.

Future records should include these fields when applicable:

- `workspace_root_id`
- `user_workspace_id`
- `project_id`
- `repository_id`
- `thread_id`
- `session_id`
- `worker_id`
- `trace_id`

Existing transitional fields named `workspace_id`, `workspace_path`, and `session_id` may remain for compatibility, but new implementation should not treat them as the final domain vocabulary.

## UI Contract

- Setup/admin infrastructure pages may say Workspace root.
- User/account pages may say User workspace.
- Chat sidebar says Projects.
- Project rows contain thread rows.
- Project menus include New thread, Clone repository, and Manage access.
- Thread content loads into the conversation area and replaces the previous thread view.

## REST API Contract

Use plural resource names. Nest collection creation/listing where the parent resource is required, and use canonical direct routes for individual resources.

Infrastructure roots:

- `GET /v1/workspace-roots`
- `POST /v1/workspace-roots`
- `GET /v1/workspace-roots/{workspace_root_id}`
- `POST /v1/workspace-roots/{workspace_root_id}/validations`

User workspaces:

- `GET /v1/user-workspaces`
- `GET /v1/user-workspaces/{user_workspace_id}`
- `POST /v1/user-workspaces/{user_workspace_id}/access-grants`
- `POST /v1/user-workspaces/{user_workspace_id}/projects`

Projects:

- `GET /v1/projects`
- `POST /v1/projects`
- `GET /v1/projects/{project_id}`
- `PATCH /v1/projects/{project_id}`
- `DELETE /v1/projects/{project_id}`
- `POST /v1/projects/{project_id}/access-grants`
- `GET /v1/projects/{project_id}/repositories`
- `POST /v1/projects/{project_id}/repositories/clone`
- `GET /v1/projects/{project_id}/threads`
- `POST /v1/projects/{project_id}/threads`

Repositories:

- `GET /v1/repositories/{repository_id}`
- `DELETE /v1/repositories/{repository_id}`

Threads:

- `GET /v1/threads/{thread_id}`
- `PATCH /v1/threads/{thread_id}`
- `DELETE /v1/threads/{thread_id}`
- `GET /v1/threads/{thread_id}/messages`
- `POST /v1/threads/{thread_id}/messages`
- `POST /v1/threads/{thread_id}/workers`

Workers:

- `GET /v1/workers`
- `GET /v1/workers/{worker_id}`
- `DELETE /v1/workers/{worker_id}`
- `POST /v1/workers/{worker_id}/handoffs`
- `POST /v1/worker-handoffs/{handoff_id}/consumptions`

Users and roles:

- `GET /v1/users`
- `POST /v1/users`
- `GET /v1/users/{user_id}`
- `PATCH /v1/users/{user_id}`
- `POST /v1/users/{user_id}/deactivations`
- `POST /v1/users/{user_id}/reactivations`
- `PUT /v1/users/{user_id}/role`
- `GET /v1/roles`

Context packs:

- `GET /v1/context-packs`
- `POST /v1/context-packs`
- `GET /v1/context-packs/{pack_id}`
- `POST /v1/context-packs/{pack_id}/assignments`
- `GET /v1/context-pack-assignments`
- `DELETE /v1/context-pack-assignments/{assignment_id}`

Outputs and evidence:

- `GET /v1/outputs`
- `POST /v1/outputs`
- `GET /v1/outputs/{output_id}`
- `GET /v1/outputs/{output_id}/download`
- `GET /v1/evidence-records`
- `GET /v1/audit-events`
- `GET /v1/execution-receipts`

Do not expose compatibility routes such as `/v1/workspaces`, `/v1/sessions`, or `/v1/admin/*` in v0. Internal function names may remain temporarily while public routes use the REST contract above.

Avoid these phrases in new UI:

- Workspace as a synonym for project.
- Start session as the primary user-facing chat action.
- Open thread in the composer.
- Workspace root as a chat sidebar project name except as temporary legacy data.

## Migration Contract

Implement the hierarchy with additive migrations:

1. Add user workspace, project, repository, and project access tables.
2. Backfill existing assigned workspace paths into default user workspaces and projects.
3. Move session/thread creation to project IDs and optional repository IDs.
4. Move clone flows under selected projects.
5. Keep compatibility reads for old sessions until the scaffold database can be reset or migrated safely.

Do not edit previously applied migrations for live dev databases. Add new idempotent migrations.
