CREATE TABLE clone (
    id TEXT NOT NULL PRIMARY KEY,
    node_id TEXT NOT NULL,
    handle INTEGER NOT NULL,
    project_id TEXT NOT NULL REFERENCES project (id),
    repo_id TEXT NOT NULL REFERENCES repo (id),
    account_id TEXT NOT NULL REFERENCES account (id),
    mode TEXT NOT NULL,
    state TEXT NOT NULL,
    reason TEXT,
    branch TEXT NOT NULL,
    checkout TEXT NOT NULL,
    UNIQUE (node_id, handle)
) STRICT;

CREATE TABLE session (
    id TEXT NOT NULL PRIMARY KEY,
    clone_id TEXT NOT NULL REFERENCES clone (id),
    provider_id TEXT NOT NULL UNIQUE
) STRICT;
