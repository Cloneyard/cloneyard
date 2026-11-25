# cloneyard-store

A SQLite database on a thread of its own: the node's `cloneyard.db`, and any other file that takes the same migrations, such as one per Restricted project.

## Scope

- `Store::open`: opens or creates the file in WAL mode with foreign keys on, and migrates it to the latest schema.
- A `VACUUM INTO` copy in `backups/` beside the file, before new migrations run on a database that has a schema.
- A refusal to open a database whose schema is newer than the binary, which points at `backups/`.
- The tables `meta`, `change_log`, `project`, `repo`, `repo_path`, `account`, `account_dir`, `clone`, `session` and `hook_event`, in `migrations/`.
- The node id, created the first time it is asked for.
- Projects with their repos and the repos' paths on a node; a new project, its repo and its path go in one transaction.
- Accounts with their directories on a node; a new account and its directory go in one transaction.
- Clones with their sessions and whether they run without the sandbox, which migration `0006` sets for the clones before it: a new clone gets the node's next handle from a counter in `meta`, which never gives a number twice, and goes in one transaction with its first session; a session another clone has is an error; `lose_clones` sets the clones still running, `working`, `blocked` or `idle`, to `error`.
- Hook events of clones, only ever appended, with a `seq` cursor: one goes in one transaction with its session when that is new and the state it gives a running clone, while a finished one keeps its state. An event of an unknown clone or with another clone's session is refused, and a spool file the clone's events already have adds nothing.
- One write path for entity tables, which adds a `change_log` row for each entity a transaction changes, with the node id and an HLC timestamp that never goes back; an update logs the key and the fields it set.

## Out of scope

- Where the files live and the locks around them, which the daemon owns.
- SQL anywhere else: each feature adds the methods it needs to this crate.
- Reading and trimming `change_log`, down migrations and backup rotation.

[ADR 0003](../../docs/adr/0003-sqlite-via-rusqlite.md) explains rusqlite on a dedicated thread, [ADR 0004](../../docs/adr/0004-restricted-project-database-file.md) the file per Restricted project, [ADR 0008](../../docs/adr/0008-entity-change-log.md) the change log, and [ADR 0010](../../docs/adr/0010-clone-ids-and-handles.md) the handles of clones.
