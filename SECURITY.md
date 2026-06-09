# Security Policy

Local Codex for Enterprise is an experimental community fork and is not affiliated with, endorsed by, or supported by OpenAI.

## Supported Versions

No public stable release is supported yet. Security review is focused on the upcoming `v0.1.0` release candidate.

| Version | Supported |
| --- | --- |
| `v0.1.x` | Planned |
| `< v0.1.0` | No |

## Reporting a Vulnerability

Until this project has a public vulnerability disclosure program, please report issues through the GitHub repository owner using a private channel. Do not open public issues containing exploit details, secrets, private paths, tokens, screenshots with private data, or working proof-of-exploit material.

If the issue is in upstream OpenAI Codex rather than this fork, follow OpenAI's upstream security process for the original project.

## Security Boundaries

The enterprise server is responsible for:

- Authentication.
- Seeded RBAC authorization.
- Workspace allowlisting.
- Worker supervision.
- Handoff token issue and consume.
- Trace-aware audit events.
- Execution receipts.

The enterprise server must not persist:

- Prompts.
- Model outputs.
- Auth headers.
- Handoff JWTs.
- Plaintext passwords.
- API tokens.
- Repository credentials.
- Environment secrets.
- Private real-life examples.

Receipts are evidence, not reasoning. They record IDs, hashes, outcomes, and redacted metadata only.

## Local Deployment Expectations

This project is designed for self-hosted local or private-network evaluation. Do not expose the enterprise server directly to the public internet without external TLS termination, network access controls, secret rotation, and an independent security review.

## Before Public Release

Run at least:

```sh
just test -p codex-enterprise-server
just fmt-check
git diff --check
gitleaks dir . --verbose --no-banner
git grep -nE '<private-path-or-secret-patterns>'
```

Review all matches before publishing.
