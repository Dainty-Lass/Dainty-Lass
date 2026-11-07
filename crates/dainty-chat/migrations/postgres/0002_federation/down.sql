DROP TABLE chat_outbox;
DROP INDEX chat_direct_messages_mid;
ALTER TABLE chat_direct_messages DROP COLUMN mid;
DROP INDEX chat_room_messages_mid;
ALTER TABLE chat_room_messages DROP COLUMN mid;
