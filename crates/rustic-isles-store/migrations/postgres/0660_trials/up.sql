CREATE TABLE trials_runs (
    account TEXT NOT NULL,
    trial_id TEXT NOT NULL,
    difficulty BIGINT NOT NULL,
    mode TEXT NOT NULL,
    weapon BIGINT NOT NULL,
    completion_time DOUBLE PRECISION NOT NULL,
    objectives_completed BIGINT NOT NULL,
    player_role_id TEXT NOT NULL,
    platform TEXT NOT NULL,
    party_key TEXT NOT NULL,
    party TEXT NOT NULL,
    runs BIGINT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (account, trial_id, difficulty, mode, weapon)
);
CREATE INDEX trials_runs_board ON trials_runs (trial_id, difficulty, mode, completion_time);
