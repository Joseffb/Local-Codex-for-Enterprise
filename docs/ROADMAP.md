# Roadmap

This roadmap tracks public product direction for Local Codex for Enterprise. It is intentionally high level and does not contain private work notes, customer examples, local paths, prompts, model outputs, secrets, screenshots, or personal data.

## Current MVP Focus

- Prove the browser chat, project, repository, thread, worker, app-server, and local Docker model loop end to end.
- Keep the domain hierarchy explicit: workspace root -> user workspace -> project -> repository -> thread.
- Keep Context Packs as governed instruction/context material only.
- Preserve trace, audit, and receipt continuity without storing prompts, model outputs, credentials, handoff tokens, or private examples.
- Keep the UI functional for setup, login, admin, projects, context packs, outputs, audit, and chat.

## Near-Term Work

- Polish the browser chat workbench so thread switching clears and reloads the selected thread history cleanly.
- Add a copy icon to each chat turn so users can copy any user or Codex message directly from the transcript.
- Improve project administration under user profiles:
  - create projects for a selected user workspace
  - rename projects
  - remove projects
  - view repositories and threads by project
- Add clearer repository management inside project menus.
- Add first-class project CRUD views to the admin UI while preserving the REST API contract.
- Improve browser-worker connection diagnostics so failed handoff, app-server, and model-call states are obvious to users.
- Expand demo data to cover user workspace, project, repository, thread, context pack, output, and audit use cases.
- Validate the full Compose-based enterprise demo with Postgres and Docker Model Runner.

## Planned: Scheduled Sessions

Scheduled sessions are planned, but they are not part of the current project/thread cleanup run.

The architectural decision is that automation is implemented as scheduled sessions. A scheduler creates sessions on behalf of a user according to a schedule definition. The scheduler does not execute Context Packs directly, and Context Packs remain guidance/context artifacts only.

Everything that can be represented as a session should be a session. Interactive use is a user-created session. Automation is a scheduler-created session. Both share the same workers, traces, receipts, audits, Context Pack loading, and execution lifecycle.

Conceptual lifecycle:

```text
Schedule
  -> creates session
  -> supplies task prompt
  -> loads assigned Context Packs
  -> starts worker
  -> runs Codex task
  -> produces artifact/output
  -> records receipts
```

Responsibility split:

- Context Pack = knowledge, rules, calibration, handoffs, operating guidance, and project context.
- Schedule = trigger, owner, target scope, cadence, and enabled/disabled state.
- Prompt = task instruction for what Codex should do during the scheduled session.
- Session = execution boundary that combines the prompt and assigned Context Packs.

Example uses include weekly architecture review, daily repository audit, dependency/security review, documentation refresh, executive reporting, Power BI analysis, Salesforce analysis, and RAG maintenance jobs.

Minimal design direction:

- Extend sessions with a narrow session type: `interactive` or `scheduled`.
- Add `enterprise_schedules` with `schedule_id`, `owner_user_id`, `name`, `description`, `cron_expression`, `enabled`, `workspace_id`, `task_prompt`, optional `prompt_template_ref`, `created_at`, and `updated_at`.
- Add `enterprise_schedule_context_packs` with `schedule_id` and `context_pack_id`.
- Add `enterprise_schedule_runs` with `run_id`, `schedule_id`, `session_id`, `trace_id`, `started_at`, `completed_at`, and `result`.
- Support cron-style schedules through a small scheduler that only creates sessions.
- Enforce RBAC before creating, editing, pausing, resuming, or deleting schedules.
- Scope every run to a user workspace, project, repository, thread, and Context Pack assignment where applicable.
- Record audit events and execution receipts for schedule creation, update, pause, resume, run start, completion, failure, and cancellation.
- Allow Context Packs to offer optional reusable prompt templates only as inert text templates. A schedule may reference one, but the schedule still decides what runs.
- Store only redacted metadata in scheduled-session receipts.
- Never store prompts, model outputs, auth headers, handoff tokens, passwords, API tokens, repo credentials, private examples, or local machine paths in schedule logs, receipts, or audit metadata.

Hard boundary:

- Context Packs remain instructions, knowledge, calibration, handoffs, operating rules, verification guidance, and project context.
- Context Packs answer how Codex should think, what context applies, and what rules should be followed.
- Schedules answer when a task should run, who it runs for, and what scope it targets.
- Prompts answer what Codex should do.
- Context Packs do not execute code, trigger actions, create agents, alter RBAC, perform governance reasoning, or function as workflow definitions.
- Execution prompts are not stored inside Context Packs as the thing that runs. They belong to scheduled sessions, or to a schedule-owned reference to an inert reusable template.
- Scheduled sessions do not introduce a workflow engine, automation runtime, pack execution engine, agent orchestration framework, approval workflow engine, Fernain bridge, policy reasoning engine, committee runtime, or generalized governance system.
- The goal is to extend the existing session architecture, not create a second execution architecture.

Deferred scheduled-session details:

- Multi-node scheduler coordination.
- Retry backoff UI.
- Calendar UI.
- Notification integrations.
- SIEM export.
- Approval gates.
- Fernain compatibility export.

## Later Work

- Full audit export and reporting dashboards.
- Model and tool invocation capture from the worker/app-server boundary.
- Worker restart reconciliation.
- Browser IDE polish.
- SSO/SAML/OIDC.
- Groups, teams, and org hierarchy.
- Custom RBAC role and policy editor.
- Multi-node federation.
- Public release hardening for v0.1.0.

## Out Of Scope For This Product

- Fernain-style governance reasoning.
- Resolver graphs.
- Committee cognition.
- Authority package runtime.
- General policy orchestration.
- Hidden cloud fallback.
- Bundling Docker Model Runner inside the Enterprise server container for v0.1.0.
