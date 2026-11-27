-- An export that the provider sends again adds nothing, as each table is unique per clone by the provider's id of the request or the tool call.
CREATE TABLE api_request (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    clone_id TEXT NOT NULL REFERENCES clone (id),
    session_id TEXT NOT NULL REFERENCES session (id),
    at INTEGER NOT NULL,
    request_id TEXT,
    model TEXT,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    cache_creation_tokens INTEGER NOT NULL,
    cost_usd_micros INTEGER NOT NULL,
    duration_ms INTEGER,
    UNIQUE (clone_id, request_id)
) STRICT;

CREATE TABLE api_error (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    clone_id TEXT NOT NULL REFERENCES clone (id),
    session_id TEXT NOT NULL REFERENCES session (id),
    at INTEGER NOT NULL,
    request_id TEXT,
    model TEXT,
    error TEXT,
    status_code INTEGER,
    attempt INTEGER,
    duration_ms INTEGER,
    UNIQUE (clone_id, request_id)
) STRICT;

CREATE TABLE tool_result (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    clone_id TEXT NOT NULL REFERENCES clone (id),
    session_id TEXT NOT NULL REFERENCES session (id),
    at INTEGER NOT NULL,
    tool_use_id TEXT,
    tool_name TEXT NOT NULL,
    success INTEGER NOT NULL,
    duration_ms INTEGER,
    error_type TEXT,
    input_size_bytes INTEGER,
    result_size_bytes INTEGER,
    UNIQUE (clone_id, tool_use_id)
) STRICT;

CREATE TABLE tool_decision (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    clone_id TEXT NOT NULL REFERENCES clone (id),
    session_id TEXT NOT NULL REFERENCES session (id),
    at INTEGER NOT NULL,
    tool_use_id TEXT,
    tool_name TEXT NOT NULL,
    decision TEXT NOT NULL,
    source TEXT,
    UNIQUE (clone_id, tool_use_id)
) STRICT;
