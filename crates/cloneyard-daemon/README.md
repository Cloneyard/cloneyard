# cloneyard-daemon

The `cloneyard daemon` process on a node: it reads the node's settings, holds the single-instance locks, opens the node's database and serves the API on a Unix socket.

## Scope

- The settings in `$XDG_CONFIG_HOME/cloneyard/node.toml`, or `~/.config/cloneyard/node.toml`; a missing file means the defaults.
- The runtime dir `$XDG_RUNTIME_DIR/cloneyard/`, mode 0700, with `daemon.sock` and `daemon.lock`.
- The data dir `$XDG_DATA_HOME/cloneyard/`, or `~/.local/share/cloneyard/`, mode 0700, with `cloneyard.db`, `cloneyard.lock`, `backups/`, `clones/<id>/`, each clone's own directory, and `caches/`, which the clones share.
- The `system.*`, `project.*`, `account.*` and `clone.*` methods and `rpc.discover`, registered through their types in `cloneyard-api` on the node's id and database, which the handlers share; `system.ping` answers with the node id, and `rpc.discover` with the OpenRPC document.
- `project.add` takes the repo at the top level that `git rev-parse --show-toplevel` finds for the path, with symlinks resolved, so it needs `git` on its `PATH`.
- `account.add` records an account with its directory, with symlinks resolved; it never runs the provider, since the login in the directory is the CLI's.
- `clone.run` records the clone, makes its checkout with `git clone --shared` and `git switch --create`, and starts the headless `claude` there in a process group of its own, so it needs `claude` on its `PATH`. It starts `claude` in the sandbox, through `/proc/self/exe sandbox`: the clone can't read the secrets in the home, the data dir or the node's accounts and repos, apart from its own directory, its account's and the objects its checkout borrows, and writes only there, in `caches/`, `/tmp` and `/dev`. On a kernel without Landlock it refuses with `NoSandbox`, unless `node.toml` says `unsandboxed = true`, which starts clones without the sandbox. A task per clone reads its stream-json, records its sessions and its state, and closes its stdin after the first `result`.
- `clone.stop` sends SIGTERM to the clone's process group and SIGKILL 5 s later; on SIGTERM the daemon does the same to every running clone, which ends `error`, and at start it sets the clones still `working` to `error`.
- Logs through `tracing` to stderr, and a clean shutdown on SIGTERM and SIGINT.

```toml
unsandboxed = false # true starts clones without the sandbox

[log]
level = "info" # a tracing filter; RUST_LOG overrides it
```

## Out of scope

- The protocol, which lives in `cloneyard-rpc`.
- The methods' names and types, and the OpenRPC document, which live in `cloneyard-api`.
- The database and its schema, which live in `cloneyard-store`.
- Arguments and output for people, which live in `cloneyard-cli`.
- The arguments and output of `claude`, which live in `cloneyard-provider-claude`.
- Landlock and seccomp, which live in `cloneyard-sandbox`.

[ADR 0010](../../docs/adr/0010-clone-ids-and-handles.md) explains the ids and handles of clones, [ADR 0011](../../docs/adr/0011-clone-checkout-with-git-clone-shared.md) their checkouts, and [ADR 0012](../../docs/adr/0012-clone-sandbox-of-landlock-and-seccomp.md) their sandbox.
