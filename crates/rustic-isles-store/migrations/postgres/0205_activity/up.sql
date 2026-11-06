CREATE TABLE player_activity_daily (
    account TEXT NOT NULL,
    day TEXT NOT NULL,
    counters TEXT NOT NULL,
    ttl_at BIGINT NOT NULL,
    PRIMARY KEY (account, day)
);
CREATE INDEX player_activity_daily_day ON player_activity_daily (day);

CREATE TABLE activity_hunts (
    id TEXT PRIMARY KEY,
    account TEXT NOT NULL,
    host BOOLEAN NOT NULL,
    kind TEXT NOT NULL,
    hunt_id TEXT NOT NULL,
    hunt_ref TEXT NOT NULL,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    end_reason TEXT,
    result TEXT,
    kills BIGINT NOT NULL,
    ttl_at BIGINT NOT NULL
);
CREATE INDEX activity_hunts_account_started ON activity_hunts (account, started_at);

CREATE TABLE slayers_path_unlocks (
    id TEXT PRIMARY KEY,
    account TEXT NOT NULL,
    node_id TEXT NOT NULL,
    unlocked_at TEXT NOT NULL,
    source TEXT NOT NULL,
    ttl_at BIGINT NOT NULL
);
CREATE INDEX slayers_path_unlocks_account_unlocked ON slayers_path_unlocks (account, unlocked_at);

CREATE TABLE activity_flags (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    severity TEXT NOT NULL,
    account TEXT NOT NULL,
    day TEXT NOT NULL,
    detail TEXT NOT NULL,
    acknowledged BOOLEAN NOT NULL,
    created_at TEXT NOT NULL,
    acknowledged_at TEXT,
    acknowledged_by TEXT,
    note TEXT,
    ttl_at BIGINT NOT NULL
);
CREATE INDEX activity_flags_created ON activity_flags (acknowledged, created_at);
CREATE INDEX activity_flags_account_created ON activity_flags (account, created_at);
