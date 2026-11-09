-- The registry's events, kept (module sharing A5, D2). AUTOINCREMENT: retention never hands a seq out twice,
-- so a reader's `after` cursor stays good.
CREATE TABLE events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    at_ms BIGINT NOT NULL,
    name TEXT NOT NULL,
    source TEXT,
    payload TEXT NOT NULL
);
CREATE INDEX events_name_seq ON events (name, seq);
CREATE INDEX events_at_ms ON events (at_ms);
