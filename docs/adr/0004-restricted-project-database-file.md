# Restricted projects live in their own database file

A Restricted project keeps all of its data in its own SQLite file, `restricted/<project-id>.db` in the node's data directory, with the same migrations as the main `cloneyard.db`. The main database holds no trace of it, not even its id, and sync only ever opens the main database, so the privacy boundary is a separate file rather than a flag that one bug could ignore. The daemon finds Restricted projects by listing that directory.

## Considered options

- **A local-only registry in the main database**, mapping project ids to files and kept out of the change log: easier to list, but it leaves a trace in the file that sync opens, and keeping it out of sync depends on code doing the right thing.
- **A project row with a `restricted` flag in the main database**, with the data in the separate file: the project's name and existence would leave the node with sync.

## Consequences

- Foreign keys and unique constraints can't span files, so rules such as "a repo belongs to one project" are checked by the daemon across files, not by SQLite.
