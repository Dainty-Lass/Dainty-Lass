CREATE TABLE accounts (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    is_admin BOOLEAN NOT NULL DEFAULT 0,
    score DOUBLE PRECISION,
    avatar BLOB,
    deleted_at BIGINT
);
CREATE TABLE items (
    account_id BIGINT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    slot INTEGER NOT NULL,
    name TEXT NOT NULL,
    PRIMARY KEY (account_id, slot)
);
