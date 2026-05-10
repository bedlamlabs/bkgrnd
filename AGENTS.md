# bkgrnd Codex Instructions

Global Codex and Hosaka rules from `/Users/geoffmccaleb/AGENTS.md` apply here.

Read local project docs and config before making changes, especially `hosaka.config.yaml`, `CLAUDE.md`, and package scripts when present.

## Project Superpowers

Local Codex superpowers for bkgrnd live under `.codex/`.

- Use `.codex/skills/project-superpowers` to route specialized work.
- `ux-ui-builder` is for implementing UX/UI changes; `ux-ui-reviewer` is read-only review.
- `cloud-deployment-worker` may implement deployment or CI changes but must not run deployment git commands.
- `github-orchestrator` owns deployment git commands and deployment branch/release orchestration.
- `helga-qa` is the adversarial QA agent; when Qwen3 is configured, start it only when needed and follow the project safety valve.
- `security-worker` implements scoped hardening; `security-reviewer` reviews without editing.

Hosaka evidence files and gate artifacts must be produced only by the owning Hosaka scripts. Use `/monitor` or project monitor tooling for long CI/deploy waits where configured.

