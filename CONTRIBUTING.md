# Contributing

Thanks for considering a contribution to Local Codex for Enterprise.

This project is a community fork and self-hosted extension of OpenAI Codex. It is not affiliated with, endorsed by, or supported by OpenAI.

## Project Scope

In scope for the enterprise fork:

- Local-first Docker Model Runner and Docker Model Gateway support.
- Enterprise setup, auth, seeded RBAC, workspace allowlisting, sessions, workers, handoffs, audit events, and receipts.
- Workflow Context Packs as Markdown instruction/context material.
- Release-readiness documentation, demos, and safety checks.

Out of scope for the current release:

- SSO/SAML/OIDC.
- Custom RBAC policy editor.
- Approval workflow engine.
- Fernain bridge.
- Governance reasoning runtime.
- General policy orchestration.
- Full browser IDE polish.

## Safety Rules

Never commit:

- Prompts.
- Model outputs.
- Auth headers.
- Handoff tokens.
- Passwords.
- API tokens.
- Repository credentials.
- Private local paths.
- Screenshots with private data.
- Real personal examples.

Receipts and audit metadata must store redacted evidence only.

## Development Workflow

Before changing code, read [AGENTS.md](AGENTS.md) and follow the project validation contract.

For enterprise-server changes, run:

```sh
just test -p codex-enterprise-server
just fmt
just fmt-check
git diff --check
```

For release-readiness work, also run:

```sh
gitleaks dir . --verbose --no-banner
git grep -nE '<private-path-or-secret-patterns>'
```

## Documentation

Public docs must:

- Clearly identify this as a community fork.
- Preserve OpenAI Codex attribution.
- Avoid unsupported security claims.
- Avoid private paths and personal examples.
- Keep examples synthetic and reusable.

## Pull Requests

A release-ready pull request should include:

- Summary of changes.
- Validation commands and results.
- Security/privacy review notes.
- Any remaining limitations or blockers.
