# Clones run in a sandbox of Landlock and seccomp

Every clone runs in Cloneyard's own sandbox, the same boundary for every mode and provider. The daemon starts a clone as `/proc/self/exe sandbox <policy> -- claude …`: that process applies Landlock and two seccomp filters to itself, then runs `claude`, so the clone and everything it starts stay inside and the daemon stays outside. It marks every descriptor but stdin, stdout and stderr close-on-exec first, since the daemon can inherit some from whatever started it, so `claude` inherits none of them.

Landlock lets a clone read everything except the hidden paths: the secrets in the home (`~/.ssh`, `~/.config/gh`, `~/.claude` and others), the data dir, and the directory of every account and repo on the node. Its own directory, its account's directory and the objects its checkout borrows through `alternates` stay visible. It writes only in its own directory, checkout included, its account's directory, `caches` in the data dir, `/tmp` and `/dev`. The first Landlock ABI is required; the later rights and the scopes of signals and abstract sockets apply where the kernel has them. The rules target ABI v6, because v9's `ResolveUnix` would also stop datagram sockets outside the writable paths.

The filters make `socket(AF_UNIX, …)` fail with `EACCES` unless the type is `SOCK_DGRAM`, and `io_uring_setup` fail with `EPERM`, since io_uring opens sockets without the syscall; the x32 numbers of both are filtered too, and a syscall of another ABI kills the process. So a clone reaches neither Docker, systemd, tmux, an SSH agent nor the daemon's socket.

Claude Code's native sandbox gives way. Its `bwrap` fails inside Landlock, which forbids changing mounts (`Failed to make / slave: Operation not permitted`), and its proxy can't listen on a Unix socket under the filter. Claude Code then turns its sandbox off for the session, and Bash needs an approval that a headless clone can't get. So the provider runs `claude` in a clone with `"sandbox": {"enabled": false}` in `--settings`, and with `DISABLE_AUTOUPDATER=1`, as the updater would write the binary that runs outside.

## Considered options

- **`pre_exec` in the daemon**: it is `unsafe`, which the workspace forbids, and after a fork of the multithreaded daemon only async-signal-safe calls are allowed, while building the rules allocates.
- **Claude Code's native sandbox as the only layer**: it covers only Claude Code, and a repo's own settings can turn it off.
- **Our own namespaces, as `bwrap` makes them**: they would hide paths inside writable trees, but on Ubuntu 24.04 AppArmor blocks unprivileged user namespaces for a binary without a profile of its own.
- **The user's caches writable**: a crate source changed in `~/.cargo/registry` would run in the user's next `cargo build` outside the sandbox.
- **Only the clone's own repo and account hidden**: another project's `.env` is as secret, and another account's directory holds its OAuth token.

## Consequences

- On a kernel without Landlock, clones start only with `unsandboxed = true` in `node.toml`, which `cy clones` shows; the clones started before this sandbox are marked `unsandboxed` too.
- A hidden path inside a writable one, such as a repo in `/tmp`, can't be hidden, so a clone of it ends `error`.
- A clone still sees its account's login, which Claude Code needs, the daemon's environment except `ANTHROPIC_*` and `CLAUDE*`, TCP and UDP, localhost included, the files of other processes in `/tmp`, and the names of the files in hidden directories. On a kernel older than 6.12 it can signal the user's other processes.
- An account's directory is its clones', like a checkout's `.git`: Cloneyard runs `claude` on it outside the sandbox only for the login of `cy account add`, before any clone used it.
- `git fetch` and `git push` to `origin` fail in a checkout, since the repo is hidden apart from its objects.
- Clones share caches of their own, so their first build downloads the dependencies again, and a clone can poison the caches of other clones but not the user's.
- The `bwrap` test of `tools/spike` warns once `bwrap` runs inside Landlock, the point to revisit this decision.
