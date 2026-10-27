-- One directed friendship edge per owner and other (RU5 chat; Node's `friendships`, _id `owner:other`). `id` keeps
-- the order Node listed them in (insertion). `other` is a global account id that may live on another node (DD17), so
-- it has no foreign key and carries its userId, which is what the client sees.
CREATE TABLE friend_edges (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    owner TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    other TEXT NOT NULL,
    other_user_id TEXT NOT NULL,
    status TEXT NOT NULL,
    direction TEXT,
    created TEXT NOT NULL,
    favorite BOOLEAN NOT NULL DEFAULT FALSE,
    UNIQUE (owner, other)
);
CREATE INDEX friend_edges_other ON friend_edges (other);
