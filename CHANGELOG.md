# Changelog

All notable changes to Local Codex for Enterprise will be documented in this file.

This project is a community fork and is not affiliated with, endorsed by, or supported by OpenAI.

## [Unreleased]

## [0.0.1-beta.8] - 2026-06-12

### Added

- Added beta Scheduled Sessions as ordinary `scheduled` sessions created on behalf of a user.
- Added Postgres schedule, schedule Context Pack, and schedule run storage.
- Added UTC 5-field cron validation, scheduler polling config, run-now, run history, stale-run timeout handling, and scheduled-session admin UI.
- Added `smoke` scheduled runner mode as the visible beta/default deterministic validation path.

### Security

- Schedule metadata and receipts record IDs, status, type, runner mode, and provenance only; prompts, model outputs, tokens, credentials, and private examples are not stored in audit metadata.
- Context Packs are loaded by scheduled sessions but are not executed as workflows, schedulers, governance runtimes, or automatic skill activations.

## [0.0.1-beta.7] - 2026-06-12

### Added

- Added cross-thread knowledge references for transcript exports, handoffs, output imports, and AI summaries.
- Added server-generated Markdown output artifacts for bounded transcript exports and handoffs.
- Added `/chat` knowledge-transfer UI for selecting accessible threads/outputs without dumping raw transcript content into the conversation.

### Security

- Thread reference metadata records provenance only: IDs, type, origin, status, truncation, and timestamps.
- AI summaries run through the current target thread worker; no source-thread worker, worker-to-worker messaging, workflow runtime, or background execution was added.

## [0.0.1-beta.3] - 2026-06-09

### Changed

- Improved browser chat turn controls with copy, resubmit, and inline edit/reset behavior.
- Preserved persisted turn timestamps when rendering loaded thread history.
- Added default-thread auto-labeling after the first user/assistant exchange.
- Routed greetings and acknowledgements to brief conversational replies instead of conceptual planning mode.

## [0.0.1-beta.2] - 2026-06-09

### Changed

- Added the Context Pack contract, defining packs as versioned operating packages rather than Codex skills, executable workflows, or governance runtimes.
- Added canonical `WORKFLOWS.md` and `PROMPTS.md` Context Pack files plus custom uppercase Markdown import support.
- Added server-provided chat turn guidance for conceptual planning and collapsed raw tool output in the browser transcript.

## [0.0.1-beta.1] - 2026-06-09

### Fixed

- Fixed browser chat thread rename failures caused by a missing PATCH JSON helper.
- Fixed long chat replies so the transcript scrolls inside the browser workbench instead of expanding the page.
- Rendered Codex replies as readable Markdown for headings, lists, inline code, bold text, and code blocks.
- Added a no-repository conceptual planning guardrail so concept prompts do not default to repository inspection unless requested.

## [0.0.1-beta.0] - 2026-06-09

### Added

- Enterprise control-plane MVP with first-run owner setup, password login, browser cookie auth, seeded RBAC role assignment, workspace registration, session records, worker supervision, handoff tokens, trace-aware audit events, and execution receipts.
- Workflow Context Pack validation, assignment, and load receipts.
- Axum-served web shell for setup, login, admin, context packs, sessions, and audit.
- Docker Compose stack for local enterprise evaluation with Postgres.
- Public-release documentation, example context pack, example receipts, demo guide, and v0.1.0 release checklist.
- Explicit enterprise domain contract for the workspace root -> user workspace -> project -> repository -> thread hierarchy.

### Changed

- Docker image now builds the Codex CLI, enterprise server, and app server for enterprise evaluation.
- Product language now treats workspace roots and user workspaces as filesystem boundaries, projects as work containers, repositories as checkouts, and threads as chat histories.

### Security

- Receipts and audit metadata are documented as redacted evidence only.
- Public release docs state that prompts, model outputs, auth headers, handoff tokens, passwords, API tokens, repo credentials, and private examples must not be persisted.

## [0.1.0] - Planned

Initial public release candidate for local enterprise evaluation.
