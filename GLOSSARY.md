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

**Change log**:
A node's ordered record of each change it makes to data that changes over time, such as projects and repos, which sync sends to the hub. Data that is only ever added to, such as telemetry, is not in it.
_Avoid_: journal, history, audit log

**Account**:
Who Cloneyard's agents run as at a provider, such as a Claude subscription; on each node its login lives in a directory of its own, such as Claude Code's config directory.
_Avoid_: profile, login

**Provider**:
The coding agent CLI that an account logs in to and Cloneyard runs, such as Claude Code.
_Avoid_: runtime, harness, backend

**Clone**:
A coding agent that Cloneyard runs on an account, in a checkout of its own, and that you talk to; it stays the same clone across the sessions it goes through.
_Avoid_: job, worker, instance

**Session**:
The provider's unit of a clone's work under one id, such as a Claude Code session, which its transcript, hooks and telemetry carry; clearing or forking in the provider starts a new session of the same clone.
_Avoid_: conversation, thread, run

**Handle**:
A clone's short name on its node, such as `CT-0042`, for typing; another node can give out the same one, so only the clone's id names it everywhere.
_Avoid_: id, number, ticket

**Checkout**:
A clone's own copy of a repo, on a branch of its own, which the clone may change while the repo stays as it was.
_Avoid_: worktree, working copy, clone

**Headless**:
The mode of a clone that runs from a prompt without a terminal and reports its state through its output; the other mode is interactive.
_Avoid_: background, batch, job

**Sandbox**:
The boundary a clone runs in, which limits what it reads, writes and reaches on its node; a clone outside it is unsandboxed.
_Avoid_: jail, container, isolation

**Hook event**:
What one hook of a clone's provider reports, such as a tool call, a permission prompt or the end of a turn; a clone's hook events keep the order they ran in.
_Avoid_: hook, callback, notification

**Spool**:
The directory in a clone's own one where its hook events wait for the daemon, a file each, until the daemon records them.
_Avoid_: queue, inbox, outbox
