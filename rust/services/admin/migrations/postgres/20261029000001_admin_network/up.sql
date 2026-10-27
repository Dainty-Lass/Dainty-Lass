-- Admin actions relayed across nodes (DN11): via_node on admin_actions names the originating peer when an
-- action arrived over Service::Admin, request_key deduplicates replays across restarts, and
-- admin_relay_queue holds actions waiting for a peer that was down.
ALTER TABLE admin_actions ADD COLUMN via_node TEXT;
ALTER TABLE admin_actions ADD COLUMN request_key TEXT;
CREATE INDEX admin_actions_relay_key ON admin_actions (via_node, request_key);

CREATE TABLE admin_relay_queue (
    id TEXT PRIMARY KEY,
    peer TEXT NOT NULL,
    request_key TEXT NOT NULL,
    operation TEXT NOT NULL,
    account TEXT,
    revoke_tickets INTEGER NOT NULL,
    actor_key TEXT NOT NULL,
    operator TEXT NOT NULL,
    approved_by TEXT,
    target TEXT,
    game TEXT,
    body TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    attempts INTEGER NOT NULL,
    last_error TEXT
);
CREATE INDEX admin_relay_queue_peer ON admin_relay_queue (peer, created_at);
