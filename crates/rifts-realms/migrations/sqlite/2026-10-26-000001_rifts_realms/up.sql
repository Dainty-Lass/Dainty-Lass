-- Realms and player-hosted hunts on store (DD15). Times are Unix milliseconds, enums their upper-case wire names.
-- A realm's and a hunt's lists are child tables ordered by seq; hunt fields nothing queries stay in detail (JSON).

CREATE TABLE rifts_realms (
    realm_id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    host_account_id TEXT NOT NULL UNIQUE,
    host_public_key TEXT NOT NULL,
    join_policy TEXT NOT NULL,
    listed BOOLEAN NOT NULL,
    created_at BIGINT NOT NULL,
    last_online_at BIGINT,
    online_until BIGINT,
    version TEXT,
    offline_notified_for BIGINT,
    hosting_max_servers BIGINT,
    hosting_max_players BIGINT,
    hosting_afk_seconds BIGINT,
    hosting_hub_session_id TEXT,
    hosting_hub_state TEXT,
    hosting_training_state TEXT,
    hosting_active_servers BIGINT,
    hosting_updated_at BIGINT
);
CREATE INDEX rifts_realms_listed ON rifts_realms (listed, online_until);

CREATE TABLE rifts_realm_members (
    realm_id TEXT NOT NULL REFERENCES rifts_realms (realm_id),
    account_id TEXT NOT NULL,
    role TEXT NOT NULL,
    joined_at BIGINT NOT NULL,
    seq BIGINT NOT NULL,
    PRIMARY KEY (realm_id, account_id)
);
CREATE INDEX rifts_realm_members_account ON rifts_realm_members (account_id);

CREATE TABLE rifts_realm_bans (
    realm_id TEXT NOT NULL REFERENCES rifts_realms (realm_id),
    account_id TEXT NOT NULL,
    reason TEXT,
    at BIGINT NOT NULL,
    seq BIGINT NOT NULL,
    PRIMARY KEY (realm_id, account_id)
);

CREATE TABLE rifts_realm_join_requests (
    realm_id TEXT NOT NULL REFERENCES rifts_realms (realm_id),
    account_id TEXT NOT NULL,
    at BIGINT NOT NULL,
    seq BIGINT NOT NULL,
    PRIMARY KEY (realm_id, account_id)
);

CREATE TABLE rifts_realm_player_invites (
    realm_id TEXT NOT NULL REFERENCES rifts_realms (realm_id),
    account_id TEXT NOT NULL,
    at BIGINT NOT NULL,
    seq BIGINT NOT NULL,
    PRIMARY KEY (realm_id, account_id)
);
CREATE INDEX rifts_realm_player_invites_account ON rifts_realm_player_invites (account_id);

CREATE TABLE rifts_realm_hub_rooms (
    room_id TEXT PRIMARY KEY,
    realm_id TEXT NOT NULL REFERENCES rifts_realms (realm_id),
    seq BIGINT NOT NULL
);
CREATE INDEX rifts_realm_hub_rooms_realm ON rifts_realm_hub_rooms (realm_id);

CREATE TABLE rifts_realm_invites (
    code TEXT PRIMARY KEY,
    realm_id TEXT NOT NULL,
    created_by TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    max_uses BIGINT NOT NULL,
    uses BIGINT NOT NULL,
    revoked_at BIGINT,
    purge_at BIGINT NOT NULL
);
CREATE INDEX rifts_realm_invites_realm ON rifts_realm_invites (realm_id, created_at);
CREATE INDEX rifts_realm_invites_purge ON rifts_realm_invites (purge_at);

CREATE TABLE rifts_realm_presence (
    account_id TEXT PRIMARY KEY,
    realm_id TEXT NOT NULL,
    bound_at BIGINT NOT NULL,
    last_seen_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL
);
CREATE INDEX rifts_realm_presence_realm ON rifts_realm_presence (realm_id, expires_at);
CREATE INDEX rifts_realm_presence_expires ON rifts_realm_presence (expires_at);

CREATE TABLE rifts_hunt_sessions (
    session_id TEXT PRIMARY KEY,
    global_id TEXT UNIQUE,
    generation BIGINT NOT NULL,
    state TEXT NOT NULL,
    state_version BIGINT NOT NULL,
    party_id TEXT NOT NULL,
    active_party_key TEXT UNIQUE,
    host_account_id TEXT NOT NULL,
    session_kind TEXT,
    hub_kind TEXT,
    realm_id TEXT,
    lease_expires_at BIGINT,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    detail TEXT NOT NULL
);
CREATE INDEX rifts_hunt_sessions_state ON rifts_hunt_sessions (state, created_at);
CREATE INDEX rifts_hunt_sessions_realm ON rifts_hunt_sessions (realm_id, state);
CREATE INDEX rifts_hunt_sessions_host ON rifts_hunt_sessions (host_account_id, state);

CREATE TABLE rifts_hunt_roster (
    session_id TEXT NOT NULL REFERENCES rifts_hunt_sessions (session_id),
    account_id TEXT NOT NULL,
    seq BIGINT NOT NULL,
    steam_id TEXT NOT NULL,
    candidate_id TEXT NOT NULL,
    reconnect_epoch BIGINT NOT NULL,
    admission_state TEXT NOT NULL,
    local_bridge_instance_id TEXT,
    local_bridge_port BIGINT,
    local_bridge_ready_at BIGINT,
    admitted_at BIGINT,
    disconnected_at BIGINT,
    rejoin_deadline BIGINT,
    returned_to_hub_at BIGINT,
    PRIMARY KEY (session_id, account_id)
);
CREATE INDEX rifts_hunt_roster_account ON rifts_hunt_roster (account_id);

