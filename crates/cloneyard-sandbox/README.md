# cloneyard-sandbox

The sandbox a clone runs in: Landlock limits what it reads and writes, and seccomp filters keep it off every Unix socket but datagram ones.

## Scope

- `Policy`: the paths a clone can't read, the ones it reads inside them anyway, and the ones it writes, resolved through their symlinks; a hidden path inside a visible or writable one is an error, since Landlock can't take back part of a tree it granted.
- `command`: `<exe> sandbox <policy> -- <program>` for a command, with its environment and directory and its program found on its `PATH`, so a missing one fails before the start.
- `enter`: what `cy sandbox` runs; it refuses an inherited descriptor other than 0–2, restricts the process to the policy and replaces it with the program.
- Landlock: read and execute on everything but the hidden paths, by granting what lies around them from `/` down, listing everywhere, and every right on the writable paths. ABI v1 is required; later rights and the scopes of signals and abstract sockets apply where the kernel has them.
- seccomp: `socket(AF_UNIX, …)` of a type other than `SOCK_DGRAM` fails with `EACCES` and io_uring with `EPERM`, also through their x32 numbers, and a syscall of another ABI kills the process.
- `supported`: whether the kernel has Landlock.

## Out of scope

- Which paths a clone gets, which the daemon decides.
- Network rules: Landlock limits ports, not hosts.
- Memory and CPU limits.

[ADR 0012](../../docs/adr/0012-clone-sandbox-of-landlock-and-seccomp.md) explains the sandbox.
