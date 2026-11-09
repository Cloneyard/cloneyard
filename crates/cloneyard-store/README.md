# cloneyard-store

A SQLite database on a thread of its own: the node's `cloneyard.db`, and any other file that takes the same migrations, such as one per Restricted project.

## Scope

- `Store::open`: opens or creates the file in WAL mode with foreign keys on, and migrates it to the latest schema.
- A `VACUUM INTO` copy in `backups/` beside the file, before new migrations run on a database that has a schema.
- A refusal to open a database whose schema is newer than the binary, which points at `backups/`.
- The tables `meta`, `change_log`, `project` and `repo`, in `migrations/`.
- The node id, created the first time it is asked for.

## Out of scope

- Where the files live and the locks around them, which the daemon owns.
- SQL anywhere else: each feature adds the methods it needs to this crate.
- Writes to `change_log`, down migrations and backup rotation.

[ADR 0003](../../docs/adr/0003-sqlite-via-rusqlite.md) explains rusqlite on a dedicated thread, and [ADR 0004](../../docs/adr/0004-restricted-project-database-file.md) the file per Restricted project.
