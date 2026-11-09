# Cloneyard

Cloneyard is a platform for orchestrating coding agents: every client talks to a daemon on a node through one API.

## Language

**Node**:
One Cloneyard installation with its own settings, data and id; usually one per machine.
_Avoid_: machine, host, server

**Daemon**:
The long-running Cloneyard process on a node, which clients reach only through its API.
_Avoid_: server, service, core

**Project**:
A group of repos worked on together, which share settings and one privacy boundary; a single repository is a project with one repo.
_Avoid_: workspace, system, monorepo

**Repo**:
A git repository that belongs to exactly one project; it stays the same repo when its remote URL or its path on a node changes.
_Avoid_: repository, clone, checkout

**Restricted project**:
A project whose data never leaves its node, except through a manual export.
_Avoid_: private project, company project, offline project
