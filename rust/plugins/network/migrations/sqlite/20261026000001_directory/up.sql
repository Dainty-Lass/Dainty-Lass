-- The directory (DN4): the log every node holds the same copy of, numbered by the leader, and what it
-- adds up to. Only appended to: a rename is a new entry, never an edit.
CREATE TABLE network_directory_log (
    seq BIGINT PRIMARY KEY NOT NULL,
    at BIGINT NOT NULL,
    op TEXT NOT NULL,
    account TEXT NOT NULL,
    username TEXT NOT NULL,
    home TEXT
);
CREATE TABLE network_directory_accounts (
    account TEXT PRIMARY KEY NOT NULL,
    home TEXT NOT NULL,
    username TEXT NOT NULL,
    -- The claim: lower case, unique network-wide
    username_key TEXT NOT NULL UNIQUE,
    seq BIGINT NOT NULL
);
CREATE INDEX network_directory_accounts_home ON network_directory_accounts (home);
