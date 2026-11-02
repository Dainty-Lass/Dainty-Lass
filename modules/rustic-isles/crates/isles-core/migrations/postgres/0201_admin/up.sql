-- The admin dashboard's cookie sessions and the audit of what operators changed (admin.ts; DD23's audit sits beside
-- the change log). `account` is the operator's global account id; the audit keeps the userIds the operator saw.
CREATE TABLE admin_sessions (
    id TEXT PRIMARY KEY,
    token_hash TEXT NOT NULL UNIQUE,
    account TEXT NOT NULL,
    csrf_token TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    ip TEXT NOT NULL,
    user_agent TEXT NOT NULL,
    revoked_at TEXT,
    ttl_at BIGINT NOT NULL
);

CREATE TABLE admin_audit (
    seq BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    id TEXT NOT NULL UNIQUE,
    actor_user_id TEXT NOT NULL,
    target_user_id TEXT,
    action TEXT NOT NULL,
    old_state TEXT,
    new_state TEXT,
    reason TEXT,
    ip TEXT NOT NULL,
    request_id TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX admin_audit_target ON admin_audit (target_user_id);
