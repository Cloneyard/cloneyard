CREATE TABLE hook_event (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    clone_id TEXT NOT NULL REFERENCES clone (id),
    session_id TEXT NOT NULL REFERENCES session (id),
    hook TEXT NOT NULL,
    tool TEXT,
    at INTEGER NOT NULL,
    file TEXT NOT NULL,
    payload TEXT NOT NULL,
    -- A spool file read again after a crash between its commit and its delete adds nothing.
    UNIQUE (clone_id, file)
) STRICT;

CREATE INDEX hook_event_clone ON hook_event (clone_id, seq);
