CREATE TABLE playtime (
    account TEXT PRIMARY KEY,
    online_ms BIGINT NOT NULL,
    last_seen_ms BIGINT
);
