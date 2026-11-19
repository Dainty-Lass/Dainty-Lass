CREATE TABLE save_snapshots (
    id TEXT PRIMARY KEY,
    account TEXT NOT NULL,
    source TEXT NOT NULL,
    actor_user_id TEXT,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    restored_at BIGINT
);
CREATE INDEX save_snapshots_account ON save_snapshots (account, created_at);
