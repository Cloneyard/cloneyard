# cloneyard-provider-claude

Claude Code as a provider: what Cloneyard knows about starting `claude` for an account, shared by the CLI and the daemon.

## Scope

- `command`: `claude` with an account's directory as its `CLAUDE_CONFIG_DIR`, and without the `ANTHROPIC_*` and `CLAUDE*` variables, which would log it in some other way or tie it to a surrounding session.
- `headless::command`: `claude -p` with stream-json in and out on a given session id, with `--permission-prompts none`, so a tool that would ask for permission is denied at once; `headless::user_message` is the line that sends it a message.
- `headless::inside_sandbox`: turns off what can't work inside Cloneyard's sandbox, Claude Code's own sandbox through `--settings` and its auto-updater through `DISABLE_AUTOUPDATER`.
- `headless::parse`: the events on its stdout that a clone's state needs, `system/init` with the session id and `result`, which is an error whenever `is_error` is true, whatever its `subtype` says.

## Out of scope

- Running and supervising the process, which the CLI does for a login and the daemon for a clone.
- Other providers, each of which gets a crate of its own.