CREATE TABLE rifts_hunt_transitions (
    transition_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    generation BIGINT NOT NULL,
    from_state TEXT NOT NULL,
    to_state TEXT NOT NULL,
    state_version BIGINT NOT NULL,
    reason TEXT NOT NULL,
    actor_type TEXT NOT NULL,
    actor_account_id TEXT,
    at BIGINT NOT NULL
);
CREATE INDEX rifts_hunt_transitions_session ON rifts_hunt_transitions (session_id, at, state_version);

CREATE TABLE rifts_host_capabilities (
    capability_id TEXT PRIMARY KEY,
    secret_digest TEXT NOT NULL UNIQUE,
    session_id TEXT NOT NULL,
    generation BIGINT NOT NULL,
    host_account_id TEXT NOT NULL,
    host_steam_id TEXT NOT NULL,
    bridge_instance_id TEXT NOT NULL,
    scopes TEXT NOT NULL,
    issued_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    revoked_at BIGINT,
    last_sequence BIGINT NOT NULL
);
CREATE INDEX rifts_host_capabilities_session ON rifts_host_capabilities (session_id, generation);

CREATE TABLE rifts_host_presences (
    account_id TEXT PRIMARY KEY,
    revision BIGINT NOT NULL,
    consent_to_host BOOLEAN NOT NULL,
    observed_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    detail TEXT NOT NULL
);
CREATE INDEX rifts_host_presences_expires ON rifts_host_presences (expires_at);

CREATE TABLE rifts_host_offers (
    offer_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    generation BIGINT NOT NULL,
    account_id TEXT NOT NULL,
    steam_id TEXT NOT NULL,
    rank BIGINT NOT NULL,
    score DOUBLE PRECISION NOT NULL,
    score_breakdown TEXT NOT NULL,
    status TEXT NOT NULL,
    offered_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    responded_at BIGINT,
    response_reason TEXT
);
CREATE INDEX rifts_host_offers_session ON rifts_host_offers (session_id, generation, rank);
CREATE UNIQUE INDEX rifts_host_offers_accepted ON rifts_host_offers (session_id, generation) WHERE status = 'ACCEPTED';

CREATE TABLE rifts_host_reliability (
    account_id TEXT PRIMARY KEY,
    offers_accepted BIGINT NOT NULL,
    offers_declined BIGINT NOT NULL,
    offers_expired BIGINT NOT NULL,
    starts_succeeded BIGINT NOT NULL,
    starts_failed BIGINT NOT NULL,
    clean_completions BIGINT NOT NULL,
    unexpected_failures BIGINT NOT NULL,
    cooldown_until BIGINT,
    updated_at BIGINT NOT NULL
);

CREATE TABLE rifts_participant_observations (
    observation_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    generation BIGINT NOT NULL,
    account_id TEXT NOT NULL,
    steam_id TEXT NOT NULL,
    host_reachable BOOLEAN NOT NULL,
    local_bridge_alive BOOLEAN NOT NULL,
    metrics_available BOOLEAN NOT NULL,
    last_packet_age_ms BIGINT,
    route_kind TEXT NOT NULL,
    ping_ms BIGINT,
    jitter_ms BIGINT,
    packet_loss_percent DOUBLE PRECISION,
    observed_at BIGINT NOT NULL
);
CREATE INDEX rifts_participant_observations_session ON rifts_participant_observations (session_id, generation, observed_at);
CREATE INDEX rifts_participant_observations_account ON rifts_participant_observations (account_id, observed_at);

CREATE TABLE rifts_control_sequences (
    account_id TEXT PRIMARY KEY,
    next_sequence BIGINT NOT NULL
);

CREATE TABLE rifts_control_messages (
    message_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    sequence BIGINT NOT NULL,
    message_type TEXT NOT NULL,
    session_id TEXT,
    generation BIGINT,
    payload TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    acknowledged_at BIGINT,
    UNIQUE (account_id, sequence)
);
CREATE INDEX rifts_control_messages_expires ON rifts_control_messages (expires_at);

CREATE TABLE rifts_hub_presence (
    session_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    generation BIGINT NOT NULL,
    realm_id TEXT NOT NULL,
    peer_id TEXT NOT NULL,
    state TEXT NOT NULL,
    candidate_id TEXT NOT NULL,
    entry_epoch BIGINT NOT NULL,
    local_bridge_port BIGINT,
    local_bridge_ready_at BIGINT,
    admission_id TEXT,
    route TEXT,
    requested_at BIGINT NOT NULL,
    ticketed_at BIGINT,
    admitted_at BIGINT,
    lost_at BIGINT,
    left_at BIGINT,
    left_reason TEXT,
    expires_at BIGINT NOT NULL,
    purge_at BIGINT,
    PRIMARY KEY (session_id, account_id)
);
CREATE INDEX rifts_hub_presence_generation ON rifts_hub_presence (session_id, generation, state);
CREATE INDEX rifts_hub_presence_account ON rifts_hub_presence (account_id, state);
CREATE INDEX rifts_hub_presence_realm ON rifts_hub_presence (realm_id, state);
CREATE INDEX rifts_hub_presence_purge ON rifts_hub_presence (purge_at);
