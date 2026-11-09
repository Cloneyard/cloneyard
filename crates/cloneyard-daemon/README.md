# cloneyard-daemon

The `cloneyard daemon` process on a node: it reads the node's settings, holds the single-instance lock and serves the API on a Unix socket.

## Scope

- The settings in `$XDG_CONFIG_HOME/cloneyard/node.toml`, or `~/.config/cloneyard/node.toml`; a missing file means the defaults.
- The runtime dir `$XDG_RUNTIME_DIR/cloneyard/`, mode 0700, with `daemon.sock` and `daemon.lock`.
- The `system.*` methods.
- Logs through `tracing` to stderr, and a clean shutdown on SIGTERM and SIGINT.

```toml
[log]
level = "info" # a tracing filter; RUST_LOG overrides it
```

## Out of scope

- The protocol, which lives in `cloneyard-rpc`.
- Arguments and output for people, which live in `cloneyard-cli`.
