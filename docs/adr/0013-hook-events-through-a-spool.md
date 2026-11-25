# Hook events reach the daemon only through a spool

A clone's hooks reach the daemon by one path, whether the daemon runs or not: a spool, `clones/<id>/spool` in the data dir, inside the clone's own directory. The provider gives `claude` a hook for each event Cloneyard listens to in `--settings`, which runs the daemon's own binary as `cy hook <event>`, with `CLONEYARD_SPOOL` naming the spool. `cy hook` writes the JSON on stdin as `.<name>.tmp` and renames it to `<name>.json`, where the name is the time in nanoseconds, the pid and the event, so the names sort in the order the hooks ran. It prints nothing, as Claude Code adds the stdout of `SessionStart` and `UserPromptSubmit` to the session, and it always exits 0. Claude Code waits for each hook, up to 10 s, so one session's hooks run in order. `--settings` also says `"disableAllHooks": false`: it outranks the settings files in the checkout, which Claude Code reloads during a session, so neither the repo nor the clone turns the hooks off.

The daemon watches the spool of each running clone with inotify, through `rustix`, but only as a wake-up: it lists the spool and takes its `*.json` files in name order. Each event goes into the append-only `hook_event` table of [ADR 0008](0008-entity-change-log.md) in one transaction with its session, when that is new, and the state it gives a running clone; the daemon deletes the file after the commit. The file name is unique per clone, so a file read again after a crash between the commit and the delete adds nothing. At start the daemon marks the lost clones and then takes what every spool holds. The daemon knows the clone from the directory, not from the payload: a file that names another clone's session is dropped, like one that isn't a hook's JSON or holds more than 16 MiB. The payload is stored without `tool_response`, which can hold a whole file.

## Considered options

- **Claude Code's HTTP hooks to the daemon**: an event is lost whenever the daemon is down or restarts, and the daemon would have to tell the clone from the request.
- **HTTP hooks with a spool for when the daemon is down**: two paths, whose events would have to be put back in one order.
- **Datagrams on a Unix socket, with a spool as a fallback**: a datagram holds about 200 KB, less than the input of a `Write` of a large file, and the daemon would know the clone only from the payload.
- **A descriptor the clone inherits**: the sandbox of [ADR 0012](0012-clone-sandbox-of-landlock-and-seccomp.md) closes every descriptor but stdin, stdout and stderr, and a descriptor dies with the daemon, so it reaches no clone that outlives it.

## Consequences

- An event reaches the database as soon as inotify wakes the daemon, or at the daemon's next start; a spool grows on disk while the daemon is down.
- A clone can forge or delete its own events, but no other clone's, as the sandbox lets it write only in its own directory; an unsandboxed clone can write any spool.
- A clone's state follows its hooks while it runs: `Notification` with `permission_prompt` gives `blocked`, `Stop` gives `idle`, and a prompt or a tool hook gives `working` again. A hook never changes `done`, `error` or `stopped`, so a `Stop` that arrives after the `result` leaves a headless clone `done`.
- The sandbox makes the daemon's binary readable to every clone, as the hooks run it even when it lies in a hidden repo.
- Every hook starts a `cy` process, which writes one file.
- The `hooks_off` test of `tools/spike` fails once a checkout's `disableAllHooks` wins over `--settings`.
