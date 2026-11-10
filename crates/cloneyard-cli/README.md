# cloneyard-cli

The `cloneyard` binary and its short alias `cy`: a thin client of the daemon, and the command that runs it.

## Commands

- `cy daemon` runs the daemon in the foreground until SIGTERM or Ctrl+C.
- `cy ping` prints the daemon's version, the node id and the round-trip time of one call.
- `cy rpc schema` prints the OpenRPC document that the daemon serves, in the format of `schema/cloneyard.openrpc.json`.

## Out of scope

- Logic behind the commands: each one calls the daemon's API and prints the answer.
