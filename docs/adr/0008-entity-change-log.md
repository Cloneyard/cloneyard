# Entity tables log their changes, append-only tables are their own log

Every write to an entity table, such as `project`, `repo` or `repo_path`, goes through one write path in `cloneyard-store`, which adds a `change_log` row in the same transaction for each entity it changes: a ULID, the node id, an HLC timestamp, the entity and its id, the operation (`upsert` or `delete`) and the fields it set, as JSON. The HLC holds milliseconds in its high 48 bits and a counter in its low 16. Each row takes the larger of the wall clock and the last HLC plus one, and the last HLC is kept in `meta` in the same transaction, so it keeps growing when the clock is set back, across restarts too. Tables that are only appended to, such as tool calls and API requests, stay out of the change log: their rows never change, so each such table is its own log, which sync reads through a `seq INTEGER PRIMARY KEY AUTOINCREMENT` cursor, with the ULID kept as the row's id. That leaves nothing for `op = 'append'`, so migration 0003 rebuilds `change_log` without it, which loses no rows, as nothing had written to the table.

## Considered options

- **Append-only rows in the change log, with `op = 'append'`**: each row would be written twice, once in its table and once in the log, and removing old telemetry would have to reach into the log too.
- **Each write method adding its own `change_log` rows**: a method that forgets one leaves a change that sync never sends, and nothing fails.
- **The last HLC read as `MAX(hlc)` from `change_log`**: one write less per transaction, but trimming the log could set the clock back.
- **The last HLC only in memory**: after a restart with the clock set back, new rows would sort before the ones already logged.
- **A ULID as the cursor of an append-only table**: its time part follows the wall clock, which can go back.
- **A plain rowid as that cursor**: once retention empties a table, SQLite starts its rowids at 1 again, below a cursor sync already holds.
- **Keeping `'append'` as a reserved value**: an allowed value that nothing writes.

## Consequences

- A new entity table gets its variant in the write path, which names its key columns; a key of several columns, as in `repo_path`, becomes an id with their values joined by `:`.
- A store method that writes an entity table with its own SQL bypasses the change log, so review treats it as a bug.
- The append-only tables that come with telemetry each have `seq` as their primary key and the ULID as `id`.
