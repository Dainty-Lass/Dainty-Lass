CREATE TABLE friend_edges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    owner TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    other TEXT NOT NULL,
    other_user_id TEXT NOT NULL,
    status TEXT NOT NULL,
    direction TEXT,
    created TEXT NOT NULL,
    favorite BOOLEAN NOT NULL DEFAULT FALSE,
    UNIQUE (owner, other)
);
CREATE INDEX friend_edges_other ON friend_edges (other);
