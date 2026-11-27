# cloneyard-provider-claude

Claude Code as a provider: what Cloneyard knows about starting `claude` for an account, shared by the CLI and the daemon.

## Scope

- `command`: `claude` with an account's directory as its `CLAUDE_CONFIG_DIR`, and without the `ANTHROPIC_*` and `CLAUDE*` variables, which would log it in some other way or tie it to a surrounding session.
- `settings`: a clone's `--settings`, with a hook for each event in `hook::EVENTS` that runs `cy hook <event>` and `"disableAllHooks": false`, so no settings file in the checkout turns them off; inside Cloneyard's sandbox it also turns Claude Code's own sandbox off, whose `bwrap` can't start there.
- `inside_sandbox`: turns off the auto-updater through `DISABLE_AUTOUPDATER`, as it would replace the `claude` that runs outside the sandbox.
- `headless::command`: `claude -p` with stream-json in and out on a given session id and settings, with `--permission-prompts none`, so a tool that would ask for permission is denied at once; `headless::user_message` is the line that sends it a message.
- `headless::parse`: the events on its stdout that a clone's state needs, `system/init` with the session id and `result`, which is an error whenever `is_error` is true, whatever its `subtype` says.
- `hook::parse`: what a hook's JSON tells, its session, its tool, the id of its tool call with the command of `Bash` or the path of a file tool, and the state it gives a clone (`Notification` with `permission_prompt` gives blocked, `Stop` idle, a prompt or a tool hook working), and the JSON without `tool_response`, which can hold a whole file.

## Out of scope

- Running and supervising the process, which the CLI does for a login and the daemon for a clone.
- Other providers, each of which gets a crate of its own.
