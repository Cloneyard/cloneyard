# Contributing to Cloneyard

Issues, commits, pull requests and docs are written in English.

## Workflow

Work goes through GitHub issues, one per feature or bug fix, in three stages. Each stage starts after the maintainer's OK.

1. **Discussion**: the issue body says what we build, what will be visible and what is out of scope.
2. **Plan**: a comment on the issue with the steps, files and tests.
3. **Pull request**: one small PR per issue, with `Closes #<number>` as the last line of its description.

Open an issue before you write code, so the scope is agreed first.

## Contributor Licence Agreement

You sign the Cloneyard CLA once, on your first pull request: CLA Assistant comments on it with a link, and the signature covers all your later contributions. The CLA lets the project relicense contributions; [ADR 0001](docs/adr/0001-agpl-only-with-cla.md) explains why.

## Commits

- [Conventional Commits](https://www.conventionalcommits.org): `type(scope): description`, with the types `feat`, `fix`, `chore`, `refactor`, `docs`, `test`, `style`, `perf`, `build` and `ci`. The scope names the one module or area the change is in; leave it out when the change spans several.
- The subject is at most 50 characters.
- A body only when the reason for the change doesn't follow from the diff.
- Atomic: one logical change per commit, together with its tests.
- No AI attribution: no `Co-Authored-By` trailer for an agent and no "Generated with" footer.

## Before you open a pull request

CI runs these three checks; run them locally first:

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```
