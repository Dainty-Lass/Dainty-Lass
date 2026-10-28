-- Ticket revocation floors (DN5): per account, the unix millisecond timestamp at which its sessions
-- were ended (ban or timeout), so tickets issued at or before that floor stay refused across restarts.
CREATE TABLE network_ticket_revocations (
    account TEXT PRIMARY KEY NOT NULL,
    revoked_at BIGINT NOT NULL
);
