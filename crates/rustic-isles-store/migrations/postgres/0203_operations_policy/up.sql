CREATE TABLE operations_policy (
    id BIGINT PRIMARY KEY,
    guard_enforcement TEXT NOT NULL,
    diagnostics_profile TEXT NOT NULL,
    version BIGINT NOT NULL,
    updated_at TEXT,
    updated_by TEXT,
    diagnostics_expires_at TEXT
);
INSERT INTO operations_policy (id, guard_enforcement, diagnostics_profile, version)
    VALUES (1, 'ENFORCE', 'PRODUCTION', 0);
