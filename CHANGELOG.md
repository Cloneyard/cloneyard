# Changelog

All notable changes to Cloneyard will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- The `cloneyard` binary and its short alias `cy`, which print their version with `--version` ([#1](https://github.com/Cloneyard/cloneyard/issues/1)).
- `cy daemon` runs one daemon per node in the foreground, on a Unix socket in `$XDG_RUNTIME_DIR/cloneyard/` ([#2](https://github.com/Cloneyard/cloneyard/issues/2)).
- `cy ping` prints the daemon's version, the node id and the round trip ([#2](https://github.com/Cloneyard/cloneyard/issues/2), [#3](https://github.com/Cloneyard/cloneyard/issues/3)).
- `node.toml` in `$XDG_CONFIG_HOME/cloneyard/` sets the daemon's log level, which `RUST_LOG` overrides ([#2](https://github.com/Cloneyard/cloneyard/issues/2)).
- The node database in `$XDG_DATA_HOME/cloneyard/`, backed up before it is migrated ([#3](https://github.com/Cloneyard/cloneyard/issues/3)).
- `cy rpc schema` prints the OpenRPC document of the API ([#4](https://github.com/Cloneyard/cloneyard/issues/4)).

[Unreleased]: https://github.com/Cloneyard/cloneyard/commits/master
