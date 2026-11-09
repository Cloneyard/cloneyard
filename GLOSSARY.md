# Cloneyard

Cloneyard is a platform for orchestrating coding agents: every client talks to a daemon on a node through one API.

## Language

**Node**:
A machine that runs Cloneyard, with its own settings and its own data.
_Avoid_: machine, host, server

**Daemon**:
The long-running Cloneyard process on a node, which clients reach only through its API.
_Avoid_: server, service, core
