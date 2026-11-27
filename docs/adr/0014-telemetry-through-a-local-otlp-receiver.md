# Clones send their telemetry to a receiver on 127.0.0.1, with a token each

The tokens and cost of a clone come from Claude Code's OpenTelemetry, which the daemon receives itself: an OTLP/HTTP JSON receiver on `127.0.0.1`, on the port that `[telemetry] port` in `node.toml` sets, 7318 by default. The daemon binds it at start next to its socket, so a port in use stops the start. The provider starts each clone with telemetry on and its logs going as `http/json` to the receiver's `/v1/logs`, every second. It sends no metrics or traces, puts `127.0.0.1` in `NO_PROXY` and passes on none of the `OTEL_*` variables of the daemon's environment.

Each clone gets a random token at start, which it sends as a bearer token from `OTEL_EXPORTER_OTLP_HEADERS`. The daemon holds the tokens in memory and refuses a clone's token once its process ends, so a request without the token of a running clone gets 401. The token tells the receiver which clone sent an export, not the `session.id` in it. It sits in the environment because the sandbox keeps a clone from reading another clone's `/proc/<pid>/environ`, while any process reads `/proc/<pid>/cmdline`, which holds `--settings`. Claude Code passes no `OTEL_*` variable on to its tools or hooks.

The receiver keeps `api_request`, `api_error`, `tool_result` and `tool_decision`, each in an append-only table of [ADR 0008](0008-entity-change-log.md), with only the attributes Cloneyard counts, so `user.email`, `user.account_uuid` and `organization.id` never reach the database. Claude Code sends an export again when it gets no answer, so each table is unique per clone by `request_id` or `tool_use_id`; `event.sequence` counts per process and can't serve. A record of another clone's session is dropped while the rest of its export is kept, and the answer counts the dropped records as `partialSuccess`. A database error gets 503, which Claude Code retries.

## Considered options

- **A Unix socket**: Claude Code's exporter sends only to an HTTP URL, and the sandbox of [ADR 0012](0012-clone-sandbox-of-landlock-and-seccomp.md) blocks Unix stream sockets.
- **An OpenTelemetry Collector**: one more program to install and run, only to pass the data on.
- **The `result` of stream-json**: it has the cost of a headless turn, but nothing of an interactive clone and nothing per tool.
- **`session.id` to tell the clone**: any process on the node could report under another clone's session.
- **A port per clone**: as many listeners as clones, which still need to tell a clone from any other process on localhost.
- **The token in `--settings`**: other clones would read it from the command line.
- **A free port at each start**: a clone that outlives the daemon keeps the endpoint it started with. Port 0 still takes one, for tests that run side by side.
- **Tokens in the database**: a running clone doesn't outlive the daemon yet, so neither does its token.

## Consequences

- A repo's `.claude/settings.json` can neither send a clone's telemetry elsewhere nor turn it off, as Claude Code ignores the exporter variables there when the environment sets them. Managed settings with an OpenTelemetry endpoint of their own still win, and their clones report no usage.
- A clone can forge the records of its own sessions, but no other clone's; an unsandboxed clone can read the other clones' tokens.
- On a subscription, the cost is Claude Code's estimate at API prices.
- An export that Claude Code sends while the daemon is down is lost once its retries run out.
- axum brings `matchit` under BSD-3-Clause, which joins the allow list of [ADR 0006](0006-permissive-dependency-licences.md).
- The `telemetry` test of `tools/spike` fails once Claude Code changes the variables, the header, the last export before it exits or the attributes this relies on.
