-- The operations policy (operationsPolicy.ts): one row, read by the Guard rules and changed from the dashboard by
-- version. Node left the row to the deployment ("a missing singleton is a deployment fault"); here the migration
-- makes it, with the values Node falls back to: Guard enforced, production diagnostics.
CREATE TABLE operations_policy (
    id BIGINT PRIMARY KEY,
    guard_enforcement TEXT NOT NULL,
    diagnostics_profile TEXT NOT NULL,
    version BIGINT NOT NULL,
    updated_at TEXT,
    updated_by TEXT,
    diagnostics_expires_at TEXT
);
INSERT INTO operations_policy (id, guard_enforcement, diagnostics_profile, version) VALUES (1, 'ENFORCE', 'PRODUCTION', 0);
