CREATE TABLE inventories (
    character_id TEXT PRIMARY KEY,
    account_id TEXT,
    instanced_items TEXT NOT NULL,
    stacked_items TEXT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 0,
    bootstrap_version TEXT,
    deleted_at BIGINT
);
CREATE INDEX inventories_account ON inventories (account_id);

CREATE TABLE inventory_transactions (
    account_id TEXT NOT NULL,
    character_id TEXT NOT NULL,
    transaction_id TEXT NOT NULL,
    request_hash TEXT,
    status TEXT NOT NULL,
    result TEXT,
    created_at TEXT NOT NULL,
    completed_at TEXT,
    economy_source TEXT,
    economy_session_id TEXT,
    extra TEXT,
    PRIMARY KEY (account_id, character_id, transaction_id)
);
CREATE INDEX inventory_transactions_account ON inventory_transactions (account_id, created_at);
