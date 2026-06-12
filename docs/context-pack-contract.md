# Context Pack Contract

Context Packs are versioned lifecycle packages for Local Codex for Enterprise.

They are not themselves Codex skills, executable workflows, or governance runtimes.

Context Packs bundle durable operating material that tells Codex what to know and how to operate inside a user, team, project, workspace, or organization. They may contain or reference Codex skill files as governed package contents, but importing or loading a Context Pack does not automatically execute skills, call tools, create sessions, alter RBAC, or run governance reasoning.

## Responsibilities

A Context Pack may contain:

- project knowledge
- user calibration
- operating instructions
- coding standards
- task procedures
- workflow guidance
- verification checklists
- handoffs
- escalation guidance
- reusable prompt templates
- reporting expectations
- role expectations
- team conventions
- organizational standards
- optional Codex skill files or skill references

Context Packs answer:

- What should the agent know?
- How should the agent operate here?

## Standard File Structure

Context Packs intentionally use standardized Markdown filenames for portability, discoverability, consistency, community sharing, and reusable templates.

Canonical files:

- `PACK.md`
- `CALIBRATION.md`
- `OPERATING-INSTRUCTIONS.md`
- `PROJECT-RULES.md`
- `WORKFLOWS.md`
- `VERIFICATION.md`
- `HANDOFF.md`
- `ESCALATION.md`
- `CONTEXT.md`
- `PROMPTS.md`

Only `PACK.md` is universally required. All other canonical files are optional unless the pack manifest requires them. Custom uppercase Markdown files are allowed when a team needs additional structure.

The intended authoring flow is:

```text
Copy Pack -> Edit Pack -> Assign Pack -> Use Pack
```

## Stored Files And Loaded Context

Context Packs may store more than Markdown documents. A pack can contain documents, bundle files, assets, templates, scripts, or skill-style file trees as package contents.

Stored files can be hashed, downloaded, audited, and preserved as evidence. Storage does not mean execution.

Only active files marked as loadable participate in session context loading. Loadable files should be text-like operating material such as Markdown, text, JSON, TOML, or YAML. Assets, scripts, binaries, and other non-loadable files may remain in the package for provenance or download, but they are not injected into prompts.

Path safety is part of the contract:

- no absolute paths
- no traversal such as `..`
- no empty path segments
- no backslashes
- no control characters
- hidden path segments are blocked by default unless a future explicit policy allows them

Deletion is soft deletion. Removing a file changes future context loading, but historical receipts remain queryable.

## PACK.md Manifest

`PACK.md` is the pack manifest. It may define:

- pack name
- version
- description
- required documents
- load order
- prompt templates
- metadata
- categories or tags

Example:

```yaml
name: Management Operating Pack
version: 1.0
required_documents:
  - OPERATING-INSTRUCTIONS.md
load_order:
  - PACK.md
  - CALIBRATION.md
  - OPERATING-INSTRUCTIONS.md
  - PROJECT-RULES.md
  - WORKFLOWS.md
  - PROMPTS.md
templates:
  - weekly-management-report
  - inventory-review
```

The current MVP validates required documents and deterministic load order. Additional manifest metadata is contract-level direction for future schema work.

## Non-Executable Boundary

Context Packs may describe workflows and procedures.

Example:

```text
When performing inventory analysis:
1. Review inventory data.
2. Review sales trends.
3. Compare against reorder thresholds.
4. Generate recommendations.
```

This is guidance only.

Context Packs do not:

- execute code
- trigger schedules
- call MCP tools
- create workers
- create sessions
- alter RBAC
- perform governance reasoning
- dispatch agents
- act as workflow engines

Execution happens through sessions and workers.

## Prompt Templates

Context Packs may include reusable prompt templates in `PROMPTS.md`.

Prompt templates are reusable text assets. They do not execute by themselves.

Interactive sessions or future scheduled sessions choose whether to use a template. The pack can offer the template, but the session decides what runs.

## Scheduled Sessions

Automation must be represented as scheduled sessions, not as Context Pack execution.

```text
Schedule
  -> creates session
  -> loads Context Packs
  -> starts worker
  -> executes prompt or selected template
  -> produces artifact
  -> stores receipts
```

A schedule never executes a Context Pack. A schedule creates a session. The session loads Context Packs. The worker executes the task.

## Architecture Vocabulary

Use this terminology consistently:

- Context Pack = operating package
- Session = execution unit
- Worker = runtime
- Schedule = trigger
- Receipt = evidence

Everything that can be represented as a session should be a session. Interactive work is a user-created session. Scheduled work is a scheduler-created session. Both use the same worker, trace, audit, receipt, and Context Pack loading architecture.

## Receipts

Receipts prove which operating package influenced a run.

Receipts should record:

- pack id
- pack version
- document id
- document hash
- load order
- prompt template identity when used
- assignment source
- session id
- worker id when applicable
- trace id

Receipts must not store:

- private/runtime prompts
- model outputs
- passwords
- tokens
- auth headers
- credentials
- private secrets

## Relationship To Codex Skills

Codex skills are runtime capability packages for agent behavior, procedural tool use, or local assistant extensions.

Context Packs are enterprise-owned lifecycle packages. They need user, workspace, project, assignment, receipt, audit, and RBAC surfaces. Those are enterprise control-plane concerns, not local runtime skill installation concerns.

Local Codex for Enterprise does not treat a Context Pack as a Codex skill because skills alone do not provide the correct tenancy, assignment, audit, or receipt boundary for this product.

Skills can still exist in the underlying Codex ecosystem, and a Context Pack may include or reference skill files. Runtime activation of any included skill must be explicit, auditable, and permissioned. Pack import, assignment, or session loading is not the same as skill execution.
