# Tool calls take their command and path from the hooks

`cy usage tools` counts a clone's tool calls from the `tool_result` and `tool_decision` records of [ADR 0014](0014-telemetry-through-a-local-otlp-receiver.md), and takes what a call is about from its `PreToolUse` hook event of [ADR 0013](0013-hook-events-through-a-spool.md). Both carry Claude Code's `tool_use_id`, which the provider reads from each tool hook at ingest into `hook_event`, with the command of `Bash` or the path of `Read`, `Write`, `Edit` and `NotebookEdit`. `OTEL_LOG_TOOL_DETAILS` stays off, so the telemetry still holds no tool input. A denied call has a `reject` and no `tool_result`, so it counts as a call and a denial, never as an error.

A row groups the commands that start with the same program and first argument, unless that is an option or a path, after leading `NAME=value` words and `cd <dir> &&`. It groups the files by their path in the clone's checkout, so one file in two clones is one row.

## Considered options

- **`OTEL_LOG_TOOL_DETAILS`**: it would put each call's input into the telemetry, which ADR 0014 keeps out of it, while the hook already holds that input.
- **Reading the hook's JSON when counting**: the store would have to know the shape of Claude Code's payload.
- **The full command as the row**: each call with other arguments would be a row of its own.
- **The program alone**: `gh api` and `gh pr view` would share a row, though their results differ in size.

## Consequences

- A call whose hook never reached the spool, or was recorded before migration `0009`, counts under its tool alone.
- A clone can forge its own hooks, so it can also give its calls another command or path.
- A retry is a call whose previous call of the same row in its session failed, so a command run again after a fix counts as one.
- The `telemetry` test of `tools/spike` fails once a `PreToolUse` hook no longer carries the `tool_use_id` of its call's telemetry, or a denied call no longer gives a `reject`.
