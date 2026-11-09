CREATE TABLE meta (
    key TEXT NOT NULL PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;

CREATE TABLE change_log (
    id TEXT NOT NULL PRIMARY KEY,
    node_id TEXT NOT NULL,
    -- Milliseconds in the high 48 bits, a counter in the low 16.
    hlc INTEGER NOT NULL,
    entity TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    op TEXT NOT NULL CHECK (op IN ('upsert', 'delete', 'append')),
    fields TEXT NOT NULL
) STRICT;

CREATE TABLE project (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
) STRICT;

CREATE TABLE repo (
    id TEXT NOT NULL PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES project (id),
    name TEXT NOT NULL,
    remote_url TEXT UNIQUE,
    UNIQUE (project_id, name)
) STRICT;
