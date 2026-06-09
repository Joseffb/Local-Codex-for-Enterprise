# Verification

Minimum verification for enterprise-server changes:

```sh
just test -p codex-enterprise-server
just fmt-check
git diff --check
```

Release-readiness verification should also include Docker Compose startup, `/healthz`, browser login/session/worker validation, gitleaks, and focused private-path grep.
