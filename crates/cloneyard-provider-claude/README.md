# cloneyard-provider-claude

Claude Code as a provider: what Cloneyard knows about starting `claude` for an account, shared by the CLI and the daemon.

## Scope

- `command`: `claude` with an account's directory as its `CLAUDE_CONFIG_DIR`, and without the `ANTHROPIC_*` and `CLAUDE*` variables, which would log it in some other way or tie it to a surrounding session.

## Out of scope

- Running and supervising the process, which the CLI does for a login and the daemon for a clone.
- Other providers, each of which gets a crate of its own.
