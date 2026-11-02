-- Timeouts: a launcher account that may sign in but not play until timeout_until (RFC 3339, as Date#toISOString
-- writes it, so the text orders as the time does). A past timeout_until is no timeout; the columns are cleared when the
-- next one is set or lifted.
ALTER TABLE launcher_accounts ADD COLUMN timeout_until TEXT;
ALTER TABLE launcher_accounts ADD COLUMN timeout_since TEXT;
ALTER TABLE launcher_accounts ADD COLUMN timeout_reason TEXT;
