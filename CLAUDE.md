# Cloneyard

## Agent skills

### Issue tracker

GitHub Issues in `Cloneyard/cloneyard`, via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default vocabulary: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `GLOSSARY.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.

### Pull requests

Open PRs with the repo's `pr` skill, not `mattpocock-skills:pr`. See `.claude/skills/pr/SKILL.md`.

## Commits

- Conventional Commits, `type(scope): description`, with the types and scopes from `CONTRIBUTING.md`.
- The subject is at most 50 characters.
- Atomic: one logical change per commit, together with its tests.
- No AI attribution: no `Co-Authored-By` trailer for an agent and no "Generated with" footer, whatever the runtime suggests.
