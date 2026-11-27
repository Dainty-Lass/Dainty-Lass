-- One old save copied into one account, once (spec-legacy-ramsgate.md): the key keeps a second account from
-- taking the same save, the unique account keeps one account from taking two.
CREATE TABLE legacy_imports (
    source TEXT NOT NULL,
    source_user TEXT NOT NULL,
    account TEXT NOT NULL,
    matched_by TEXT NOT NULL,
    imported_at BIGINT NOT NULL,
    report TEXT NOT NULL,
    PRIMARY KEY (source, source_user)
);
CREATE UNIQUE INDEX legacy_imports_account ON legacy_imports (account, source);
-- A one-time code an admin gives a player with no Discord link; only its SHA-256 is kept.
CREATE TABLE legacy_claims (
    code_hash TEXT PRIMARY KEY,
    source TEXT NOT NULL,
    source_user TEXT NOT NULL,
    created_by TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    redeemed_by TEXT,
    redeemed_at BIGINT
);
CREATE UNIQUE INDEX legacy_claims_redeemed ON legacy_claims (redeemed_by, source);
