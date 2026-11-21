CREATE TABLE gauntlet_progress (
    account TEXT NOT NULL,
    season_id TEXT NOT NULL,
    best_level BIGINT NOT NULL,
    best_remaining_sec BIGINT NOT NULL,
    runs BIGINT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (account, season_id)
);
