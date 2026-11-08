-- The term each entry was written in, for elections (docs/distributed-mode.md): 0 for entries from before
-- failover, or from a network that doesn't use it.
ALTER TABLE network_directory_log ADD COLUMN term BIGINT NOT NULL DEFAULT 0;
