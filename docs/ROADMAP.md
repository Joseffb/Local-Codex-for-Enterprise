# Roadmap

This roadmap tracks public product direction for Local Codex for Enterprise. It is intentionally high level and does not contain private work notes, customer examples, local paths, prompts, model outputs, secrets, screenshots, or personal data.

## Current MVP Focus

- Prove the browser chat, project, repository, thread, worker, app-server, and local Docker model loop end to end.
- Keep the domain hierarchy explicit: workspace root -> user workspace -> project -> repository -> thread.
- Keep Context Packs as governed operating packages only.
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
- Expand Context Pack file management after the first stored-file slice.
  - Admins can create, upload, register, rename, download, and soft-remove pack files after a pack exists.
  - Arbitrary file types may be stored as package contents, but paths must remain safe and portable.
  - Only active loadable text files are loaded into session context; assets, scripts, and non-loadable files remain stored evidence/package contents.
  - Skill files may be included or referenced as package contents, but importing a file must not activate or execute the skill.
  - Registration and context loading should preserve receipts so future sessions prove exactly which loadable files were loaded.
- Add first-class project CRUD views to the admin UI while preserving the REST API contract.
- Improve browser-worker connection diagnostics so failed handoff, app-server, and model-call states are obvious to users.
- Expand demo data to cover user workspace, project, repository, thread, context pack, output, and audit use cases.
- Validate the full Compose-based enterprise demo with Postgres and Docker Model Runner.

## Planned: Scheduled Sessions

Scheduled sessions are planned, but they are not part of the current project/thread cleanup run.

The architectural decision is that automation is implemented as scheduled sessions. A scheduler creates sessions on behalf of a user according to a schedule definition. The scheduler does not execute Context Packs directly. Context Packs are loaded into the scheduled session, and the session executes work through the existing worker lifecycle.

Everything that can be represented as a session should be a session. Interactive use is a user-created session. Automation is a scheduler-created session. Both share the same workers, traces, receipts, audits, Context Pack loading, and execution lifecycle.

Conceptual lifecycle:

```text
Schedule
  -> creates session
  -> loads Context Packs
  -> starts worker
  -> executes prompt/template
  -> produces artifacts
  -> records receipts
```

Responsibility split:

- Context Pack = versioned lifecycle package for knowledge, rules, calibration, handoffs, operating guidance, workflow guidance, reusable prompt templates, project context, outputs, and optional skill files or references.
- Schedule = trigger, owner, target scope, cadence, and enabled/disabled state.
- Prompt = task instruction for what Codex should do during the scheduled session.
- Session = execution boundary that combines the prompt and assigned Context Packs.

Example uses include weekly executive reports, daily job application runs, weekly architecture review, security audit, documentation refresh, inventory analysis, sales reporting, dependency/security review, Power BI analysis, Salesforce analysis, and RAG maintenance jobs.

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

- Context Packs remain versioned lifecycle packages: instructions, knowledge, calibration, handoffs, operating rules, workflow guidance, verification guidance, reusable prompt templates, project context, outputs, and optional skill files or references.
- Context Packs answer what Codex should know and how Codex should operate in a user, team, project, workspace, or organization.
- Schedules answer when a task should run, who it runs for, and what scope it targets.
- Prompts answer what Codex should do.
- Context Packs do not execute code, trigger actions, call MCP tools, create sessions, create workers, create agents, alter RBAC, perform governance reasoning, or function as workflow definitions.
- Context Packs are not themselves Codex skills. Skills are runtime capability packages; Context Packs are enterprise lifecycle packages with assignment, audit, receipt, tenancy, and RBAC surfaces.
- Context Packs may include or reference Codex skill files as governed package contents. Runtime activation of any included skill must be explicit, auditable, and permissioned.
- Execution prompts are not stored inside Context Packs as the thing that runs. They belong to scheduled sessions, or to a schedule-owned reference to an inert reusable template.
- Scheduled sessions do not introduce a workflow engine, automation runtime, pack execution engine, agent orchestration framework, approval workflow engine, Fernain bridge, policy reasoning engine, committee runtime, or generalized governance system.
- The goal is to extend the existing session architecture, not create a second execution architecture.

