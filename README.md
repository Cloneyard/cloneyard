# Cloneyard

Cloneyard is an open-source platform for orchestrating coding agents. It is a single core written in Rust with one API that every client talks to, and it starts on one machine with Claude Code.

**Status:** early development, nothing is usable yet. The work is tracked in the [milestones](https://github.com/Cloneyard/cloneyard/milestones).

## Building from source

You need [rustup](https://rustup.rs). The first build installs the toolchain pinned in `rust-toolchain.toml`.

```bash
git clone https://github.com/Cloneyard/cloneyard.git
cd cloneyard
cargo build --release
```

The build puts two binaries in `target/release/`: `cloneyard` and its short alias `cy`.

```bash
./target/release/cy --version
```

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). Report security issues privately, as described in [`SECURITY.md`](SECURITY.md).

## Licence

[AGPL-3.0-only](LICENSE). The reasons are in [ADR 0001](docs/adr/0001-agpl-only-with-cla.md).
