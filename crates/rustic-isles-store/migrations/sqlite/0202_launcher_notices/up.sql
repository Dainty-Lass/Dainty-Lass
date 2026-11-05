CREATE TABLE launcher_notices (
    id BIGINT PRIMARY KEY,
    version BIGINT NOT NULL,
    updated_at TEXT,
    updated_by TEXT,
    announcements TEXT NOT NULL,
    tags TEXT NOT NULL
);
