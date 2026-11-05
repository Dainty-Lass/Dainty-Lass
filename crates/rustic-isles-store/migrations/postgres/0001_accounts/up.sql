CREATE TABLE accounts (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    notes BIGINT NOT NULL DEFAULT 0,
    deleted_at BIGINT
);
