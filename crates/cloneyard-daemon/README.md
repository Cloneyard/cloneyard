# cloneyard-daemon

The `cloneyard daemon` process on a node: it reads the node's settings, holds the single-instance locks, opens the node's database and serves the API on a Unix socket.

## Scope

- The settings in `$XDG_CONFIG_HOME/cloneyard/node.toml`, or `~/.config/cloneyard/node.toml`; a missing file means the defaults.
- The runtime dir `$XDG_RUNTIME_DIR/cloneyard/`, mode 0700, with `daemon.sock` and `daemon.lock`.
- The data dir `$XDG_DATA_HOME/cloneyard/`, or `~/.local/share/cloneyard/`, mode 0700, with `cloneyard.db`, `cloneyard.lock` and `backups/`.
- The `system.*` methods and `rpc.discover`, registered through their types in `cloneyard-api`; `system.ping` answers with the node id, and `rpc.discover` with the OpenRPC document.
- Logs through `tracing` to stderr, and a clean shutdown on SIGTERM and SIGINT.

```toml
[log]
level = "info" # a tracing filter; RUST_LOG overrides it
```

## Out of scope

- The protocol, which lives in `cloneyard-rpc`.
- The methods' names and types, and the OpenRPC document, which live in `cloneyard-api`.
- The database and its schema, which live in `cloneyard-store`.
- Arguments and output for people, which live in `cloneyard-cli`.
