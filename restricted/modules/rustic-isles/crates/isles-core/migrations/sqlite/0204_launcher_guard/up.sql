-- Launcher Guard (RF1 decision 1: isles keeps the sessions and the findings; launcherGuard.ts). `account` is the global
-- account id; the JSON columns hold what Node kept as document fields. `ttl_at` is unix milliseconds, purged past it.
CREATE TABLE launcher_guard_sessions (
    guard_session_id TEXT PRIMARY KEY,
    account TEXT NOT NULL,
    role TEXT NOT NULL,
    public_key_base64 TEXT NOT NULL,
    manifest_id TEXT NOT NULL,
    manifest_sequence BIGINT NOT NULL,
    channel TEXT NOT NULL,
    challenge TEXT NOT NULL,
    last_sequence BIGINT NOT NULL,
    status TEXT NOT NULL,
    risk_score DOUBLE PRECISION NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    last_heartbeat_at TEXT,
    -- JSON: launcherVersion, lastSignals, reasons, history (kept together: store's tables stop at 16 columns)
    detail TEXT NOT NULL,
    ttl_at BIGINT NOT NULL
);
CREATE INDEX launcher_guard_sessions_account ON launcher_guard_sessions (account, role, status);
CREATE INDEX launcher_guard_sessions_created ON launcher_guard_sessions (created_at);

CREATE TABLE guard_findings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_name TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    signer TEXT NOT NULL,
    size BIGINT NOT NULL,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    session_count BIGINT NOT NULL,
    account_count BIGINT NOT NULL,
    account_ids TEXT NOT NULL,
    ttl_at BIGINT NOT NULL,
    UNIQUE (sha256, file_name)
);
