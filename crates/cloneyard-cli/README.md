# cloneyard-cli

The `cloneyard` binary and its short alias `cy`: a thin client of the daemon, and the command that runs it.

## Commands

- `cy daemon` runs the daemon in the foreground until SIGTERM or Ctrl+C.
- `cy ping` prints the daemon's version, the node id and the round-trip time of one call.

## Out of scope

- Logic behind the commands: each one calls the daemon's API and prints the answer.