Acceptance criteria:

- Scheduled automation is implemented as scheduled sessions.
- Context Packs are loaded by sessions and never executed directly.
- Interactive and scheduled sessions use the same worker, context loading, audit, trace, and receipt architecture.
- No workflow engine, governance runtime, or agent orchestration framework is introduced.

Deferred scheduled-session details:

- Multi-node scheduler coordination.
- Retry backoff UI.
- Calendar UI.
- Notification integrations.
- SIEM export.
- Approval gates.
- Fernain compatibility export.

## Planned: Cross-Thread Knowledge Transfer

Cross-thread knowledge transfer is planned for recurring reporting, long-running project work, and cases where a user needs work from one thread available in another thread. The goal is knowledge continuity, not agent-to-agent autonomy.

Problem examples:

- Read thread XYZ and summarize it.
- Read the latest report thread and discuss the findings.
- Use conclusions from architecture thread A in implementation thread B.

Core decision:

- Implement thread references, thread summaries, and thread artifacts before implementing thread messaging.
- Focus on knowledge transfer, not agent communication.
- Keep execution session-based.

Initial capabilities:

- Thread reference: read an existing thread by explicit user action.
- Thread summary: summarize an existing thread into decisions, findings, action items, and open questions.
- Thread artifact import: import reports, reviews, findings, or handoffs from another thread.
- Thread handoff: create a handoff from thread A and use that handoff in thread B.

Conceptual flows:

```text
Scheduled report thread
  -> produces report artifact
  -> interactive discussion thread
  -> reads report artifact
  -> performs analysis
```

```text
Architecture thread
  -> produces handoff
  -> implementation thread
```

Future capability:

- Sending a prompt to another thread may be evaluated only after references, summaries, handoffs, and artifacts prove useful.
- Cross-thread prompting must remain heavily audited because it introduces execution across thread boundaries.

Guiding principle:

- Threads are knowledge containers.
- Artifacts, summaries, and handoffs move knowledge between threads.
- Execution remains session-based.
- Schedules create sessions.
- Context Packs guide sessions.
- Receipts prove what happened.

Acceptance criteria:

- Thread summaries can be generated from existing threads.
- Artifacts can be imported between threads.
- Handoffs can move knowledge between threads.
- Audit and receipt systems record cross-thread references.
- No workflow engine, governance runtime, or agent orchestration framework is introduced.

## Planned: Additional Model Provider Adapters

Local Codex for Enterprise should remain local-first by default while allowing explicitly configured cloud or external model providers. The provider roadmap should extend the existing adapter approach instead of coupling the control plane to any one model vendor.

Planned adapter sequence:

1. Add a Gemini OpenAI-compatible provider path.
   - Reuse the existing `chat_completions` wire adapter where possible.
   - Treat this as the fastest validation path for Gemini support.
   - Keep it explicitly configured; do not make it a fallback or default.

2. Add a native Claude Messages adapter.
   - Introduce a dedicated Claude/Anthropic wire API instead of forcing Claude through the OpenAI-compatible adapter.
   - Translate Codex internal requests/events into Claude Messages API requests and streamed events.
   - Support system/developer context, user and assistant messages, tool schemas, tool-use blocks, tool results, text deltas, stop reasons, and usage metadata.
   - Keep Claude explicitly configured; do not add hidden cloud fallback.

3. Add a native Gemini adapter only if the OpenAI-compatible path proves limiting.
   - Candidate reasons include tool/function-call edge cases, multimodal support, long-context behavior, native streaming differences, or provider-specific safety/usage metadata.
   - Prefer keeping Gemini on the generic chat-completions path until there is a concrete product reason to add a second adapter.

Provider boundary:

- Docker Model Runner and Docker Model Gateway remain the default local-first provider path.
- Cloud providers are opt-in only.
- Provider adapters must preserve the Local-Only Contract: no OpenAI, Anthropic, Google, or other outbound model calls unless the user explicitly configures/selects that provider.
- Provider adapters should translate provider-specific wire formats into the existing Codex session, worker, trace, audit, and receipt model.
- Provider support is not model training, preference learning, or governance reasoning.

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
