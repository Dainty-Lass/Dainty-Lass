-- RF6's Guard persistence tables (DD15): evidence chain state, batches, findings, traffic windows,
-- observed host writes, realm hunt XP summaries and operations alerts.

CREATE TABLE rifts_hunt_guard (
    session TEXT NOT NULL,
    generation BIGINT NOT NULL,
    revision BIGINT NOT NULL,
    completion_mode TEXT,
    last_sequence BIGINT NOT NULL,
    head_hash TEXT NOT NULL,
    encounter_state TEXT NOT NULL,
    disposition TEXT NOT NULL,
    finding_count BIGINT NOT NULL,
    batch_count BIGINT NOT NULL,
    event_count BIGINT NOT NULL,
    event_bytes BIGINT NOT NULL,
    last_monotonic_ms BIGINT,
    event_types TEXT NOT NULL,
    has_unattributed_damage INTEGER NOT NULL,
    last_received_at BIGINT,
    final_sequence BIGINT,
    final_hash TEXT,
    completed_at BIGINT,
    PRIMARY KEY (session, generation)
);

CREATE TABLE rifts_guard_batches (
    session TEXT NOT NULL,
    generation BIGINT NOT NULL,
    batch_id TEXT NOT NULL,
    previous_hash TEXT NOT NULL,
    batch_hash TEXT NOT NULL,
    first_sequence BIGINT NOT NULL,
    last_sequence BIGINT NOT NULL,
    event_count BIGINT NOT NULL,
    event_bytes BIGINT NOT NULL,
    event_types TEXT NOT NULL,
    received_at BIGINT NOT NULL,
    PRIMARY KEY (session, generation, batch_id)
);
CREATE INDEX rifts_guard_batches_session_seq ON rifts_guard_batches (session, generation, first_sequence);

CREATE TABLE rifts_guard_findings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session TEXT NOT NULL,
    generation BIGINT NOT NULL,
    sequence BIGINT,
    code TEXT NOT NULL,
    detail TEXT NOT NULL,
    at BIGINT NOT NULL
);
CREATE INDEX rifts_guard_findings_session_at ON rifts_guard_findings (session, generation, at);

CREATE TABLE rifts_traffic_windows (
    window_id TEXT PRIMARY KEY,
    scope TEXT NOT NULL,
    window_start BIGINT NOT NULL,
    requests BIGINT NOT NULL,
    bytes BIGINT NOT NULL,
    expires_at BIGINT NOT NULL
);
CREATE INDEX rifts_traffic_windows_expires ON rifts_traffic_windows (expires_at);

CREATE TABLE rifts_host_writes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    target TEXT NOT NULL,
    host TEXT NOT NULL,
    session TEXT NOT NULL,
    generation BIGINT NOT NULL,
    realm TEXT,
    session_kind TEXT,
    hub_kind TEXT,
    summary TEXT NOT NULL,
    at BIGINT NOT NULL
);
CREATE INDEX rifts_host_writes_session ON rifts_host_writes (session, generation, at);
CREATE INDEX rifts_host_writes_target ON rifts_host_writes (target, at);
CREATE INDEX rifts_host_writes_host ON rifts_host_writes (host, at);

CREATE TABLE rifts_hunt_xp_summaries (
    session TEXT NOT NULL,
    member TEXT NOT NULL,
    host TEXT NOT NULL,
    realm TEXT,
    minutes BIGINT NOT NULL,
    grants TEXT NOT NULL,
    flagged INTEGER NOT NULL,
    created_at BIGINT NOT NULL,
    ttl_at BIGINT NOT NULL,
    PRIMARY KEY (session, member)
);
CREATE INDEX rifts_hunt_xp_summaries_member ON rifts_hunt_xp_summaries (member, created_at);
CREATE INDEX rifts_hunt_xp_summaries_ttl ON rifts_hunt_xp_summaries (ttl_at);

CREATE TABLE rifts_operations_alerts (
    alert_id TEXT PRIMARY KEY,
    code TEXT NOT NULL,
    severity TEXT NOT NULL,
    message TEXT NOT NULL,
    details TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    acknowledged_at BIGINT,
    acknowledged_by TEXT
);
CREATE INDEX rifts_operations_alerts_created ON rifts_operations_alerts (created_at);
