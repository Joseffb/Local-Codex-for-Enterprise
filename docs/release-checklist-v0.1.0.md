# v0.1.0 Release Checklist

Use this checklist before making the repository public.

## Product Framing

- [ ] README states what the project is.
- [ ] README states what the project is not.
- [ ] README includes OpenAI attribution and community-fork disclaimer.
- [ ] README avoids unsupported production/security claims.
- [ ] License and NOTICE are present.

## Documentation

- [ ] README quick start works from a clean checkout.
- [ ] Docker Compose setup is documented.
- [ ] Architecture overview is documented.
- [ ] Workflow Context Packs are explained.
- [ ] Example Context Pack is synthetic and safe.
- [ ] Example receipts contain IDs, hashes, and redacted metadata only.
- [ ] Demo guide covers browser/session/worker/local-model validation.
- [ ] SECURITY.md is fork-specific.
- [ ] CONTRIBUTING.md is fork-specific.
- [ ] CHANGELOG.md has an unreleased section.
- [ ] THREAT_MODEL.md is current.

## Safety

- [ ] No prompts are committed.
- [ ] No model outputs are committed.
- [ ] No auth headers are committed.
- [ ] No handoff tokens are committed.
- [ ] No passwords are committed.
- [ ] No API tokens are committed.
- [ ] No repository credentials are committed.
- [ ] No private local paths are committed.
- [ ] No screenshots with private data are committed.
- [ ] No private real-life examples are committed.

## Validation

- [ ] `just test -p codex-enterprise-server`
- [ ] `just fmt-check`
- [ ] `git diff --check`
- [ ] `docker compose -f compose.enterprise.yaml config`
- [ ] `docker compose -f compose.enterprise.yaml build enterprise`
- [ ] `docker compose -f compose.enterprise.yaml up -d postgres enterprise`
- [ ] `curl -fsS http://127.0.0.1:8787/healthz`
- [ ] Browser login/session/worker path verified.
- [ ] Local Docker model code-change proof verified.
- [ ] App-server model catalog text is fork-safe and does not imply an official OpenAI product.
- [ ] `gitleaks dir . --verbose --no-banner`
- [ ] Focused private-path grep reviewed.

## Public Release Decision

- [ ] Remaining blockers are documented.
- [ ] Demo status is documented.
- [ ] Repository visibility change is intentional.
- [ ] `v0.1.0` tag or release candidate tag is prepared.
