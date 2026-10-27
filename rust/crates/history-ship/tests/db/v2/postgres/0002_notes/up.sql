ALTER TABLE accounts ADD COLUMN title TEXT;
CREATE TABLE notes (
    id BIGSERIAL PRIMARY KEY,
    account_id BIGINT REFERENCES accounts (id),
    body TEXT NOT NULL
);
UPDATE accounts SET title = 'founder' WHERE id = 1;
