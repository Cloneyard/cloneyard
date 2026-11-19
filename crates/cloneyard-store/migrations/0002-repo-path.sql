CREATE TABLE repo_path (
    repo_id TEXT NOT NULL REFERENCES repo (id),
    node_id TEXT NOT NULL,
    path TEXT NOT NULL,
    PRIMARY KEY (repo_id, node_id),
    UNIQUE (node_id, path)
) STRICT;
