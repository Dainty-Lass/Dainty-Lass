-- Federation (DN10b): a message id the nodes agree on, so a message that reaches a node twice is stored once
ALTER TABLE chat_room_messages ADD COLUMN mid TEXT;
CREATE UNIQUE INDEX chat_room_messages_mid ON chat_room_messages (room, mid);
ALTER TABLE chat_direct_messages ADD COLUMN mid TEXT;
CREATE UNIQUE INDEX chat_direct_messages_mid ON chat_direct_messages (mid);
-- Calls to another node that must happen eventually (a direct message's copy at a home, a mute at its home)
CREATE TABLE chat_outbox (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    peer TEXT NOT NULL,
    call TEXT NOT NULL,
    tries BIGINT NOT NULL,
    next_at BIGINT NOT NULL,
    created_at BIGINT NOT NULL
);
CREATE INDEX chat_outbox_due ON chat_outbox (next_at, id);
