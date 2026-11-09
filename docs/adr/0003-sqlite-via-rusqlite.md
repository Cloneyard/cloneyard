# SQLite through rusqlite on a dedicated thread

Each node keeps its data in SQLite in WAL mode with foreign keys on, through rusqlite with the bundled SQLite, so the build doesn't depend on the system library. One dedicated thread owns the connection, and schema migrations run with rusqlite_migration. Before applying new migrations to a database that already has a schema, the daemon writes a copy with `VACUUM INTO` into `backups/`. There are no down migrations: a binary older than its database refuses to start, and the way back is to restore that copy.

## Considered options

- **sqlx**: an async API over the same blocking SQLite, which it runs on a worker thread of its own anyway, with a larger dependency tree.
- **`spawn_blocking` per query**: each call can land on a different thread, so the connection has to sit behind a mutex or a pool.
- **The online backup API before migrations**: it suits copying a database that is in use, such as a scheduled backup, not a copy taken before anything has opened it.
- **Down migrations**: an older binary doesn't have the down steps of newer migrations, so they would only work as a command run with the newer binary before the downgrade, and each one drops the data written since the upgrade.
