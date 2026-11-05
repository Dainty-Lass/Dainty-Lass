-- The launcher notices (launcherNotices.ts): one document, replaced when its version is the one the saver loaded.
-- `announcements` and `tags` are JSON text, kept as validated so their key order reaches the dashboard unchanged.
CREATE TABLE launcher_notices (
    id BIGINT PRIMARY KEY,
    version BIGINT NOT NULL,
    updated_at TEXT,
    updated_by TEXT,
    announcements TEXT NOT NULL,
    tags TEXT NOT NULL
);
