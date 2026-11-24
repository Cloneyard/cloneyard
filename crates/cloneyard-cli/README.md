# cloneyard-cli

The `cloneyard` binary and its short alias `cy`: a thin client of the daemon, and the command that runs it.

## Commands

- `cy account add <name>` runs `claude auth login` with `accounts/<name>/claude` in the data dir as its config directory, marks Claude Code's first-run screens as seen there and adds the account; a cancelled or failed login removes the directory. `cy accounts` lists the accounts with their providers and directories.
- `cy clone run --project <p> --account <a> "<prompt>"` starts a headless clone and prints its handle, and fails with the reason when the clone did not start. `cy clones` lists the clones with their projects, accounts, modes and states, and marks those without the sandbox `unsandboxed`, `cy clone show <handle>` shows one with its reason, sandbox, branch, checkout and sessions, and `cy clone stop <handle>` ends a working one.
- `cy daemon` runs the daemon in the foreground until SIGTERM or Ctrl+C.
- `cy ping` prints the daemon's version, the node id and the round-trip time of one call.
- `cy project add <path> [--name <name>]` adds the git repo at the path as a project and prints its name and id; `cy projects` lists the projects with the paths of their repos.
- `cy sandbox <policy> -- <program>`, hidden from `--help`, enters a clone's sandbox and runs the program in it; the daemon starts every clone through it.
- `cy rpc schema` prints the OpenRPC document that the daemon serves, in the format of `schema/cloneyard.openrpc.json`.

## Out of scope

- Logic behind the commands: each one calls the daemon's API and prints the answer, or the method's own error with exit code 1. The one exception is the Claude Code login of `cy account add`, which needs your terminal; [ADR 0009](../../docs/adr/0009-account-login-in-the-cli.md) explains it.
