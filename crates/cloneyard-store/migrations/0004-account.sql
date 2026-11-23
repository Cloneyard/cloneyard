CREATE TABLE account (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    provider TEXT NOT NULL
) STRICT;

CREATE TABLE account_dir (
    account_id TEXT NOT NULL REFERENCES account (id),
    node_id TEXT NOT NULL,
    dir TEXT NOT NULL,
    PRIMARY KEY (account_id, node_id),
    UNIQUE (node_id, dir)
) STRICT;
