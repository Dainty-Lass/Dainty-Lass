-- RU5 progression: XP tracks, mastery objectives, the grant ledger, escalation, the Slayer's Path, breadcrumbs and
-- encountered content, keyed by the global account id (DD18). Node's counters are numbers; BIGINT holds them here.
CREATE TABLE progression_tracks (
    account TEXT NOT NULL,
    progression_id TEXT NOT NULL,
    progress BIGINT NOT NULL DEFAULT 0,
    confirmed_fremium_rank BIGINT NOT NULL DEFAULT 0,
    confirmed_premium_rank BIGINT NOT NULL DEFAULT 0,
    update_version BIGINT NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    -- WeaponReforgeState as JSON, in Node's field names
    reforge TEXT,
    prestige_claimed BIGINT,
    deleted_at BIGINT,
    PRIMARY KEY (account, progression_id)
);

CREATE TABLE progression_objectives (
    account TEXT NOT NULL,
    objective_id TEXT NOT NULL,
    value BIGINT NOT NULL DEFAULT 0,
    completed_count BIGINT NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at BIGINT,
    PRIMARY KEY (account, objective_id)
);

-- The raw reports of game servers, kept for diagnosis (append-only)
CREATE TABLE progression_objective_events (
    id TEXT PRIMARY KEY,
    account TEXT NOT NULL,
    raw_body TEXT NOT NULL,
    received_at TEXT NOT NULL
);

CREATE TABLE progression_grants (
    grant_id TEXT PRIMARY KEY,
    account TEXT NOT NULL,
    progression_id TEXT NOT NULL,
    amount BIGINT NOT NULL,
    request_hash TEXT NOT NULL,
    status TEXT NOT NULL,
    -- the track right after the grant, as JSON in Node's field names
    result TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX progression_grants_account ON progression_grants (account, created_at);

CREATE TABLE escalation_progress (
    account TEXT NOT NULL,
    season_id TEXT NOT NULL,
    escalation_level BIGINT NOT NULL,
    next_level_xp BIGINT NOT NULL,
    talents_progress TEXT NOT NULL,
    unlock_progress TEXT NOT NULL,
    update_version BIGINT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at BIGINT,
    PRIMARY KEY (account, season_id)
);

CREATE TABLE player_journey (
    account TEXT PRIMARY KEY,
    nodes TEXT NOT NULL,
    update_version BIGINT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 0,
    deleted_at BIGINT
);

CREATE TABLE breadcrumbs (
    character_id TEXT PRIMARY KEY,
    account TEXT NOT NULL,
    items TEXT NOT NULL,
    update_version BIGINT NOT NULL,
    deleted_at BIGINT
);

CREATE TABLE encountered_content (
    character_id TEXT PRIMARY KEY,
    account TEXT NOT NULL,
    content TEXT NOT NULL,
    deleted_at BIGINT
);
