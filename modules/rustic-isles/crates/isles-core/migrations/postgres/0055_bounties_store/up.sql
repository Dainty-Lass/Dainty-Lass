-- RU5 bounties-store: bounty and cooldown state, entitlements, wallets, economy rollups. Keyed by the global account id
-- (DD18); `store`'s change log (DD23) triggers on every table. The player's own tables are soft-deleted (deleted_at).
CREATE TABLE bounty_states (
    account_id TEXT PRIMARY KEY,
    -- StoredBounty[] as Node stores it: {group, bounty, claimedAt?, rewardCredit?}
    bounties TEXT NOT NULL,
    -- {base?, daily?, weekly?}
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
    -- [{id, startedDate, updatedAt}] in write order
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

-- ordinal keeps Node's insertion order of the balances subdocument, which /balance's key order follows
CREATE TABLE wallet_balances (
    account_id TEXT NOT NULL,
    catalog_id TEXT NOT NULL,
    balance DOUBLE PRECISION NOT NULL,
    ordinal BIGINT NOT NULL,
    deleted_at BIGINT,
    PRIMARY KEY (account_id, catalog_id)
);

-- One counter per dotted path of the day's document ("bySource.STORE.debited"); monitoring only
CREATE TABLE economy_rollup_counters (
    account_id TEXT NOT NULL,
    day TEXT NOT NULL,
    currency TEXT NOT NULL,
    path TEXT NOT NULL,
    value DOUBLE PRECISION NOT NULL,
    ttl_at BIGINT NOT NULL,
    PRIMARY KEY (account_id, day, currency, path)
);
