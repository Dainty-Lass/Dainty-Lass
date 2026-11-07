ALTER TABLE accounts ADD COLUMN title TEXT;
CREATE TABLE notes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id BIGINT REFERENCES accounts (id),
    body TEXT NOT NULL
);
UPDATE accounts SET title = 'founder' WHERE id = 1;
