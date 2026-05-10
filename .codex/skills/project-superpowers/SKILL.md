---
name: project-superpowers
description: Route project work to local Codex subagents for implementation, review, deployment orchestration, security, UX/UI, and adversarial QA.
---

# Project Superpowers: bkgrnd

Use this skill when work in this project benefits from a specialized local Codex subagent.

## Routing

- UX/UI implementation: use `ux-ui-builder`.
- UX/UI review: use `ux-ui-reviewer`.
- Cloud, CI, hosting, or deployment implementation: use `cloud-deployment-worker`.
- Deployment git orchestration: use `github-orchestrator`.
- Security implementation: use `security-worker`.
- Security review: use `security-reviewer`.
- Adversarial QA: use `helga-qa`.

## Rules

- Read this project's `AGENTS.md` and `hosaka.config.yaml` when present.
- Hosaka remains the source of truth for gated workflows and evidence.
- Deployment git commands belong to `github-orchestrator`; implementation agents must not run them.
- `/monitor` or the project monitor script should be used for long CI/deploy waits where available.
- Helga/Qwen3 should only be started when needed and stopped according to the project safety valve.
- Never fabricate evidence JSON or hand-write Hosaka gate artifacts unless the owning script explicitly permits it.
