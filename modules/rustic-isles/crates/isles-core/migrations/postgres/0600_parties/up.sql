-- Parties (RU5 matchmaking-parties, DD18): the party's global id is the key, the client sees client_id (Node's
-- partyId, "<hex>_<base64 buildId>"). Members are global account ids; one party per account.
CREATE TABLE parties (
    id TEXT PRIMARY KEY,
    client_id TEXT NOT NULL UNIQUE,
    revision BIGINT NOT NULL,
    leader TEXT NOT NULL,
    build_id TEXT NOT NULL,
    activity TEXT NOT NULL,
    active_hunt_session_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- position keeps the join order: Node's members array, whose first entry takes over a leaderless party
CREATE TABLE party_members (
    account TEXT PRIMARY KEY,
    party TEXT NOT NULL,
    position BIGINT NOT NULL,
    joined_at TEXT NOT NULL
);
CREATE INDEX party_members_party ON party_members (party, position);

-- seq orders what Node ordered by rowid
CREATE TABLE party_invites (
    recipient TEXT NOT NULL,
    sender TEXT NOT NULL,
    party TEXT NOT NULL,
    party_revision BIGINT NOT NULL,
    sending_display_name TEXT NOT NULL,
    sending_platform TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    consumed_at TEXT,
    seq BIGINT NOT NULL,
    PRIMARY KEY (recipient, sender)
);

-- The transactional outbox the chat plugin publishes from (party-events@); party is Node's partyId
CREATE TABLE party_outbox (
    id TEXT PRIMARY KEY,
    party TEXT NOT NULL,
    party_revision BIGINT NOT NULL,
    event_type TEXT NOT NULL,
    recipients TEXT NOT NULL,
    payload TEXT NOT NULL,
    state TEXT NOT NULL,
    created_at TEXT NOT NULL,
    claimed_at TEXT,
    claim_owner TEXT,
    published_at TEXT,
    seq BIGINT NOT NULL
);
CREATE INDEX party_outbox_state ON party_outbox (state, created_at, seq);
