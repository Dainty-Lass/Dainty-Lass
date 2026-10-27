-- RU5 characters-loadouts: a character belongs to an account by its global id (DD18); `ord` keeps insertion order
-- (Node's findMany returned natural order). Soft-deleted with the account's data (DD23).
CREATE TABLE characters (
    character_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    ord BIGINT NOT NULL,
    created_date TEXT NOT NULL,
    last_modified_date TEXT NOT NULL,
    name TEXT NOT NULL,
    update_version BIGINT NOT NULL,
    data TEXT NOT NULL,
    deleted_at BIGINT
);
CREATE INDEX characters_account ON characters (account_id, ord);

-- One row per character; `loadouts` and `persistent` are JSON text as Node stored them, `revision` guards every write.
CREATE TABLE loadouts (
    character_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    loadouts TEXT NOT NULL,
    persistent TEXT NOT NULL,
    active_index BIGINT NOT NULL DEFAULT 0,
    unlocked_total_slots BIGINT,
    revision BIGINT NOT NULL DEFAULT 0,
    bootstrap_version TEXT,
    deleted_at BIGINT
);
CREATE INDEX loadouts_account ON loadouts (account_id);
