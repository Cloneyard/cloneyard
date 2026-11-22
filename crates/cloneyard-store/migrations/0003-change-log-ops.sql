-- Nothing has written to change_log yet, and SQLite can't change a CHECK in place.
DROP TABLE change_log;

CREATE TABLE change_log (
    id TEXT NOT NULL PRIMARY KEY,
    node_id TEXT NOT NULL,
    -- Milliseconds in the high 48 bits, a counter in the low 16.
    hlc INTEGER NOT NULL,
    entity TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    op TEXT NOT NULL CHECK (op IN ('upsert', 'delete')),
    fields TEXT NOT NULL
) STRICT;
