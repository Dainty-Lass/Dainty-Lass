DROP TABLE admin_relay_queue;
DROP INDEX admin_actions_relay_key;
ALTER TABLE admin_actions DROP COLUMN request_key;
ALTER TABLE admin_actions DROP COLUMN via_node;
