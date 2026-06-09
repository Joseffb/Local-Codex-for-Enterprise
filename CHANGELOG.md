# Changelog

All notable changes to Local Codex for Enterprise will be documented in this file.

This project is a community fork and is not affiliated with, endorsed by, or supported by OpenAI.

## [Unreleased]

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
