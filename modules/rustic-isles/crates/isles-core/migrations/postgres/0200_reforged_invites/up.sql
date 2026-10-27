-- The Reforged API's invite codes (reforgedApi.ts: the `reforgedInvites` collection).
CREATE TABLE reforged_invites (
    code TEXT PRIMARY KEY,
    uses_remaining BIGINT NOT NULL,
    infinite_uses BOOLEAN NOT NULL,
    note TEXT,
    created_at TEXT NOT NULL
);
