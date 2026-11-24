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
- `cy project add <path>` adds a git repo as a project, and `cy projects` lists the projects with their paths ([#25](https://github.com/Cloneyard/cloneyard/issues/25)).
- The API's methods have errors of their own, with codes from 1000 and their fields as data ([#25](https://github.com/Cloneyard/cloneyard/issues/25)).
- `cy account add <name>` logs in to Claude Code in a config directory of its own and adds it as an account, and `cy accounts` lists the accounts ([#29](https://github.com/Cloneyard/cloneyard/issues/29)).
- `cy clone run --project <p> --account <a> "<prompt>"` runs Claude Code headless in a checkout of its own, and `cy clones`, `cy clone show` and `cy clone stop` list, show and end clones ([#30](https://github.com/Cloneyard/cloneyard/issues/30)).
- Clones run in a sandbox of Landlock and seccomp, which hides your secrets, the data directory and the repos from them, lets them write only in their own directory and keeps them off Unix sockets ([#31](https://github.com/Cloneyard/cloneyard/issues/31)).
- `unsandboxed = true` in `node.toml` runs clones without the sandbox, which they need on a kernel without Landlock, and `cy clones` marks them `unsandboxed` ([#31](https://github.com/Cloneyard/cloneyard/issues/31)).

[Unreleased]: https://github.com/Cloneyard/cloneyard/commits/master
