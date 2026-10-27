-- Guarded actions (DD23 layer 4, DH6): a destructive or bulk admin action waits here for its delay to end or a
-- second operator's approval, and runs once. `params` is what the action needs to run (and the Discord operator
-- it is for), as JSON; never a key. At most 16 columns: Diesel's default table width.
CREATE TABLE guarded_actions (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    -- What it acts on, so the same action asked twice waits once: a player and game, a key id, a revert plan id
    subject TEXT NOT NULL,
    summary TEXT NOT NULL,
    params TEXT NOT NULL,
    -- delay or approval
    mode TEXT NOT NULL,
    -- pending, approved, running, done, failed, canceled or expired
    state TEXT NOT NULL,
    -- The role an approver needs: at least the initiator's, and the action's own
    needs_role TEXT NOT NULL,
    initiated_by TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    -- A delay's end; null for an approval
    release_at BIGINT,
    expires_at BIGINT NOT NULL,
    approved_by TEXT,
    canceled_by TEXT,
    -- When it was done, failed, canceled or expired
    ended_at BIGINT,
    result TEXT
);
CREATE INDEX guarded_actions_state ON guarded_actions (state, created_at);
-- Who approved an action that ran, beside the key that started it
ALTER TABLE admin_actions ADD COLUMN approved_by TEXT;
