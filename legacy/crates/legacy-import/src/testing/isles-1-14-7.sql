-- The 1.14.7 account tables, as rustic-isles' isles-core makes them on SQLite (migrations 0001_accounts, 0020_auth and
-- 0200_reforged_invites at rustic-isles af9c125; AGPL-3.0-only with Mystic Paradox's additional terms, NOTICE.md).
-- The tests' stand-in for a 1.14.7 server's database: only the tables an import reads or writes.

-- The game account (DD18): its global id is the key, the client sees user_id (Node's userId).
CREATE TABLE accounts (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    notes BIGINT NOT NULL DEFAULT 0,
    deleted_at BIGINT
);

-- RU5 auth: launcher accounts and their sessions, codes and keys. Account ids are the global ids of accounts (DD18);
-- the client-visible userId stays on accounts.user_id.
CREATE TABLE launcher_accounts (
    account_id TEXT PRIMARY KEY REFERENCES accounts (id),
    email TEXT,
    display_name_normalized TEXT NOT NULL,
    display_name TEXT NOT NULL,
    password_hash TEXT,
    status TEXT NOT NULL,
    approval_status TEXT,
    approval_updated_at TEXT,
    approval_updated_by TEXT,
    approval_reason TEXT,
    roles TEXT NOT NULL,
    roles_updated_at TEXT,
    roles_updated_by TEXT,
    created_at TEXT NOT NULL,
    last_login_at TEXT,
    username_set BIGINT,
    consent_terms_version TEXT,
    consent_privacy_version TEXT,
    consent_accepted_at TEXT,
    deletion_requested_at TEXT,
    deletion_scheduled_for TEXT,
    email_verified_at TEXT,
    email_verification_pending_since TEXT,
    deleted_at BIGINT
);
-- Unique among live rows only: a registration the sweep removes frees its address and name
CREATE UNIQUE INDEX launcher_accounts_email ON launcher_accounts (email) WHERE email IS NOT NULL AND deleted_at IS NULL;
CREATE UNIQUE INDEX launcher_accounts_display_name ON launcher_accounts (display_name_normalized) WHERE deleted_at IS NULL;

CREATE TABLE auth_identities (
    provider TEXT NOT NULL,
    provider_subject TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts (id),
    provider_username TEXT NOT NULL,
    provider_avatar_url TEXT,
    linked_at TEXT NOT NULL,
    PRIMARY KEY (provider, provider_subject)
);
CREATE INDEX auth_identities_account ON auth_identities (provider, account_id);

CREATE TABLE refresh_sessions (
    id TEXT PRIMARY KEY,
    token_hash TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts (id),
    family_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    device_name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    revoked_at TEXT
);
CREATE INDEX refresh_sessions_token_hash ON refresh_sessions (token_hash);
CREATE INDEX refresh_sessions_family ON refresh_sessions (family_id, account_id);
CREATE INDEX refresh_sessions_account ON refresh_sessions (account_id);

CREATE TABLE game_exchange_codes (
    code_hash TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts (id),
    launcher_session_id TEXT NOT NULL,
    build_changelist BIGINT NOT NULL,
    executable_sha256 TEXT NOT NULL,
    runtime_channel TEXT NOT NULL,
    runtime_sha256 TEXT NOT NULL,
    runtime_manifest_version TEXT NOT NULL,
    runtime_artifact_set_sha256 TEXT NOT NULL,
    realm_id TEXT,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    consumed_at TEXT
);
CREATE INDEX game_exchange_codes_account ON game_exchange_codes (account_id);

CREATE TABLE discord_oauth_transactions (
    state TEXT PRIMARY KEY,
    code_verifier TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    consumed_at TEXT,
    completion_code_hash TEXT,
    completion_expires_at TEXT,
    completion_consumed_at TEXT,
    account_id TEXT REFERENCES accounts (id),
    link_account_id TEXT REFERENCES accounts (id)
);
CREATE INDEX discord_oauth_completion ON discord_oauth_transactions (completion_code_hash);

CREATE TABLE email_codes (
    account_id TEXT NOT NULL REFERENCES accounts (id),
    purpose TEXT NOT NULL,
    code_hash TEXT NOT NULL,
    attempts BIGINT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    PRIMARY KEY (account_id, purpose)
);

CREATE TABLE game_server_api_keys (
    key_hash TEXT PRIMARY KEY
);

CREATE TABLE user_api_keys (
    account_id TEXT PRIMARY KEY REFERENCES accounts (id),
    key_hash TEXT NOT NULL
);
CREATE INDEX user_api_keys_hash ON user_api_keys (key_hash);

-- The Reforged API's invite codes (reforgedApi.ts: the `reforgedInvites` collection).
CREATE TABLE reforged_invites (
    code TEXT PRIMARY KEY,
    uses_remaining BIGINT NOT NULL,
    infinite_uses BOOLEAN NOT NULL,
    note TEXT,
    created_at TEXT NOT NULL
);
