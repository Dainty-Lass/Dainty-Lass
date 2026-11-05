CREATE TABLE bounty_states (
    account_id TEXT PRIMARY KEY,
    bounties TEXT NOT NULL,
    draft_data TEXT NOT NULL,
    daily_reward_credited_on TEXT,
    daily_premium_draft_granted_on TEXT,
    update_version BIGINT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at BIGINT
);

CREATE TABLE cooldown_states (
    account_id TEXT PRIMARY KEY,
    entries TEXT NOT NULL,
    update_version BIGINT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at BIGINT
);

CREATE TABLE entitlements (
    account_id TEXT NOT NULL,
    name TEXT NOT NULL,
    duration DOUBLE PRECISION NOT NULL,
    activated_date TEXT,
    granted_at TEXT,
    source_sku_id TEXT,
    deleted_at BIGINT,
    PRIMARY KEY (account_id, name)
);

CREATE TABLE wallets (
    account_id TEXT PRIMARY KEY,
    bootstrap_version TEXT,
    deleted_at BIGINT
);

CREATE TABLE wallet_balances (
    account_id TEXT NOT NULL,
    catalog_id TEXT NOT NULL,
    balance DOUBLE PRECISION NOT NULL,
    ordinal BIGINT NOT NULL,
    deleted_at BIGINT,
    PRIMARY KEY (account_id, catalog_id)
);

CREATE TABLE economy_rollup_counters (
    account_id TEXT NOT NULL,
    day TEXT NOT NULL,
    currency TEXT NOT NULL,
    path TEXT NOT NULL,
    value DOUBLE PRECISION NOT NULL,
    ttl_at BIGINT NOT NULL,
    PRIMARY KEY (account_id, day, currency, path)
);
