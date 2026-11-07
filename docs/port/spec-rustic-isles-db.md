<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# Clean-Room Behaviour Specification: `crates/rustic-isles-db` (RD27)

> **Scope and purpose (RD27):** this is the clean-room behaviour specification for an MIT crate, `rustic-isles-db`,
> the Rustic Isles database tool: `migrate` (a MongoDB 1.14.7 deployment into the two-engine store, with the RB11 data
> repairs on the way), `verify` (per-table counts and content hashes of the source against the target) and
> `export-mongo` (the store back into MongoDB, RB13's one-release rollback). It is unit 11 of
> [`spec-rustic-isles-core.md`](spec-rustic-isles-core.md) §16. The `replay` subcommand that shares the restricted
> binary is unit 10's and is out of scope except where the two share the argument parser (§3.1). The source is
> dainty-restricted's `modules/rustic-isles/crates/isles-db` and `modules/rustic-isles/docs/db-migrate.md`, which are
> AGPL-3.0-only with Mystic Paradox's section 7 terms; an implementer who never sees them writes the MIT crate from
> this document alone.
>
> Everything below is stated from observable contracts: the command line, the documents read from MongoDB and the rows
> written, the documents written back, the derived ids, the report and its text, exit codes and error texts, the wire
> commands sent to MongoDB, and the tests. No implementation code is reproduced and no private helper is named.
> Node script names appear only as provenance. Game-derived values (starter catalog ids, default dyes) are **not** in
> this document: they are inputs the caller supplies (§14.3), as in the characters spec.
>
> Written against dainty-restricted `e0e238b4434316bb210677d88bc59e3448688794` (worktree
> `splashloader-integration-90130d`; the files of this unit last changed in `47c5498`, 2026-11-04) and dainty-lass
> `98739f8005c2aea1b0d27de907d473f1c50d080e` (branch `feat/rustic-isles-clean-room`).

---

## 1. Provenance and licensing classification

### 1.1 Evidence used

| Source | What it says |
| --- | --- |
| `modules/rustic-isles/NOTICE.md` (via the store spec §1.1) | The module is a modified Mystic Paradox; every file AGPL-3.0-only with the section 7 terms |
| File headers (`//!`) | `migrate.rs`: RU7 + RB11, writes under `system:migrate`; `transforms.rs`: the five RB11 repairs, each named after a Node `scripts/migrate_*.ts`; `mongo.rs`: read-only OP_MSG + BSON client and an in-process test server; `verify.rs`: counts and canonical SHA-256; `export.rs`: RB13 |
| `git log` of `crates/isles-db` | 15 commits, one author (the maintainer), `ed35716` (2026-10-27, RU7/RB11) to `47c5498`; RB13 in `692a816`, `07b21a5`, `a1ebe14`, `ee30229`; the limit-1 probe in `14c371d`; replay in `0a29728` |
| `modules/rustic-isles/docs/db-migrate.md` | The host-facing description: flags, area-to-collection table, DD18 ids and idempotence, the five RB11 repairs |
| `crates/isles-db/tests/migrate.rs` | Authored integration tests (described in §16, not copied) |
| `crates/dainty-host/src/lib.rs` (restricted) | Read for the seam only: how `reforged-host db` calls the library (§14.4) |
| dainty-lass `crates/dainty-hostkit/src/commands/db.rs`, `cli.rs`, `hooks.rs` | The MIT host kit's `db` command and its hook trait (§14.4) |
| dainty-lass `crates/dainty-store/src/engine.rs`, `history.rs` | `PostgresConfig` (its own redacted `Debug`, `check_address`), `Actor` |

### 1.2 Classification and what is rewritten

Category A is code written for Dainty Lass with no Paradox code in it; Category B ports Paradox TypeScript.

| Restricted file (`crates/isles-db/`) | Lines | Cat. | Source | Rewritten as |
| --- | ---: | :-: | --- | --- |
| `src/lib.rs` (argument parser, usage text) | 530 | A (the replay half is unit 10's) | RU7, RB13 | §3 |
| `src/main.rs` | 161 | A | | §12 (output, exit codes) |
| `src/mongo.rs` | 1067 | A | MongoDB wire protocol (public spec) | §4; the test server §16.1 |
| `src/migrate.rs` | 3338 | A with B data rules | Node `ParadoxBackend` collection shapes, DD18 | §6, §7, §9 |
| `src/transforms.rs` | 598 | B | `scripts/migrate_launcher_approval.ts`, `migrate_inventory_storage.ts`, `migrate_starter_instance_refs.ts`, `migrate_banner_instance_ids.ts`, `migrate_loadout_slot_entitlements.ts` | §5, §6, §8 |
| `src/verify.rs` | 1758 | A | | §10 |
| `src/export.rs` | 874 | A with B shapes | Node collection shapes | §11 |
| `src/schema.rs` | 511 | schema facts | isles-core's migrations | not rewritten: the tables are the store spec's §4 (§14.2) |
| `src/replay.rs` | 202 | A | RU8 | unit 10 |

### 1.3 What stays in dainty-restricted

- The `isles-db` binary until the composer uses this crate, the 1.14.7 starter values (the version crate's
  `StarterKit` and the two banner defaults, §14.3), `crates/dainty-host`'s `reforged-host db` wiring (it switches its calls to
  this crate), the replay CLI (unit 10), and the restricted tests' sample collections.
- When this crate lands and restricted's host calls it, `crates/isles-db` is deleted there except what unit 10 keeps.

---

## 2. Conventions

- **Times** written to the store are `Date#toISOString` text (`2026-01-01T00:00:00.000Z`) where the source could be
  read as a time (§5.3); otherwise the source text is kept as it came. TTL columns are Unix milliseconds.
- **JSON** is `serde_json` with `preserve_order` and `float_roundtrip` enabled in this crate's own manifest (core §17
  item 8). "Compact text" is `serde_json`'s compact form. An `f64` written into JSON or hashed keeps `serde_json`'s float
  text (`1200.0`, never `1200`).
- **Ids**: an account is `account:<ULID>@<node>` (DD18; core §2.2 `AccountId`), a party `party:<ULID>@<node>`
  (`module_api::GlobalId`). The client's `userId` is kept in `accounts.user_id` and the Node `partyId` in
  `parties.client_id`.
- **Source document**: one MongoDB document decoded to a JSON object (§4.4). "Field `a|b|c`" means the **first of
  those keys present in the document, whatever its value** (`null` and `""` included), read with the reader named
  (§5); when that value yields nothing the default applies and the later keys are **not** tried (`{"userId": null,
  "_id": "u1"}` has no user id, §17 P19). "Default" is used when no key is present.
- **Area**: one of `auth`, `characters`, `inventory`, `progression`, `bounties`, `parties`, `reforged` (§3.3).
- **Legacy names**: none. Nothing here reads or writes a `MYSTICPARADOX_*` variable or an `x-mysticparadox-*` header
  (core §17 item 2). The hash prefixes `isles-db:` of §6 are **values**, not names: every id already derived in a
  migrated database depends on them, so they are kept byte for byte.

---

## 3. Commands and arguments

### 3.1 Shape

The crate is a library with a command-line front (§14.1). The first argument selects the command:

| First argument | Command |
| --- | --- |
| none, `-h`, `--help`, `help` | Help (usage text on stdout, exit 0) |
| `migrate` | §7 to §9 |
| `verify` | §10 |
| `export-mongo` | §11 |
| `replay` | unit 10's parser (not this crate's; restricted keeps routing it there) |
| anything else | error `unknown subcommand "<arg>"; expected 'migrate', 'verify', 'export-mongo', or 'replay'` (the argument in Rust `Debug` quoting) |

Arguments after the command are read left to right. A flag that takes a value takes the **next argument whatever it
is** (so `--to --dry-run` sets the target to the text `--dry-run`); a missing value is an error naming the flag
(table below). A value flag given twice keeps the last; `--area` accumulates. `-h`/`--help` after the command answers Help
as soon as it is reached (an error in an earlier argument still wins; later ones are never looked at). Any other argument is
`unexpected flag "<arg>"`.

### 3.2 `migrate` and `verify` flags (one parser; `verify` is the same options)

| Flag | Value | Missing-value error | Meaning |
| --- | --- | --- | --- |
| `--from`, `--against` | MongoDB URI | `<flag> requires a MongoDB URI` (the flag as typed) | The source (§4.1). Both names work for both commands |
| `--to` | SQLite path, `sqlite://<path>`, `postgres://...` or `postgresql://...` | `--to requires a target path or postgres:// URL` | The store (§9.1) |
| `--node` | node id | `--node requires a node ID` | DD18 node for derived ids (§6); default `local`; parsed by `module_api::NodeId`'s `FromStr`, failure `invalid --node "<text>": <error>` |
| `--area` | area list | `--area requires an area name` | Comma-separated, repeatable (§3.3); empty items ignored |
| `--game-data-dir` | folder | `--game-data-dir requires a path` | Where the storage policy is looked for (§8.2) |
| `--dry-run` | | | migrate: read and transform only, never open `--to` (§7.1). verify: accepted and ignored |
| `--skip-invalid` | | | §7.3 |
| `--allow-remote-postgres` | | | Allows a Postgres host off this machine (§9.1) |
| `--json` | | | Report as JSON (§12) |

After the loop: no `--from`/`--against` → `missing required --from <mongodb://...>`; no `--to` → `missing required
--to <sqlite-file|postgres-url>`, **except** with `--dry-run`, where the target is the empty text (so `verify
--dry-run` without `--to` parses, then fails at open with `--to target cannot be empty`). The node is parsed after
both checks.

### 3.3 Areas

Each item is trimmed and ASCII-lower-cased, then:

| Area | Accepted spellings |
| --- | --- |
| `auth` | `auth`, `accounts` |
| `characters` | `characters`, `loadouts`, `characters-loadouts`, `characters_loadouts` |
| `inventory` | `inventory`, `inventory-reforge`, `inventory_reforge` |
| `progression` | `progression` |
| `bounties` | `bounties`, `bounties-store`, `bounties_store`, `wallets` |
| `parties` | `parties`, `matchmaking`, `chat`, `friendships` |
| `reforged` | `reforged`, `reforged-api`, `reforged_api`, `admin`, `guard` |

Anything else: `unknown area "<lowered>"; valid areas: auth, characters, inventory, progression, bounties, parties,
reforged`. No `--area` means all seven. Duplicates are harmless (the selection is a set). The areas' canonical order
(used in reports) is the table's.

### 3.4 `export-mongo` flags

| Flag | Value | Missing-value error | Meaning |
| --- | --- | --- | --- |
| `--from` | SQLite path, `sqlite://`, `postgres://` or `postgresql://` | `--from requires a SQLite path or postgres:// URL` | The store read from |
| `--to` | MongoDB URI | `--to requires a MongoDB URI` | The MongoDB written (§11.3) |
| `--node` | node id | `--node requires a node ID` | Parsed and validated (same error as §3.2), then **ignored** |
| `--area` | as §3.3 | `--area requires an area name` | Which collections are produced |
| `--dry-run`, `--allow-remote-postgres`, `--json` | | | As §3.2; dry-run never connects to MongoDB |

`--against`, `--skip-invalid` and `--game-data-dir` are `unexpected flag` here. Missing: `missing required --from
<sqlite-file|postgres-url>`; `missing required --to <mongodb://...>` unless `--dry-run`.

### 3.5 Usage text

Free text on stdout for Help, and after a parse error (§12.4). It lists the four commands with their required flags,
then each option with a one-line meaning, then `--json` and `-h, --help`. Its wording is not a contract; the replay
options are unit 10's to describe.

---

## 4. The MongoDB side

### 4.1 The URI

Only `mongodb://` is accepted (`mongodb+srv://` and anything else: `URI must start with mongodb://`). After the
scheme:

1. A `?` splits off the query; the first `/` after that splits the authority from the database path.
2. In the authority, the **last** `@` splits credentials from the host list. Credentials must contain `:`
   (`credentials must be username:password`); user and password are each percent-decoded as UTF-8 (`invalid
   username encoding: <e>`, `invalid password encoding: <e>`). An empty password after `:` is a password of "".
3. Hosts are comma-separated, trimmed, empty entries skipped. Each `host:port` splits at its **last** `:`; a port that
   is not a `u16` is `invalid port in "<entry>"`; no colon means port 27017; an empty host is `empty host in MongoDB
   URI`; none at all is `no hosts in MongoDB URI`. (So a bracketed IPv6 literal without a port is misread.)
4. The database is the path with `/` trimmed from both ends, percent-decoded (`invalid database name: <e>`); empty
   means `dauntless`.
5. In the query, `&`-separated `key=value` pairs; a key equal to `authSource` ignoring ASCII case with a non-empty value
   sets the auth database (percent-decoded, `invalid authSource: <e>`); default `admin`. **Every other option is
   ignored**, `replicaSet`, `tls`/`ssl` and `readPreference` included.

All URI errors display as `invalid MongoDB URI: <detail>`. The parsed URI's `Debug` shows hosts, database, user and
auth source and **never the password** (a departure from restricted's derived `Debug`, by the decision "secrets never
in Debug").

### 4.2 Connecting

Hosts are tried in the order given; the first that connects, answers the handshake and (with credentials)
authenticates is used, with no replica-set discovery and no primary selection. When every host fails the last host's
error is the result. For each host: the name is resolved and the **first** address used; TCP connect with a 30 s
timeout; read and write timeouts 30 s. Plain TCP: no TLS.

Handshake: the command `{hello: 1, $db: "admin"}`; it must answer `ok` 1 (§4.3). A server that does not know `hello`
(older than MongoDB 4.4.2's command set) fails here. No client metadata is sent.

### 4.3 Commands and replies (OP_MSG)

Every request is one OP_MSG (opcode 2013) message: the 16-byte header (total length, a request id counting from 1 per
connection, `responseTo` 0, opcode), flag bits 0, one kind-0 section holding the command document. Field types on the
wire are exact: `hello`, `limit`, `batchSize`, `conversationId`, `autoAuthorize`, `saslStart`, `saslContinue` int32;
`getMore` int64; `singleBatch` boolean; `payload` binary subtype 0; names and `$db` strings.

A reply is read as: the 16-byte header; opcode not 2013 is `unexpected MongoDB response opcode <n>`; a total length
outside 21 to 64 MiB inclusive is `invalid MongoDB response length <n>`; then the rest of the message. After the 4
flag bytes, sections are walked: kind 0 is decoded as the reply document (§4.4) and ends the walk; kind 1 is skipped
by its declared size; any other kind is `unknown OP_MSG section kind <k>`; a kind-1 section cut short is `truncated
OP_MSG kind 1 section`; no kind-0 section is `OP_MSG response contained no body section`. The request id echoed in
`responseTo` and the checksum flag are not checked.

**A reply is a success** when its `ok` reads as a number equal to 1 (missing reads 0). Otherwise the command fails
with `<command>: <errmsg>` (`errmsg` text, default `command returned ok != 1`). A successful reply that carries a
non-empty `writeErrors` array fails with `<command>: <first entry's errmsg>` (default `writeError reported by
MongoDB`). Command names in these texts: `hello`, `saslStart`, `saslContinue`, `saslContinue(final)`, `find`,
`getMore`, `delete`, `insert`. They display as `MongoDB command error: <text>`.

**Reading a collection** (migrate and verify): `{find: <collection>, batchSize: 500, $db: <database>}`; the reply's
`cursor` object is required (`find(<collection>): missing cursor object`); documents from `cursor.firstBatch`, then
while `cursor.id` (int64, missing 0) is not 0, `{getMore: <id>, collection: <collection>, batchSize: 500, $db}`,
documents from `cursor.nextBatch` (`getMore(<collection>): missing cursor object`). No filter, no sort (natural
order), no session, no `killCursors` on failure. Batch entries that are not documents are skipped. A collection that
does not exist reads as empty. The whole collection is held in memory.

**Probe** (kept for the host kit's update path): `{find: <collection>, limit: 1, singleBatch: true, batchSize: 1,
$db}` answers whether `cursor.firstBatch` is a non-empty array.

**Replacing a collection** (export only, §11.3): `{delete: <collection>, deletes: [{q: {}, limit: 0}], $db}`, then the
documents in chunks of 500, each chunk `{insert: <collection>, documents: [...], $db}`. Zero documents means no insert
(the collection is left empty). No ordered flag, no write concern.

**The command set is the contract that keeps migrate and verify read-only**: they send only `hello`, `saslStart`,
`saslContinue`, `find` and `getMore`. The rewrite should make that structural (the reading client has no write
method; §15).

### 4.4 BSON to JSON (decoding)

A document is: an int32 total length that must be at least 5 and not more than the bytes available, a final NUL byte
(`BSON document missing trailing NUL`), then elements until the final byte. Shorter than 5 bytes: `BSON document too
short: <n> bytes`; bad length: `invalid BSON document length <len> (slice len <n>)`. Each element is a type byte, a
NUL-terminated UTF-8 key (`unterminated C string in BSON`, `invalid UTF-8 in BSON key: <e>`), then the value:

| Type | Becomes |
| --- | --- |
| 0x01 double | a JSON number; NaN and infinities become `null` |
| 0x02 string, 0x0D JavaScript, 0x0E symbol | a string: int32 length (0 or past the end: `invalid BSON string length <n>`), NUL-terminated (`BSON string missing trailing NUL`), UTF-8 (`invalid UTF-8 in BSON string: <e>`) |
| 0x03 document | an object (length below 5 or past the end: `invalid BSON subdocument length <n>`) |
| 0x04 array | an array of the values in element order; the index keys are not checked (`invalid BSON array length <n>`); two elements with the same key collapse to one, keeping the first one's position and the last one's value |
| 0x05 binary | the payload in standard Base64 with padding; the subtype is dropped |
| 0x06 undefined, 0x0A null, 0x7F max key, 0xFF min key | `null` |
| 0x07 ObjectId | 24 lowercase hex digits |
| 0x08 boolean | any non-zero byte is `true` |
| 0x09 datetime | `toISOString` text of the int64 milliseconds (a value outside chrono's range reads as `1970-01-01T00:00:00.000Z`) |
| 0x0B regex | `/<pattern>/<options>` |
| 0x10 int32 | a number |
| 0x11 timestamp, 0x12 int64 | the 8 bytes read as a signed int64, a number |
| 0x13 decimal128 | 32 lowercase hex digits of the 16 raw bytes in stored order (not the decimal value) |
| anything else (0x0C, 0x0F, ...) | `unsupported BSON element tag 0x<hh>` |

Each fixed-size value cut short is `truncated BSON <double|string length|subdocument|array|binary|binary
payload|ObjectId|bool|datetime|int32|int64|decimal128>`. Duplicate keys in a document: last value wins at the first
key's position. Decode errors display as `BSON decode error: <detail>`.

**Nesting depth** is unbounded in restricted (a deeply nested reply can exhaust the stack). The rewrite caps it
(proposed: 200 levels, above MongoDB's own document nesting limit, so nothing a real server returns is refused) with
`BSON nesting deeper than 200` (§17 item 7).

### 4.5 JSON to BSON (encoding, export and commands)

An object becomes a document with its keys in order; `null` → null; booleans → boolean; a number that is an integer
fitting `i64` → int32 when it fits int32, else int64; any other number (fractional, or an `f64` even when integral, or
an unsigned above `i64::MAX`) → double; string → string; array → array with keys `0`, `1`, ...; object → document.
Restricted writes a key containing NUL as is (malformed BSON the server then refuses); the rewrite refuses it before
sending (`BSON key contains NUL: <collection>`), §17 item 8.

### 4.6 Authentication (SCRAM-SHA-256)

Only when the URI carries both a user and a password (§4.1). Only SCRAM-SHA-256; no SASLprep of the password; the
user name is escaped (`=` → `=3D`, `,` → `=2C`).

1. Client nonce: Base64 of 24 random bytes. Client-first-bare `n=<user>,r=<nonce>`; message `n,,` + bare.
2. `{saslStart: 1, mechanism: "SCRAM-SHA-256", payload: <message as binary>, autoAuthorize: 1, $db: <auth db>}`.
   The reply's `conversationId` (default 1) is reused; its `payload` (a Base64 string, as §4.4 decodes binary) is
   required: `missing payload in saslStart response`, `invalid base64 in saslStart: <e>`, `invalid UTF-8 in
   saslStart: <e>`.
3. The server-first message is split on `,`: `r=` nonce (`missing r= in SCRAM`), `s=` Base64 salt (`missing s= in
   SCRAM`, `invalid salt base64: <e>`), `i=` iterations as `u32` (unparseable or missing: `missing i= in SCRAM`). The
   server nonce must start with the client nonce (`server nonce did not prefix client nonce`).
4. Salted password = PBKDF2-HMAC-SHA-256 (one block) over the password and salt with the iterations; client key =
   HMAC(salted, `Client Key`); stored key = SHA-256(client key); auth message = bare `,` server-first `,`
   `c=biws,r=<server nonce>`; proof = client key XOR HMAC(stored key, auth message); client-final
   `c=biws,r=<server nonce>,p=<Base64 proof>`.
5. `{saslContinue: 1, conversationId, payload: <client-final>, $db}`. When the reply's `done` is not `true`, one more
   `saslContinue` with an empty payload (`saslContinue(final)`).

Authentication errors display as `MongoDB authentication failed: <detail>`.

**Departures proposed for the rewrite** (security; §17 items 5 and 6): verify the server signature `v=` in the
reply to step 5 (restricted ignores it, so a man in the middle can impersonate the server) and refuse an iteration
count above a cap (proposed 1 000 000; restricted runs whatever the server asks, a CPU sink).

---

## 5. Reading source fields

Every rule below reads a field that may be absent, `null`, a string, a number, a boolean, an array or an object.

### 5.1 Text

- **Text or none**: a non-empty string as is; a number as its JSON text (`123`, `1.5`); anything else (absent, null,
  `""`, boolean, array, object) none. Whitespace-only text is **not** none.
- **Text or default**: a string as is (`""` included); a number as its JSON text; anything else the default.

### 5.2 Numbers and booleans

- **Integer or default**: an integer number as is; a fractional number truncated toward zero (saturating); a string
  that parses as a whole `i64` (no sign-less trimming, no fraction: `"1.5"` is the default); `true` 1, `false` 0;
  anything else the default.
- **Float or default**: a number as `f64`; a string that Rust's `f64` parser accepts (so `"inf"` and `"NaN"` are
  accepted, §17 parity item P13); anything else the default.
- **Boolean or default**: a boolean as is; a number is true when its integer value is not 0 (a fractional number has
  none and reads false); a string is true when it equals `true` ignoring ASCII case or is exactly `1`, and **false for
  any other string** (not the default); absent or null or anything else the default.

### 5.3 Times

- **Time or none**:
  - absent, null: none.
  - a string: trimmed; empty is none; an RFC 3339 time becomes the UTC `toISOString` form (milliseconds, `Z`); else
    text that parses as a whole `i64` is read as epoch milliseconds; else **the trimmed text is kept as it is**.
  - a number: its integer value (fraction truncated); when its magnitude is below 10 000 000 000 it is **seconds** and
    is multiplied by 1000, else milliseconds; then the `toISOString` form.
  - an object with `$date` (extended JSON): the rule applied to that value; any other object: none.
  - anything else: none.
  - An instant outside the representable range reads as `1970-01-01T00:00:00.000Z`.
- **Time or default**: the above, or the default text when none.
- **Epoch milliseconds or default**: absent or null the default; a number its integer value (truncated); a string an
  RFC 3339 time's milliseconds, else a whole `i64`, else the default; an object's `$date` by the same rule; anything
  else the default. (No seconds heuristic here, unlike the time reader.)
- **The default time** wherever a row needs one and the source has none: `2026-01-01T00:00:00.000Z`.

### 5.4 JSON fields stored either way

Node stored some fields as native BSON and some as JSON text. Two readers:

- **Value**: absent or null → the default value; a string → its JSON parse, or the default value when it does not
  parse; anything else → itself.
- **JSON text**: absent or null → the default text; a string → **the string verbatim** when it parses as JSON (not
  re-compacted: spacing is kept), else the default text; anything else → its compact text.

---

## 6. Derived ids (DD18) and existing ids

### 6.1 Formulas (byte-exact: existing databases depend on them)

- **Deterministic ULID** of (namespace, node, key): SHA-256 of the UTF-8 bytes `isles-db:<namespace>:<node>:<key>`;
  the first 16 bytes as a big-endian 128-bit number; 26 Crockford Base32 characters
  (`0123456789ABCDEFGHJKMNPQRSTVWXYZ`), the first from the top 3 bits, then 25 groups of 5 bits from the top down.
- **Account id** of a user id: `account:<ULID("account", node, user id)>@<node>`, then parsed by core's `AccountId`
  (`invalid derived account id "<text>": <e>` on failure).
- **Party id** of a Node party id: `party:<ULID("party", node, party id)>@<node>`, parsed as a `GlobalId`
  (`invalid derived party id "<text>": <e>`).
- **Repair instance id** of (user id, character id, catalog id, ordinal): SHA-256 of
  `isles-db:instance:<user>:<character>:<catalog>:<ordinal in decimal>`; character `i` (0 to 25) is the alphabet
  `0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ` at index `(digest[i] + 7 × i) mod 36`.

Test vectors computed from these formulas by an independent script:

| Input | Output |
| --- | --- |
| account, node `local`, user `u1` | `account:402D98CJC48J8KBFQ1526NZ6ZS@local` |
| account, node `eu-west`, user `u1` | `account:4QDCTFADD9TFQBS5FDCZT7TWSB@eu-west` |
| party, node `local`, party `p1` | `party:01Y36W9SXJ680YT5TTN54MAA3R@local` |
| instance `u1`, `c1`, `CAT_A`, 0 | `2UDZFBXSIH29XDDAMOVB4TBBMQ` |
| instance `u1`, `c1`, `CAT_A`, 1 | `QRBAAJNT7N1UZ40DEGJA919MOQ` |

### 6.2 Resolving

Before extraction (not on a dry run), the target's `accounts` (`id`, `user_id`, every row, deleted ones included) and
`parties` (`id`, `client_id`) are read; a stored id that does not parse is skipped. Resolving a user id: trim it; empty
is the fatal error `invalid argument: empty userId` (**not** subject to `--skip-invalid`, §17 P2); an existing mapping
wins; else the derived id, remembered for the rest of the run. Every account resolved also gets an **implicit
account row** in the extracted state (§7.4 `accounts`) unless one is already there: `user_id` and `name` the trimmed
user id, `notes` 0. Parties resolve the same way (`empty partyId`), without an implicit row.

The **user id of a document** is field `userId|accountId|_id` read as text or none (§5.1).

---

## 7. `migrate`: extraction

### 7.1 Run

1. Connect to the source (§4.2).
2. **Dry run**: extract (§7.2) with no existing ids; report; nothing else (the target is neither opened nor created).
3. Otherwise open the target (§9.1), read the existing ids (§6.2), extract, then write everything in **one** store
   write transaction under the actor `store::Actor::System("migrate")` (logged `system:migrate`), §9.
4. The report (§12.1).

Extraction is not a snapshot: collections are read one after another while the source may still be changing (the
host is expected to have stopped the Node backend).

### 7.2 Collection order

Reads happen in this order, each only when its condition holds (S = the selected areas):

| Step | Collections | Read when |
| --- | --- | --- |
| 1 | `accounts`, `authIdentities`, `refreshSessions`, `gameExchangeCodes`, `discordOAuthTransactions`, `emailCodes`, `gameServerApiKeys`, `userApiKeys` | `auth` in S |
| 2 | `playerJourney` | `progression` or `characters` in S |
| 3 | `progressionTracks`, `progressionObjectives`, `progressionObjectiveEvents`, `progressionTransactions`, `escalationProgress`, `breadcrumbs`, `encounteredContent` | `progression` in S |
| 4 | `characters` | `characters` in S |
| 5 | `inventories` | `characters` or `inventory` in S |
| 6 | `loadouts` | `characters` in S |
| 7 | (the RB11 repairs of §8 over steps 5 and 6) | |
| 8 | `inventoryTransactions` | `inventory` in S |
| 9 | `bountyStates`, `cooldownStates`, `entitlements`, `wallets`, `economyDaily` | `bounties` in S |
| 10 | `partyMemberships`, `parties`, `partyInvites`, `partyOutbox`, `friendships` | `parties` in S |
| 11 | `reforgedInvites`, `adminSessions`, `adminAudit`, `launcherNotices`, `operationsPolicy`, `launcherGuardSessions`, `guardFindings` | `reforged` in S |

Within a collection, documents are taken in source (natural) order. Rows are keyed (§7.4 "key"); a later document
with the same key **replaces** the earlier one. The order matters for implicit account rows (§6.2), for `ord` (§7.4
`characters`) and for which document wins a key.

### 7.3 Invalid documents

A document missing a required field (the "required" column of §7.4, with the detail text given) is **invalid**:

- without `--skip-invalid`: the run stops with `invalid document in <collection>: <detail>` (nothing written);
- with it: a warn log `skipping invalid document in <collection>: <detail>` (fixed texts; never document contents),
  `skipped_invalid` + 1, and the document is ignored.

Some documents are dropped **silently** without counting: `playerJourney` and `inventories` documents without their
key when the area that owns them is not selected; `partyMemberships` lacking `userId|accountId|_id` or `partyId`;
entitlement entries and party members that are not usable (§7.4).

### 7.4 Collection to rows, field by field

Readers: **T** text or none, **T/d** text or default, **I/d** integer or default, **F/d** float or default, **B/d**
boolean or default, **t** time or none, **t/d** time or default, **ms/d** epoch ms or default, **V/d** value, **J/d**
JSON text (§5). "acct(x)" is the resolved account id text of user id x (§6.2); "uid" is the document's user id. All
`created_at`-style defaults are the default time unless stated.

**`accounts`** → `accounts` (key: account id) and, sometimes, `launcher_accounts` (key: account id).
Required: uid (`missing userId/_id`).

| Column | Source |
| --- | --- |
| `id` | acct(uid) |
| `user_id` | uid (untrimmed text as read; the id derivation trims) |
| `name` | `name|displayName` T, else uid |
| `notes` | `notes` I/d 0 |

The document is also a **launcher account** when it has any of the keys `displayNameNormalized`, `passwordHash`,
`roles`, `approvalStatus` (presence, even `null`; a nested `approval` alone does not count):

| Column | Source |
| --- | --- |
| `account_id` | acct(uid) |
| `email` | `email` T |
| `display_name` | `displayName` T, else the account `name` above |
| `display_name_normalized` | `displayNameNormalized` T, else `display_name` ASCII-lower-cased |
| `password_hash` | `passwordHash` T |
| `status` | `status` T/d `active` (not checked against the enum) |
| `approval_status`, `approval_updated_at`, `approval_updated_by`, `approval_reason` | §8.1 |
| `roles` | `roles` V/d `["user"]`, compact text |
| `roles_updated_at` | `rolesUpdatedAt` t |
| `roles_updated_by` | `rolesUpdatedBy` T |
| `created_at` | `createdAt` t/d |
| `last_login_at` | `lastLoginAt` t |
| `username_set` | when the key `usernameSet` is present: I/d 0 (so `null` is 0); absent: null |
| `consent_terms_version` | `consentTermsVersion`, else `consent.termsVersion`; T |
| `consent_privacy_version` | `consentPrivacyVersion`, else `consent.privacyVersion`; T |
| `consent_accepted_at` | `consentAcceptedAt`, else `consent.acceptedAt`; t |
| `deletion_requested_at` | `deletionRequestedAt`, else `deletion.requestedAt`; t |
| `deletion_scheduled_for` | `deletionScheduledFor`, else `deletion.scheduledFor`; t |
| `email_verified_at`, `email_verification_pending_since` | `emailVerifiedAt`, `emailVerificationPendingSince`; t |
| `timeout_until`, `timeout_since`, `timeout_reason` | not migrated (null on insert, untouched on update) |

"`a`, else `b`" here means `a` when the key is present at all (even null), otherwise `b`; a present-but-null `a` reads
none.

**`authIdentities`** → `auth_identities` (key: provider, subject). Required: uid (`missing userId`), `provider` T
(`missing provider`), `providerSubject|subject` T (`missing providerSubject`).

| Column | Source |
| --- | --- |
| `provider` | `provider` (not checked) |
| `provider_subject` | the subject |
| `account_id` | acct(uid) |
| `provider_username` | `providerUsername|username` T/d the subject |
| `provider_avatar_url` | `providerAvatarUrl|avatarUrl` T |
| `linked_at` | `linkedAt|createdAt` t/d |

**`refreshSessions`** → `refresh_sessions` (key: id). Required: `sessionId|id|_id` T (`missing sessionId/_id`), uid
(`missing userId`), `tokenHash` T (`missing tokenHash`). Columns: `id`; `token_hash`; `account_id` acct(uid);
`family_id` `familyId` T/d the id; `device_id` `deviceId` T/d `unknown`; `device_name` `deviceName` T/d `Launcher`;
`created_at`, `expires_at` t/d; `revoked_at` t.

**`gameExchangeCodes`** → `game_exchange_codes` (key: code hash). Required: `codeHash|_id` T (`missing codeHash`), uid
(`missing userId`). Columns: `account_id` acct(uid); `launcher_session_id`, `executable_sha256`, `runtime_sha256`,
`runtime_manifest_version`, `runtime_artifact_set_sha256` from their camelCase fields T/d ""; `build_changelist`
`buildChangelist` I/d 0; `runtime_channel` `runtimeChannel` T/d `stable`; `realm_id` `realmId` T; `created_at`,
`expires_at` t/d; `consumed_at` t.

**`discordOAuthTransactions`** → `discord_oauth_transactions` (key: state). Required: `state|_id` T (`missing state`).
Columns: `code_verifier` `codeVerifier|pkceVerifier` T/d ""; `created_at`, `expires_at` t/d; `consumed_at`,
`completion_expires_at`, `completion_consumed_at` t; `completion_code_hash` T; `account_id` acct(`userId|accountId`
T) when present else null (no `_id` fallback here); `link_account_id` acct(`linkUserId|linkAccountId` T) when present
else null.

**`emailCodes`** → `email_codes` (key: account, purpose). Required: uid (`missing userId`), `purpose` T (`missing
purpose`), `codeHash` T (`missing codeHash`). `purpose` is renamed `verify_email` → `verify`, `password_reset` →
`reset`, `discord_unlink` → `unlink`, other text kept. `attempts` I/d 0; `created_at`, `expires_at` t/d.

**`gameServerApiKeys`** → `game_server_api_keys` (a set). Required: `keyHash|key|_id` T (`missing keyHash`).

**`userApiKeys`** → `user_api_keys` (key: account; one per account, later wins). Required: uid (`missing userId`),
`keyHash|key` T (`missing keyHash`).

**`playerJourney`** → `player_journey` (key: account), written only when `progression` is selected; the parsed nodes
are kept per account for §8.4 whenever the collection is read. A document without uid is invalid (`missing userId`)
when `progression` is selected, silently dropped otherwise (its account is not resolved then). Columns: `nodes` V/d
`{}` as compact text; `update_version` `updateVersion` I/d 0; `revision` `revision` I/d 0.

**`progressionTracks`** → `progression_tracks` (key: account, progression id). Required: uid (`missing userId`),
`progressionId` T (`missing progressionId`). Columns: `progress`, `confirmed_fremium_rank`, `confirmed_premium_rank`,
`update_version` from `progress`, `confirmedFremiumRank`, `confirmedPremiumRank`, `updateVersion` I/d 0;
`created_at`, `updated_at` t/d; `reforge`: when non-null, J/d `{}`, else null; `prestige_claimed`: when non-null, a
boolean as 0/1, else I/d 0; null/absent → null.

**`progressionObjectives`** → `progression_objectives` (key: account, objective id). Required: uid (`missing userId`),
`objectiveId` T (`missing objectiveId`). `value`, `completed_count` from `value`, `completedCount` I/d 0;
`created_at`, `updated_at` t/d.

**`progressionObjectiveEvents`** → `progression_objective_events` (key: id). Required: `id|_id` T (`missing _id`), uid
(`missing userId`). `raw_body` `rawBody|body` J/d `{}`; `received_at` `receivedAt|createdAt` t/d.

**`progressionTransactions`** → `progression_grants` (key: grant id). Required: `grantId|transactionId|_id` T (`missing
grantId`), uid (`missing userId`), `progressionId` T (`missing progressionId`). `amount` I/d 0; `request_hash`
`requestHash` T/d ""; `status` `status` T/d `applied`; `result`: the `result` field (absent → `{}`); **when it is an
object its `userId` is set to the account id text** (added at the end if new, replaced in place if present); then J/d
`{}` (a string result is kept verbatim if it parses, unchanged); `created_at` t/d.

**`escalationProgress`** → `escalation_progress` (key: account, season). Required: uid (`missing userId`), `seasonId` T
(`missing seasonId`). `escalation_level`, `next_level_xp`, `update_version` I/d 0; `talents_progress`,
`unlock_progress` J/d `[]`; `created_at`, `updated_at` t/d.

**`breadcrumbs`** → `breadcrumbs` (key: character id). Required: `characterId|_id` T (`missing characterId`), uid
(`missing userId`). `items` `breadcrumbs|items` J/d `[]`; `update_version` I/d 0.

**`encounteredContent`** → `encountered_content` (key: character id). Required: `characterId|_id` T (`missing
characterId`), uid (`missing userId`). `content` `encounteredcontent|content` J/d `[]` (the lower-case key is Node's).

**`characters`** → `characters` (key: character id). Required: `characterId|_id` T (`missing characterId`), uid
(`missing userId`). A counter starting at 0 is incremented for each document that passed both checks, **across all
accounts**. Columns: `account_id` acct(uid); `ord` `ord` I/d the counter; `created_date` `createdDate|createdAt` t/d;
`last_modified_date` `lastModifiedDate|updatedAt` t/d `created_date`; `name` T/d the character id; `update_version`
I/d 0; `data` J/d `{}`.

**`inventories`** (pending, §8). Key: `characterId|_id` T (missing: invalid `missing characterId` when `inventory` is
selected, silently dropped otherwise). The user id is uid or ""; `account_id` is acct(uid) when uid is non-empty, else
the account of the same character from `characters` extracted in this run (only when `characters` is selected), else
null. `instanced_items`, `stacked_items` per §8.2; `revision` I/d 0; `bootstrap_version` T. Every inventory document
with a character id adds 1 to `rb11_inventory_arrays_transformed`.

**`loadouts`** (pending, §8). Required: `characterId|_id` T (`missing characterId`), uid (`missing userId`). Slots:
`loadouts` V/d `[]`, a non-array reading `[]`; `persistent` V/d `{}` (a non-object is kept as is); `active_index`
`activeIndex` I/d 0; persisted unlocked total: `unlockedTotalSlots` only when it is a number (as `f64`); `revision`
I/d 0; `bootstrap_version` T.

**`inventoryTransactions`** → `inventory_transactions` (key: account, character, transaction). Required: uid (`missing
userId`), `characterId` T (`missing characterId`), `transactionId|_id` T (`missing transactionId`). Columns:
`request_hash` T; `status` T/d `completed`; `result`: when non-null J/d `{}`, else null; `created_at` t/d;
`completed_at` t; `economy_source`, `economy_session_id` from `economySource`, `economySessionId` T; `extra`: an object
of **every other key** of the document in its order, except `_id`, `userId`, `accountId`, `characterId`,
`transactionId`, `requestHash`, `status`, `result`, `createdAt`, `completedAt`, `economySource`, `economySessionId`
(so `__v`, `updatedAt`, a key named `extra` and anything else go in), compact text; null when nothing is left.

**`bountyStates`** → `bounty_states` (key: account). Required: uid (`missing userId`). `bounties` J/d `[]`;
`draft_data` `draftData` J/d `{}`; `daily_reward_credited_on`, `daily_premium_draft_granted_on` T; `update_version`
I/d 0; `created_at`, `updated_at` t/d.

**`cooldownStates`** → `cooldown_states` (key: account). Required: uid (`missing userId`). `entries`
`entries|cooldowns` J/d `{}`; `update_version` I/d 0; `created_at`, `updated_at` t/d.

**`entitlements`** → `entitlements` (key: account, name). Required: uid (`missing userId`). The `entitlements` field
V/d `[]`; when an array, each entry that is an object with a `name` T becomes a row (others silently skipped; later
same name wins): `duration` F/d 0.0; `activated_date` `activatedDate` t; `granted_at` `grantedAt` t; `source_sku_id`
`sourceSkuId` T. Names are not validated here (the store's repositories validate on their own writes).

**`wallets`** → `wallets` (key: account) and `wallet_balances`. Required: uid (`missing userId`). `bootstrap_version`
T. `balances` V/d `{}`; when an object, each entry whose value is a number becomes a balance (catalog id = key, amount
as `f64`, **ordinal = the entry's position among all entries**, so a skipped non-number leaves a gap).

**`economyDaily`** → `economy_rollup_counters` (key: account, day, currency, path). Required: uid (`missing userId`),
`day` T (`missing day`), `currency` T (`missing currency`). `ttl_at` `ttlAt|expiresAt` ms/d 0. Rows: every top-level
key except `_id`, `userId`, `day`, `currency`, `ttlAt`, `expiresAt`, `__v` is flattened: a number leaf is a row with
path = the keys joined by `.` and value its `f64`; objects recurse; any other leaf (string, boolean, array, null) is
dropped. The flattened paths are sorted.

**`partyMemberships`** (read first in the area): each document with both a uid and `partyId` T records joined-at
`joinedAt` t/d for (party id, uid). Nothing else.

**`parties`** → `parties` (key: party id) and `party_members`. Required: `partyId|_id` T (`missing partyId/_id`),
`leaderPlayerId|leader` T (`missing leaderPlayerId`). Columns: `id` the derived party id; `client_id` the Node party
id; `revision` I/d 1; `leader` acct(leader); `build_id` `buildId` T/d `647472`; `activity` T/d `IDLE`;
`active_hunt_session_id` T; `created_at` t/d; `updated_at` t/d `created_at`. Members: each **string** entry of the
`members` array (non-strings skipped) in order: (acct(entry), position = its index in the array, joined-at from the
memberships for (Node party id, entry) else `created_at`). No usable member: the leader alone at 0 with `created_at`.

**`partyInvites`** → `party_invites` (key: recipient, sender). Required: `recipientPlayerId|recipient` T (`missing
recipientPlayerId`), `sendingPlayerId|sender` T (`missing sendingPlayerId`), `partyId` T (`missing partyId`).
Columns: `recipient`, `sender` acct(...); `party` the derived party id; `party_revision` I/d 1;
`sending_display_name` T/d the sender's user id; `sending_platform` T/d `PC`; `status` T/d `PENDING`; `created_at`,
`expires_at` t/d; `consumed_at` t.

**`partyOutbox`** → `party_outbox` (key: id). Required: `eventId|_id` T (`missing eventId/_id`), `partyId` T (`missing
partyId`). Columns: `party` **the Node party id as it is** (not the derived id, §17 P8); `party_revision` I/d 1;
`event_type` T/d `PARTY_CREATED`; `recipients` `recipientAccountIds|recipients` J/d `[]` (not mapped to account ids);
`payload` J/d `{}`; `state` T/d `PENDING`; `created_at` t/d; `claimed_at`, `published_at` t; `claim_owner` T.

**`friendships`** → `friend_edges` (key: owner, other). Required: `ownerId` T (`missing ownerId`), `otherId` T
(`missing otherId`). `owner`, `other` acct(...); `other_user_id` the raw `otherId`; `status` T/d `ACCEPTED`;
`direction` T; `created` t/d; `favorite` B/d false.

**`reforgedInvites`** → `reforged_invites` (key: code). Required: `invitecode|code|_id` T (`missing invitecode`).
`uses_remaining` `usesRemaining|uses_remaining` I/d 1; `infinite_uses` `infiniteUses|infinite_uses` B/d false; `note`
T; `created_at` t/d.

**`adminSessions`** → `admin_sessions` (key: id). Required: `id|_id` T (`missing id/_id`), uid (`missing userId`),
`tokenHash` T (`missing tokenHash`). `account` acct(uid); `csrf_token` T/d ""; `created_at` t/d; `expires_at` t/d;
`ip` T/d `127.0.0.1`; `user_agent` T/d ""; `revoked_at` t; `ttl_at` `ttlAt` ms/d (the `expires_at` text's
milliseconds, else 0).

**`adminAudit`** → `admin_audit` (key: id). Required: `id|_id` T (`missing id/_id`). `actor_user_id` T/d `system`
(the Node user id, not mapped); `target_user_id` T (not mapped); `action` T/d `UNKNOWN`; `old_state`, `new_state`:
when non-null J/d `{}`, else null; `reason` T; `ip` T/d `127.0.0.1`; `request_id` T/d ""; `created_at` t/d. Accounts
are not resolved for this collection.

**`launcherNotices`** → `launcher_notices`: the **first** document only (others ignored): `id` 1; `version` I/d 1;
`updated_at` t; `updated_by` T; `announcements`, `tags` J/d `[]`.

**`operationsPolicy`** → `operations_policy`: the first document only: `id` 1; `guard_enforcement`
`guardEnforcement` T/d `ENFORCE`; `diagnostics_profile` T/d `PRODUCTION`; `version` I/d 1; `updated_at` t;
`updated_by` T; `diagnostics_expires_at` t.

**`launcherGuardSessions`** → `launcher_guard_sessions` (key: guard session id). Required: `guardSessionId|_id` T
(`missing guardSessionId`), uid (`missing accountId`). `account` acct(uid); `role` `host` when `role` T/d `client` is
exactly `host`, else `client`; `public_key_base64`, `manifest_id`, `challenge` T/d ""; `manifest_sequence`,
`last_sequence` I/d 0; `channel` T/d `stable`; `status` T/d `CHALLENGED`; `risk_score` F/d 0.0; `created_at`,
`expires_at` t/d; `last_heartbeat_at` t; `ttl_at` `ttlAt` ms/d 0; `detail`: when the key `detail` is present, J/d
`{}` (a present `null` gives `{}`); otherwise the compact text of the object, keys in this order: `launcherVersion`
(T/d ""), `lastSignals` (the raw value, or null), `reasons` (V/d `[]`), `history` (V/d `[]`).

**`guardFindings`** → `guard_findings` (key: file name, sha256). Required: `fileName` T (`missing fileName`), `sha256`
T (`missing sha256`). `account_ids`: `accountIds` V/d `[]`; each string entry that starts with `account:` is kept as
is, each other string is resolved to an account (an empty one silently dropped), non-strings dropped; compact JSON
array. `signer` T/d ""; `size` I/d 0; `first_seen_at`, `last_seen_at` t/d; `session_count` I/d 1; `account_count`
I/d the number of mapped ids; `ttl_at` `ttlAt` ms/d 0.

---

## 8. The RB11 repairs

### 8.1 Launcher approval (`migrate_launcher_approval`)

The status is `approvalStatus` when it is a string, else `approval.status` when that is a string; trimmed; empty is
absent.

- **Absent**: status `approved`, `approval_updated_at` = the row's `created_at`, `approval_updated_by` =
  `migration:0.1.73`, reason null. Counted in `rb11_launcher_approvals_backfilled` (the count uses the same test).
- **Present**: the trimmed status (not checked against `pending`/`approved`/`rejected`); `approval_updated_at` =
  time-or-none of `approvalUpdatedAt` when the key is present, else of `approval.updatedAt`; `approval_updated_by` =
  `approvalUpdatedBy` when a string, else `approval.updatedBy` when a string, non-empty, else null; `approval_reason`
  likewise from `approvalReason` / `approval.reason`.

### 8.2 Inventory arrays (`migrate_inventory_storage_arrays`)

`instancedItems` and `stackedItems` are each read V/d `[]`; a non-array is `[]`. When a storage policy was loaded, the
pair is passed through economy's storage repair on read (economy-inventory spec §8.4) and its two output arrays are
used (its change and issue lists are ignored, not logged). Without a policy the arrays are kept.

The policy is looked for only when `--game-data-dir` is given: `<dir>/inventory_storage_policy.json`, then
`<dir>/data/inventory_storage_policy.json`; the first that `StoragePolicy::load` accepts is used; when neither loads,
the run continues **without a policy and without a message** (§17 P10). One policy for the whole run.

### 8.3 Starter and banner instances (`migrate_starter_instances`, `migrate_banner_instances`)

For every character id present in the pending inventories or the pending loadouts (in character-id order), with the
starter values of §14.3:

| Has inventory | Has loadout | Repair runs on | User id used |
| --- | --- | --- | --- |
| yes | yes | the inventory's instanced items, the loadout's slots and `persistent` | the inventory's user id, or the loadout's when that is empty |
| yes | no | the instanced items only | the inventory's (may be "") |
| no | yes | an **empty throwaway** item list, the slots and `persistent` | the loadout's |

Each of the three adds 1 to `rb11_starter_instances_repaired` (it counts characters visited, not repairs).

The repair:

1. **Item ids.** Walk the instanced items by position `p`. Skip an entry that is not an object or has no string
   `catalogId`. Its current id is `instanceId` when a string, else "". A current id that is **valid** (exactly 26
   characters of `A`-`Z`, `0`-`9`) and not seen yet in this list is kept. Otherwise, when the catalog id is a starter
   instanced id **or** the current id is invalid, a new id is the repair instance id (§6.1) of (user, character,
   catalog, `p`), then `p + 1`, ... until one not yet seen. Otherwise (valid, a duplicate, not a starter) the
   duplicate is kept. The final id is written back as `instanceId` (a string) and remembered as seen; the **first**
   final id per catalog id is that catalog's id.
2. **Seeded items** (only when a loadout side was given): for the banner, then the glider catalog id, when the list
   has no item of it, an item is appended with id = repair instance id of (user, character, catalog, list length),
   then + 1, ... until unseen, keys in this order: `instanceId`, `catalogId`, `grantDate`
   `2026-01-01T00:00:00.000Z`, `markedNew` false, `craftingTrackId` "", `durability` 100, `currentXp` 0, `surged` 0,
   `talents` (25 × `0`).
3. **Slots**: in every slot that is an object, for each equipment pair (slot key, catalog id) of §14.3 whose catalog
   id has an id from steps 1 and 2: when the slot's member under that key is an **object** whose `instance_id` is not
   a valid id present in the seen set, `instance_id` is set to the catalog's id. A member that is a string, or
   absent, is left alone.
4. **`persistent`** (only when it is an object):
   - banner (when the banner catalog has an id): `banner` absent or not an object becomes `{}`; `fabric_id` and
     `pole_id`, each when not a valid seen id, become the banner's id; `sigil_id` (the default sigil), `dye_aspect1_id`,
     `dye_aspect2_id`, `sigil_PrimaryDyeId`, `sigil_SecondaryDyeId` (the default dye), each when not a non-empty
     string; `border_id`, `title_id`, `landing_id` "" and `update_version` 0, each only when absent.
   - glider (when the glider catalog has an id): likewise `glider` becomes an object; `instance_id` the glider's id
     when not a valid seen id; `PrimaryDyeId`, `SecondaryDyeId` the default dye when not a non-empty string;
     `Aspect1Id` "" and `update_version` 0 when absent.

This is the legacy (Node 0.1.x) object shape of `persistent.banner` and `persistent.glider`. The 1.14.7 starter
manifest writes both as **instance id strings** (characters spec §6.4); a string there is replaced by an object by
this step. Parity, flagged as §17 item 2.

### 8.4 Loadout slots (`migrate_loadout_slots`)

For each pending loadout, when `characters` is selected (always, in practice, since loadouts are read only then):

1. Journey total = store §7.3's `resolve_player_journey_total` of the account's journey nodes (from §7.4
   `playerJourney`; none when the account has no journey document).
2. Persisted = the persisted unlocked total truncated to an integer and clamped to 1..`MAX_TOTAL_LOADOUT_SLOTS` (6),
   when present.
3. Target = store §7.3's `resolve_requested_total(persisted, journey total)`, or `DEFAULT_ACCOUNT_LOADOUT_SLOTS` (1)
   when that is an error; clamped to 1..6.
4. When the slot list is not empty: while it is shorter than the target, append a copy of slot 0 (taken before any
   append) in which `custom_name` is "", `update_version` 0 and `slot_index` the new slot's index (set or added; a
   non-object slot 0 is copied unchanged). The list is never shortened.
5. `unlocked_total_slots` = max(list length, target). `active_index` = the stored one clamped to 0..length − 1, or 0
   for an empty list.

`rb11_loadout_slots_reconciled` + 1 per loadout. The loadout row: `character_id`; `account_id` acct(uid); `loadouts`
the slots' compact text; `persistent` compact text; `active_index`; `unlocked_total_slots`; `revision`;
`bootstrap_version`.

### 8.5 Which pending rows are written

A repaired loadout becomes a `loadouts` row when `characters` is selected; a repaired inventory becomes an
`inventories` row when `inventory` is selected. So with `characters` alone, inventories are read and repaired (and
banners and gliders may be seeded into them) but not written, and the loadouts then point at ids that exist only in
that unwritten list (§17 P6).

### 8.6 Not covered

Node's `scripts/` also holds `migrate_reward_cache_currency.ts`; restricted has no equivalent and this crate adds none
(§17 item 12).

---

## 9. `migrate`: writing

### 9.1 Opening the target (shared with verify and export)

The target text is trimmed; empty is `--to target cannot be empty`. `postgres://` or `postgresql://` is a Postgres
URL; otherwise a leading `sqlite://` is dropped and the rest is a SQLite file path. The store is opened with
`store::Database::open(config, DatabaseOptions::default(), rustic_isles_store::SCHEMA)`, which **creates a missing
SQLite file and brings any database to the latest migration** (§17 item 1).

Postgres URL, after the scheme:

1. The query (`?...`) is dropped. There must be a `/` (`Postgres URL must include /<database>`); the database is the
   text after the first `/` with `/` trimmed, empty meaning `postgres`.
2. The authority's **last** `@` splits credentials; the credentials' **first** `:` splits user and password (no `:`:
   all user, password ""); no credentials: user `postgres`, password "". **No percent-decoding** of any part
   (§17 P11).
3. Host and port split at the **last** `:`; a port that is not a `u16` is `invalid Postgres port in "<host:port>"`;
   default 5432.
4. `allow_remote` = `--allow-remote-postgres`; then `store::PostgresConfig::check_address`'s refusal text is the error
   for a host off this machine.

Argument errors display as `invalid argument: <detail>`; store errors as `store: <store's text>`.

### 9.2 Order and upsert rule

Inside the one write transaction, tables are written in this order: `accounts`, `launcher_accounts`,
`auth_identities`, `refresh_sessions`, `game_exchange_codes`, `discord_oauth_transactions`, `email_codes`,
`game_server_api_keys`, `user_api_keys`, `characters`, `loadouts`, `inventories`, `inventory_transactions`,
`bounty_states`, `cooldown_states`, `entitlements`, `wallets` (each followed by its `wallet_balances`),
`economy_rollup_counters`, `progression_tracks`, `progression_objectives`, `progression_objective_events`,
`progression_grants`, `escalation_progress`, `player_journey`, `breadcrumbs`, `encountered_content`, `parties` (each
followed by its `party_members`), `party_invites`, `party_outbox`, `friend_edges`, `reforged_invites`,
`admin_sessions`, `admin_audit`, `launcher_notices`, `operations_policy`, `launcher_guard_sessions`,
`guard_findings`. Within a table, rows go in their key's order (byte order of the key texts, tuple keys compared
element by element).

Every row is an **update by key, then an insert when no row matched**:

| Table | Key matched | Updated columns |
| --- | --- | --- |
| most | the primary key of store §4 (for `friend_edges` the unique (`owner`, `other`); `guard_findings` (`file_name`, `sha256`); `admin_audit` `id`) | every column of §7.4 except the key |
| `game_server_api_keys` | `key_hash` | none: inserted only when absent |
| `user_api_keys` | `account_id` | `key_hash` |
| `wallets` / `wallet_balances` | `account_id` / (`account_id`, `catalog_id`) | `bootstrap_version` / `balance`, `ordinal` |
| `party_members` | `account` only | `party`, `position`, `joined_at` (so a member listed in two parties ends in the later one) |
| `launcher_notices`, `operations_policy` | `id` 1 | every other column |

- **Never touched**: `deleted_at` (an update hits a soft-deleted row too and leaves it deleted, so the migrated data
  stays invisible, §17 P4); `launcher_accounts`' three `0610` columns; identity keys (`friend_edges.id`,
  `admin_audit.seq`, `guard_findings.id`, left to the database).
- **Sequence columns** (`progression_tracks.seq`, `progression_objectives.seq`, `party_invites.seq`,
  `party_outbox.seq`): read once per table as the maximum present (deleted rows included; none → 0) at the start of
  that table; each **inserted** row takes the next number, in key order (not the source's natural order, §17 P7); an
  updated row keeps its number.
- **Nothing is deleted**: target rows absent from the source stay. Party members not in the source list stay.
- **Implicit accounts overwrite**: an implicit account row (§6.2) is written like any other, so a run with areas
  that do not include `auth` sets an existing account's `name` to its user id and `notes` to 0 (§17 item 3).

### 9.3 Atomicity and idempotence

- One store write: all tables commit together or nothing does; the change log records every insert and update with
  the actor `system:migrate` (DD23).
- Extraction failures (Mongo errors, invalid documents without `--skip-invalid`, an empty user id) happen before the
  write: the target is untouched (though opened and migrated, §9.1).
- Re-running against the same source and target writes the same values (ids come back from the target, §6.2), adds no
  rows and does not move sequence numbers; `verify` then passes again.

---

## 10. `verify`

### 10.1 Run

1. Connect to the source; open the target (§9.1; **a missing SQLite file is created**, §17 item 1); read the
   existing ids (§6.2).
2. **Expected**: the extraction of §7 with those ids and the same options (areas, node, skip-invalid, game data
   directory, starter values). Nothing is written.
3. **Actual**: the target read back in one read transaction (§10.2).
4. Compare (§10.3). All tables equal: the report, success. Any table different: the error `verification failed:
   <n> collection(s) did not match` and **no report is printed** (restricted; §17 item 4).

### 10.2 Reading back

Each table is read into the same canonical records (§10.4):

- The soft-delete tables of store §2.2 are read **live only** (`deleted_at` null); the others in full.
- `wallets` (live) get their live `wallet_balances` in (`account_id`, `ordinal`) order; balances of a wallet not read
  are dropped.
- `parties` get their `party_members` in (`party`, `position`) order; members of an unread party are dropped.
- `launcher_notices`: the first row, if any. `operations_policy`: the first row, if any, **except** that when the
  expected side has no policy, the migration's untouched seed row (id 1, `ENFORCE`, `PRODUCTION`, version 0, the three
  optional columns null) reads as no policy.
- `accounts`: when `--area` was given, only the actual rows whose id is also expected are compared; with all areas,
  every live account is.

### 10.3 Comparison and hashes

For each table of the selected areas, in this order (area order of §3.3; within an area the order given): auth
`accounts`, `launcher_accounts`, `auth_identities`, `refresh_sessions`, `game_exchange_codes`,
`discord_oauth_transactions`, `email_codes`, `game_server_api_keys`, `user_api_keys`; characters `characters`,
`loadouts`; inventory `inventories`, `inventory_transactions`; progression `progression_tracks`,
`progression_objectives`, `progression_objective_events`, `progression_grants`, `escalation_progress`,
`player_journey`, `breadcrumbs`, `encountered_content`; bounties `bounty_states`, `cooldown_states`, `entitlements`,
`wallets`, `economy_rollup_counters`; parties `parties`, `party_invites`, `party_outbox`, `friend_edges`; reforged
`reforged_invites`, `admin_sessions`, `admin_audit`, `launcher_notices`, `operations_policy`,
`launcher_guard_sessions`, `guard_findings`. All areas: 37 entries.

- **Key text**: a single key as is; a tuple key's parts joined with `:` (so two tuples that join to the same text
  collapse to one entry, the later winning, §17 P12); `game_server_api_keys` keyed by the hash itself;
  `launcher_notices` and `operations_policy` keyed `current`.
- **Record hash**: lowercase hex SHA-256 of the record's compact JSON (§10.4).
- **Table hash**: lowercase hex SHA-256 over, for each entry in key-text byte order, `<key>:<record hash>\n`.
- **Mismatches**, expected keys first in order: `missing record <key>`, `content hash mismatch for <key>: expected
  <hash>, got <hash>`; then actual keys not expected: `unexpected extra record <key>`.
- A table is **ok** when the counts, the table hashes and the mismatch list all agree.

### 10.4 Canonical records (field order is the hash's order)

Option values are `null` when absent; integers JSON integers; floats `serde_json` float text; booleans `true`/`false`.

| Table | Fields in order |
| --- | --- |
| `accounts` | `id`, `user_id`, `name`, `notes` |
| `launcher_accounts` | `account_id`, `email`, `display_name_normalized`, `display_name`, `password_hash`, `status`, `approval_status`, `approval_updated_at`, `approval_updated_by`, `approval_reason`, `roles`, `roles_updated_at`, `roles_updated_by`, `created_at`, `last_login_at`, `username_set`, `consent_terms_version`, `consent_privacy_version`, `consent_accepted_at`, `deletion_requested_at`, `deletion_scheduled_for`, `email_verified_at`, `email_verification_pending_since` |
| `auth_identities` | `provider`, `provider_subject`, `account_id`, `provider_username`, `provider_avatar_url`, `linked_at` |
| `refresh_sessions` | `id`, `token_hash`, `account_id`, `family_id`, `device_id`, `device_name`, `created_at`, `expires_at`, `revoked_at` |
| `game_exchange_codes` | `code_hash`, `account_id`, `launcher_session_id`, `build_changelist`, `executable_sha256`, `runtime_channel`, `runtime_sha256`, `runtime_manifest_version`, `runtime_artifact_set_sha256`, `realm_id`, `created_at`, `expires_at`, `consumed_at` |
| `discord_oauth_transactions` | `state`, `code_verifier`, `created_at`, `expires_at`, `consumed_at`, `completion_code_hash`, `completion_expires_at`, `completion_consumed_at`, `account_id`, `link_account_id` |
| `email_codes` | `account_id`, `purpose`, `code_hash`, `attempts`, `created_at`, `expires_at` |
| `game_server_api_keys` | the hash as a JSON string |
| `user_api_keys` | the hash as a JSON string |
| `characters` | `character_id`, `account_id`, `ord`, `created_date`, `last_modified_date`, `name`, `update_version`, `data` |
| `loadouts` | `character_id`, `account_id`, `slots_json` (the `loadouts` column), `persistent`, `active_index`, `unlocked_total_slots`, `revision`, `bootstrap_version` |
| `inventories` | `character_id`, `account_id`, `instanced_items`, `stacked_items`, `revision`, `bootstrap_version` |
| `inventory_transactions` | `account_id`, `character_id`, `transaction_id`, `request_hash`, `status`, `result`, `created_at`, `completed_at`, `economy_source`, `economy_session_id`, `extra` |
| `bounty_states` | `account_id`, `bounties`, `draft_data`, `daily_reward_credited_on`, `daily_premium_draft_granted_on`, `update_version`, `created_at`, `updated_at` |
| `cooldown_states` | `account_id`, `entries`, `update_version`, `created_at`, `updated_at` |
| `entitlements` | `account_id`, `name`, `duration`, `activated_date`, `granted_at`, `source_sku_id` |
| `wallets` | `account_id`, `bootstrap_version`, `balances` (an array of `[catalog_id, balance, ordinal]` arrays) |
| `economy_rollup_counters` | `account_id`, `day`, `currency`, `path`, `value`, `ttl_at` |
| `friend_edges` | `owner`, `other`, `other_user_id`, `status`, `direction`, `created`, `favorite` |
| `reforged_invites` | `code`, `uses_remaining`, `infinite_uses`, `note`, `created_at` |
| `admin_sessions` | `id`, `token_hash`, `account`, `csrf_token`, `created_at`, `expires_at`, `ip`, `user_agent`, `revoked_at`, `ttl_at` |
| `admin_audit` | `id`, `actor_user_id`, `target_user_id`, `action`, `old_state`, `new_state`, `reason`, `ip`, `request_id`, `created_at` |
| `launcher_notices` | `id`, `version`, `updated_at`, `updated_by`, `announcements`, `tags` |
| `operations_policy` | `id`, `guard_enforcement`, `diagnostics_profile`, `version`, `updated_at`, `updated_by`, `diagnostics_expires_at` |
| `launcher_guard_sessions` | `guard_session_id`, `account`, `role`, `public_key_base64`, `manifest_id`, `manifest_sequence`, `channel`, `challenge`, `last_sequence`, `status`, `risk_score`, `created_at`, `expires_at`, `last_heartbeat_at`, `detail`, `ttl_at` |
| `guard_findings` | `file_name`, `sha256`, `signer`, `size`, `first_seen_at`, `last_seen_at`, `session_count`, `account_count`, `account_ids`, `ttl_at` |
| `progression_tracks` | `account`, `progression_id`, `progress`, `confirmed_fremium_rank`, `confirmed_premium_rank`, `update_version`, `created_at`, `updated_at`, `reforge`, `prestige_claimed` |
| `progression_objectives` | `account`, `objective_id`, `value`, `completed_count`, `created_at`, `updated_at` |
| `progression_objective_events` | `id`, `account`, `raw_body`, `received_at` |
| `progression_grants` | `grant_id`, `account`, `progression_id`, `amount`, `request_hash`, `status`, `result`, `created_at` |
| `escalation_progress` | `account`, `season_id`, `escalation_level`, `next_level_xp`, `talents_progress`, `unlock_progress`, `update_version`, `created_at`, `updated_at` |
| `player_journey` | `account`, `nodes`, `update_version`, `revision` |
| `breadcrumbs` | `character_id`, `account`, `items`, `update_version` |
| `encountered_content` | `character_id`, `account`, `content` |
| `parties` | `id`, `client_id`, `revision`, `leader`, `build_id`, `activity`, `active_hunt_session_id`, `created_at`, `updated_at`, `members` (an array of `[account, position, joined_at]` arrays) |
| `party_invites` | `recipient`, `sender`, `party`, `party_revision`, `sending_display_name`, `sending_platform`, `status`, `created_at`, `expires_at`, `consumed_at` |
| `party_outbox` | `id`, `party`, `party_revision`, `event_type`, `recipients`, `payload`, `state`, `created_at`, `claimed_at`, `claim_owner`, `published_at` |

These records are also the export's input (§11). Their `Debug` prints no secret column (password hash, token hashes,
code hashes, verifiers, CSRF tokens, Guard challenges, key hashes): a departure from restricted's derived `Debug`, by
the decision "secrets never in Debug".

---

## 11. `export-mongo` (RB13 rollback)

### 11.1 Run

1. Open the source store (§9.1: **a missing SQLite file is created and migrated**, so a mistyped path exports an
   empty database, §17 item 1).
2. Read it as §10.2 does, with "the expected side has no policy" (so an untouched seed policy is not exported).
3. Build the collections of the selected areas (§11.2). Every collection of a selected area is produced, even with no
   documents.
4. The report counts every produced collection. Dry run: stop here (no MongoDB connection).
5. Connect to `--to` (§4.2) and **replace** each produced collection in collection-name order (§4.3): its documents
   are deleted, then the new ones inserted. Not transactional: a failure leaves the collections before it replaced,
   the failing one possibly emptied, the rest untouched.

"uid(a)" below is the `user_id` of live account `a`, or **`a`'s own text** when no live account has that id
(§17 P9). Time columns are written as **strings** (never BSON dates) and `ttlAt` as numbers (§17 item 9). JSON text
columns are parsed back to values; text that does not parse becomes the default value given. Optional columns that are
null are **omitted**; required ones are always written.

### 11.2 Store to collections

| Area | Collection | `_id` | Other keys, in order |
| --- | --- | --- | --- |
| auth | `accounts` (one per live account) | user id | `userId`, `name`, `notes`; when a live launcher row exists: `email`?, `displayName`, `displayNameNormalized`, `passwordHash`?, `status`, `approvalStatus`?, `approvalUpdatedAt`?, `approvalUpdatedBy`?, `approvalReason`?, `roles` (parsed, default `["user"]`), `rolesUpdatedAt`?, `rolesUpdatedBy`?, `createdAt`, `lastLoginAt`?, `usernameSet`?, `consentTermsVersion`?, `consentPrivacyVersion`?, `consentAcceptedAt`?, `deletionRequestedAt`?, `deletionScheduledFor`?, `emailVerifiedAt`?, `emailVerificationPendingSince`? (flat names, never the nested `consent`/`deletion`/`approval` shapes) |
| auth | `authIdentities` | `<provider>:<subject>` | `userId` uid, `provider`, `providerSubject`, `providerUsername`, `providerAvatarUrl`?, `linkedAt` |
| auth | `refreshSessions` | id | `sessionId`, `userId`, `tokenHash`, `familyId`, `deviceId`, `deviceName`, `createdAt`, `expiresAt`, `revokedAt`? |
| auth | `gameExchangeCodes` | code hash | `codeHash`, `userId`, `launcherSessionId`, `buildChangelist`, `executableSha256`, `runtimeChannel`, `runtimeSha256`, `runtimeManifestVersion`, `runtimeArtifactSetSha256`, `realmId`?, `createdAt`, `expiresAt`, `consumedAt`? |
| auth | `discordOAuthTransactions` | state | `state`, `codeVerifier`, `createdAt`, `expiresAt`, `consumedAt`?, `completionCodeHash`?, `completionExpiresAt`?, `completionConsumedAt`?, `userId`? uid, `linkUserId`? uid |
| auth | `emailCodes` | `<uid>:<purpose>` | `userId`, `purpose` (the store's `verify`/`reset`/`unlink`, **not** mapped back, §17 P14), `codeHash`, `attempts`, `createdAt`, `expiresAt` |
| auth | `gameServerApiKeys` | key hash | `keyHash` |
| auth | `userApiKeys` | uid | `userId`, `keyHash` |
| characters | `characters` | character id | `characterId`, `userId`, `ord`, `createdDate`, `lastModifiedDate`, `name`, `updateVersion`, `data` (default `{}`) |
| characters | `loadouts` | character id | `characterId`, `userId`, `loadouts` (default `[]`), `persistent` (default `{}`), `activeIndex`, `unlockedTotalSlots`?, `revision`, `bootstrapVersion`? |
| characters, without progression | `playerJourney` | as below | as below (**the whole collection is replaced**, §17 item 10) |
| inventory | `inventories` | character id | `characterId`, `userId` (uid of the row's account, else of the live character's account, omitted when neither), `instancedItems`, `stackedItems` (defaults `[]`), `revision`, `bootstrapVersion`? |
| inventory | `inventoryTransactions` | the ledger id: lowercase hex SHA-256 of the compact JSON array `[uid, character id, transaction id]` | first the `extra` object's keys in order (or, when `extra` is not a JSON object, `extra` as a string); then, overriding: `userId`, `characterId`, `transactionId`, `requestHash`?, `status`, `result`? (default `{}`), `createdAt`, `completedAt`?, `economySource`?, `economySessionId`?. `_id` is placed before `userId` among the core keys, after the extras |
| progression | `playerJourney` | uid | `userId`, `nodes` (default `{}`), `updateVersion`, `revision` |
| progression | `progressionTracks` | `<uid>::<progression id>` | `userId`, `progressionId`, `progress`, `confirmedFremiumRank`, `confirmedPremiumRank`, `updateVersion`, `createdAt`, `updatedAt`, `reforge`? (default `{}`), `prestigeClaimed`? |
| progression | `progressionObjectives` | `<uid>::<objective id>` | `userId`, `objectiveId`, `value`, `completedCount`, `createdAt`, `updatedAt` |
| progression | `progressionObjectiveEvents` | id | `id`, `userId`, `rawBody` (default `{}`), `receivedAt` |
| progression | `progressionTransactions` | grant id | `grantId`, `userId`, `progressionId`, `amount`, `requestHash`, `status`, `result` (default `{}`; when an object its `userId` is set to uid), `createdAt` |
| progression | `escalationProgress` | `<uid>::<season id>` | `userId`, `seasonId`, `escalationLevel`, `nextLevelXp`, `talentsProgress`, `unlockProgress` (defaults `[]`), `updateVersion`, `createdAt`, `updatedAt` |
| progression | `breadcrumbs` | character id | `characterId`, `userId`, `breadcrumbs` (default `[]`), `updateVersion` |
| progression | `encounteredContent` | character id | `characterId`, `userId`, `encounteredcontent` (default `[]`) |
| bounties | `bountyStates` | uid | `userId`, `bounties` (`[]`), `draftData` (`{}`), `dailyRewardCreditedOn`?, `dailyPremiumDraftGrantedOn`?, `updateVersion`, `createdAt`, `updatedAt` |
| bounties | `cooldownStates` | uid | `userId`, `entries` (`{}`), `updateVersion`, `createdAt`, `updatedAt` |
| bounties | `entitlements` (one per account holding any, in account-id order) | uid | `userId`, `entitlements`: objects in name order with `name`, `duration`, `activatedDate`?, `grantedAt`?, `sourceSkuId`? |
| bounties | `wallets` | uid | `userId`, `bootstrapVersion`?, `balances`: an object of catalog id to amount in ordinal order |
| bounties | `economyDaily` (one per account, day, currency) | `<uid>:<day>:<currency>` | `userId`, `day`, `currency`, `ttlAt` (the first counter's in path order), then each counter's path split on `.` (empty segments dropped) into nested objects with the value as the leaf; a later path through an existing number replaces it with an object |
| parties | `parties` | client id | `partyId`, `revision`, `leaderPlayerId` uid, `buildId`, `activity`, `activeHuntSessionId`?, `createdAt`, `updatedAt`, `members` (uids in position order) |
| parties | `partyMemberships` (one per member, after all parties are built) | member uid | `partyId` (client id), `userId`, `joinedAt` |
| parties | `partyInvites` | `<recipient uid>` NUL (U+0000) `<sender uid>` | `recipientPlayerId`, `sendingPlayerId`, `partyId` (the stored party's client id, else the stored text), `partyRevision`, `sendingDisplayName`, `sendingPlatform`, `status`, `createdAt`, `expiresAt`, `consumedAt`? |
| parties | `partyOutbox` | id | `eventId`, `partyId` (the stored `party` text as is), `partyRevision`, `eventType`, `recipientAccountIds` (`[]`), `payload` (`{}`), `state`, `createdAt`, `claimedAt`?, `claimOwner`?, `publishedAt`? |
| parties | `friendships` | `<owner uid>:<other_user_id>` | `ownerId`, `otherId` (= `other_user_id`), `status`, `direction`?, `created`, `favorite` |
| reforged | `reforgedInvites` | code | `invitecode`, `usesRemaining`, `infiniteUses`, `note`?, `createdAt` |
| reforged | `adminSessions` | id | `id`, `userId`, `tokenHash`, `csrfToken`, `createdAt`, `expiresAt`, `ip`, `userAgent`, `revokedAt`?, `ttlAt` |
| reforged | `adminAudit` | id | `id`, `actorUserId`, `targetUserId`?, `action`, `oldState`? (`{}`), `newState`? (`{}`), `reason`?, `ip`, `requestId`, `createdAt` |
| reforged | `launcherNotices` (zero or one) | `current` | `version`, `updatedAt`?, `updatedBy`?, `announcements`, `tags` (`[]`) |
| reforged | `operationsPolicy` (zero or one) | `global` | `guardEnforcement`, `diagnosticsProfile`, `version`, `updatedAt`?, `updatedBy`?, `diagnosticsExpiresAt`? |
| reforged | `launcherGuardSessions` | guard session id | `guardSessionId`, `userId`, `role`, `publicKeyBase64`, `manifestId`, `manifestSequence`, `channel`, `challenge`, `lastSequence`, `status`, `riskScore`, `createdAt`, `expiresAt`, `lastHeartbeatAt`?, `detail` (`{}`), `ttlAt` |
| reforged | `guardFindings` | **none** (MongoDB assigns an ObjectId) | `fileName`, `sha256`, `signer`, `size`, `firstSeenAt`, `lastSeenAt`, `sessionCount`, `accountCount`, `accountIds` (each stored entry mapped through uid), `ttlAt` |

`?` marks a key omitted when null. Documents within a collection follow the store records' key order (§9.2).

### 11.3 What export guarantees

The tests fix one round trip: migrate a source, add a launcher account written the way the live register path writes
it, export to an empty MongoDB, migrate that into a fresh store, and verify the fresh store against the exported
MongoDB: all 37 tables match. Fidelity to Node's own document types (dates, TTL fields, legacy purposes) is not
claimed (§17 item 9).

---

## 12. Reports, output text, exit codes

### 12.1 The migrate report

`MigrateReport { dry_run, counts, skipped_invalid, rb11_launcher_approvals_backfilled,
rb11_inventory_arrays_transformed, rb11_starter_instances_repaired, rb11_loadout_slots_reconciled }`. `counts` has
**all 37** table names of §10.3 (sorted by name) with the number of extracted rows of each, 0 for unselected areas;
`accounts` includes implicit accounts; `launcher_notices` and `operations_policy` are 0 or 1; `wallet_balances` and
`party_members` are not counted. A dry run's counts equal a real run's (ids differ only in value).

### 12.2 The verify report

`VerifyReport { ok, collections }`, each `CollectionVerification { name, area, expected_count, actual_count,
expected_sha256, actual_sha256, ok, mismatches }`.

### 12.3 The export report

`ExportReport { dry_run, counts }`, `counts` keyed by collection name (sorted) for the collections produced.

### 12.4 Text, JSON and exit codes (the command-line front)

| Situation | stdout | stderr | Exit |
| --- | --- | --- | --- |
| Help | usage | | 0 |
| Parse error | | `error: <error>`, an empty line, the usage | 2 |
| migrate ok, text | `isles-db migrate (<dry-run|migrated>):`, then `  <table>: <n>` for each count above 0 in name order, then `  rb11 transforms: approvals=<n>, inventory_arrays=<n>, starter_instances=<n>, loadout_slots=<n>`, then `  skipped_invalid: <n>` only when above 0 | | 0 |
| verify ok, text | `isles-db verify: OK (<n> collections verified)`, then per table `  [OK] <name> (area=<area>, count=<actual>, sha256=<first 16 hex of the actual hash>)` | | 0 |
| export ok, text | `isles-db export-mongo (<dry-run|exported>):`, then `  <collection>: <n>` for each count above 0 | | 0 |
| `--json` | the report, `serde_json` pretty form (two-space indent), fields in the order of §12.1 to 12.3 | | 0 |
| any command failing | | `isles-db <migrate|verify|export-mongo> failed: <error>` | 1 |

In JSON, `area` is the variant name (`"Auth"`, `"Characters"`, ...), while the text form prints the lower-case area
(§17 P15). The binary's name in these texts is restricted's (`isles-db`); a front shipped by this crate prints
`rustic-isles-db` (§17 item 11).

### 12.5 Error texts (`DbError`, Display kept exactly)

| Variant | Display |
| --- | --- |
| `Mongo(MongoError)` | `mongo: <MongoError>` |
| `Store(store::Error)` | `store: <store's text>` |
| `Diesel(diesel error)` | `diesel: <text>` |
| `Config(String)` | `invalid argument: <text>` |
| `InvalidDocument { collection, detail }` | `invalid document in <collection>: <detail>` |
| `VerificationFailed(usize)` | `verification failed: <n> collection(s) did not match` |

`MongoError`: `InvalidUri` (`invalid MongoDB URI: <d>`), `Io` (`MongoDB I/O error: <e>`), `Bson` (`BSON decode
error: <d>`), `Command` (`MongoDB command error: <d>`), `Auth` (`MongoDB authentication failed: <d>`). No error text
contains a password: the URI's password never enters a message, and the store's Postgres errors are its own
(redacted `PostgresConfig`). The host kit still passes every message through its own redaction.

### 12.6 Logs

One warn line per skipped document (§7.3). Nothing else; never a document's contents, a URI or a credential.

---

## 13. Settings

None. The crate reads no environment variable and no file except the storage policy under `--game-data-dir`. Every
default (`host.toml`'s `[mongo]`, `[database]`, `[network]`, `backend.env`'s `MONGODB_URI`/`MONGODB_DB`, the backend
game-data folder) is resolved by the host kit's `reforged-host db` before it calls the library (§14.4). No
`host.toml` field, `setup` option or env line is added by this unit.

---

## 14. The seam

### 14.1 Shape

One crate, `rustic-isles-db` (lib), MIT, in dainty-lass `crates/rustic-isles-db`. Depends on dainty-lass crates
`store` (`Database`, `DatabaseConfig`, `DatabaseOptions`, `PostgresConfig`, `AnyConnection`, `Tx`, `Actor`, `Error`),
`module-api` (`NodeId`, `GlobalId`), `rustic-isles` (core `AccountId`), `rustic-isles-store` (`SCHEMA`, the slot rules
of its §7.3), `rustic-isles-characters` (`StarterKit`) and `rustic-isles-economy` (`inventory::storage_policy`:
`StoragePolicy::load`, `migrate_inventory_storage_arrays`); workspace `diesel` (with `32-column-tables` in its own
manifest, as the store decided, for `launcher_accounts`), `serde`, `serde_json` (own `preserve_order`,
`float_roundtrip`), `sha2`, `hmac`, `base64`, `percent-encoding`, `chrono`, `rand`, `thiserror`, `tracing`, `tokio`.
No `mongodb` crate: the wire client is its own (§4), which keeps RB15's "no `mongodb` in `cargo tree`" true.

**Seam changes for the integration pass**: `crates/rustic-isles-db` and `rustic-isles-db` in
`[workspace.dependencies]`; the `hmac` and `percent-encoding` workspace entries if missing. No migration, protocol or
`clap` change (the host kit's `db` tree exists already).

### 14.2 With `rustic-isles-store`

- The tool writes and reads **tables directly**, not through the repositories (their revision guards and business
  rules are wrong for a bulk copy). It declares its own Diesel table definitions for the 37 tables plus
  `wallet_balances` and `party_members`, with the names, columns and types of the store spec §4 (the `loadouts`
  column of `loadouts` is the slot list). A store migration that changes one of these tables changes this crate in the
  same row. (Alternative, a store row's call: the store exports its table declarations; then this crate uses those.)
- It opens the database with `store::Database::open` and the store's `SCHEMA`, not `IslesStore::open` (no home check:
  migration writes rows of every account, and the actor is fixed), and writes under `store::Actor::System("migrate")`.
- It reads the soft-delete rule (store §2.2) and the slot rules (store §7.3: `resolve_player_journey_total`,
  `resolve_requested_total`, `DEFAULT_ACCOUNT_LOADOUT_SLOTS`, `MAX_TOTAL_LOADOUT_SLOTS`) from the store.

### 14.3 Inputs the caller supplies (game-derived values)

`LegacyStarter` (public, `Clone`, `Debug`), built by the caller:

| Field | From | Used by |
| --- | --- | --- |
| `instanced` | the characters `StarterKit`'s `instanced` (characters spec §8.2 `instanced_catalog_ids`) | §8.3 step 1 |
| `equipment` | the kit's `equipment` pairs, then (`flask`, the kit's `flask`) | §8.3 step 3 |
| `banner`, `glider` | the kit's `banner`, `glider` | §8.3 steps 2 and 4 |
| `default_dye`, `default_sigil` | restricted's version values (two catalog ids the kit does not carry) | §8.3 step 4 |

A constructor from a `StarterKit` plus the two defaults is offered. Tests use synthetic values.

### 14.4 Interface (names are the contract restricted's host wiring is written against)

| Item | Contract |
| --- | --- |
| `pub enum Area { Auth, Characters, Inventory, Progression, Bounties, Parties, Reforged }` | `ALL`, `as_str()`, `parse(&str) -> Result<Area, DbError>` (§3.3); serializes as its variant name (§12.4) |
| `pub struct MigrateOptions { from_uri, to_target, node, dry_run, areas, skip_invalid, game_data_dir, allow_remote_postgres, starter }` | `effective_areas()`; `Debug` redacts the password in `from_uri` and `to_target`; equality compares every field but `starter` |
| `pub struct ExportOptions { from_source, to_uri, areas, dry_run, allow_remote_postgres }` | likewise |
| `pub enum Command { Migrate { opts, json }, Verify { opts, json }, ExportMongo { opts, json }, Help }` and `pub fn parse_args(args, starter) -> Result<Command, DbError>` | §3; `replay` answers a typed "not this crate" so the front can route it to unit 10 |
| `pub async fn run_migrate(&MigrateOptions) -> Result<MigrateReport, DbError>`, `run_verify(..) -> Result<VerifyReport, DbError>`, `run_export_mongo(&ExportOptions) -> Result<ExportReport, DbError>` | §7 to §11 |
| `pub fn print_report(..)` per command, and `pub fn main_with(args, starter) -> ExitCode` | §12.4, for a front (restricted's `isles-db` binary until it goes) |
| `MigrateReport`, `VerifyReport`, `CollectionVerification`, `ExportReport` (`Serialize`, `Deserialize`) | §12 |
| `pub enum DbError`, `pub mod mongo::{MongoUri, MongoError, MongoClient (connect, database, find_all, collection_has_documents), MongoWriter (replace_collection), decode_document, encode_document}` | §4, §12.5; `MongoWriter` exists only with the `export-mongo` feature |
| `pub fn state_to_mongo_collections(&TargetState, &BTreeSet<Area>)` | §11.2 (feature `export-mongo`) |
| feature `export-mongo` (default on) | §11; RB15 turns it off and deletes it |
| feature `test-support` | `MockMongoServer` (§16.1) |

**Restricted's host wiring** (`crates/dainty-host`'s `run_db_command` and the `db_migrate`/`db_verify`/`db_export_mongo`
hooks of dainty-lass's `hostkit::hooks`) builds the options from `host.toml` and calls `run_*`; it switches from
`isles_db` to this crate and passes the 1.14.7 `LegacyStarter`. That file is restricted's; this crate does not change
`hostkit`.

---

## 15. Security behaviour (for `docs/security/rustic-isles-db.md` and RX3)

The tool parses untrusted input on two fronts: **the MongoDB server's replies** (network bytes) and **the documents**
(a host's own database, but filled by players: names, JSON blobs, arbitrary keys). It is a trust-boundary component
for RX3.

- **Read-only source by construction.** migrate and verify can send only `hello`, `saslStart`, `saslContinue`,
  `find`, `getMore`. The write commands live in a separate type that only export builds (feature `export-mongo`).
- **Bounded parsing.** Reply length 21 bytes to 64 MiB; every length field checked against the bytes left before use;
  BSON nesting capped (§4.4, departure); no panic on any byte sequence; arithmetic on lengths never overflows (a
  negative int32 length is refused, not cast).
- **Memory**: a collection is held whole in memory (restricted). Acceptable for one host's data; noted.
- **Authentication**: SCRAM-SHA-256 with the password never logged; the rewrite verifies the server signature and caps
  iterations (§4.6, departures). The MongoDB connection is plaintext; the tool does not refuse a non-loopback MongoDB
  host (unlike Postgres). Proposed: refuse it unless `--allow-remote-mongo` (§17 item 6).
- **Secrets never in `Debug`** (decision): the URI's password, the options' URIs, and every secret column of the
  canonical records (§10.4) are redacted. Error texts never include a password (§12.5). The one log line names a
  collection and a fixed detail.
- **SQL**: Diesel's bound parameters only; no text is spliced.
- **Destructive paths**: export deletes whole collections; opening a missing SQLite source creates an empty database
  (§17 item 1). Proposed for the rewrite: export and verify refuse a SQLite path that does not exist
  (`the database <path> does not exist`) before opening.
- **Encoding**: a key with NUL is refused before sending (§4.5, departure).
- **Fuzz targets** (`fuzz/fuzz_targets/`):
  - `rustic_isles_db_bson.rs`: arbitrary bytes into the BSON decoder never panic or overflow the stack, and answer a
    document or a decode error; any decoded document re-encodes and decodes to an equal value modulo number widths.
  - `rustic_isles_db_reply.rs`: arbitrary bytes as a server reply to a `find` (an in-memory stream) never panic,
    never allocate more than the declared length, and answer documents or an error.
  - `rustic_isles_db_uri.rs`: arbitrary text into the MongoDB URI and Postgres URL parsers never panics, and a parsed
    URI's `Debug` never contains its password.
  - `rustic_isles_db_extract.rs`: arbitrary JSON documents (from the fuzz bytes) as every collection of a fake source,
    through extraction, the RB11 repairs and the export builder, never panic; every JSON text column parses; every
    loadout's `unlocked_total_slots` is at least 1; every repaired item id is 26 characters of `A`-`Z0`-`9`.
  - `rustic_isles_db_scram.rs`: arbitrary server-first and server-final payloads never panic and are refused unless
    well-formed (iterations within the cap).

---

## 16. Test cases

### 16.1 What may be copied

Nothing recorded exists for this unit; restricted's sample collections are authored test data and are described, not
copied. The tests use a **mock MongoDB server** this crate provides behind `test-support`: on `127.0.0.1:0`, OP_MSG
only, one thread per connection; `hello`/`ismaster`/`isMaster` answer `isWritablePrimary` true, `maxWireVersion` 21,
`ok` 1; `find` honours `limit` (> 0), `singleBatch` and `batchSize` capped by the server's `max_batch` (at least 1),
cursors per connection; `getMore` continues a cursor (`cursor not found` when unknown); `delete` drops the whole
collection; `insert` appends; anything else `ok` 0 `unsupported command`; `ns` is `dauntless.<collection>`; a
`snapshot()` of the collections. No authentication (a URI with credentials fails against it). Tests run on SQLite (a
temporary file) and, when `REFORGED_TEST_POSTGRES_URL` is set, also on Postgres (store spec §15.1's convention);
`REFORGED_TEST_MONGO_URI` points the rehearsal at a real MongoDB and `REFORGED_TEST_SEED_MONGO=1` lets it seed that
database first (opt-in: it writes to it). Unset, those halves print that they are skipped and pass.

### 16.2 Behaviour the authored tests fix

| # | Area | Case |
| --- | --- | --- |
| D1 | CLI | `migrate --from M --to T --node eu-west --area auth,characters --area bounties --dry-run --skip-invalid --game-data-dir G --allow-remote-postgres --json` parses to those values (areas auth, characters, bounties); `verify --against M --to T` sets the source; `export-mongo --from sqlite.db --to M --area auth --area characters --dry-run --allow-remote-postgres --json` likewise; `--help` and no arguments are Help; `unknown` is an error; `--area nope` names the valid areas; `migrate --from M` alone needs `--to`, with `--dry-run` it does not |
| D2 | URI | `mongodb://myuser:p%40ss@127.0.0.1:27017,127.0.0.2:27018/dauntless?authSource=admin` gives two hosts, database `dauntless`, user `myuser`, password `p@ss`, auth source `admin`; no path gives `dauntless`; `user@host` is a credentials error; `mongodb+srv://` refused; `Debug` has no `p@ss` |
| D3 | BSON | A document of a string, an int32 42, an int64 9 000 000 000, a double 3.5, `true` and null encodes and decodes to the same JSON; truncated and over-long inputs are errors, never panics; a datetime decodes to its ISO text; an ObjectId to 24 hex digits |
| D4 | Readers | Time: `"2026-03-01T10:00:00Z"` → `2026-03-01T10:00:00.000Z`; `1700000000` (number) → seconds → `2023-11-14T22:13:20.000Z`; `1700000000000` → same instant; `"1700000000000"` → same; `"yesterday"` kept; `{"$date": "2026-03-01T10:00:00Z"}` → normalized. Integer: `"12"` 12, `"1.5"` default, `2.9` 2, `true` 1. Boolean: `"TRUE"` true, `"no"` false even with default true, `null` default |
| D5 | Ids | §6.1's vectors; an existing target account with `user_id` `u1` keeps its id on a second run |
| D6 | Migrate | The mock server with `max_batch` 1 (every collection paged through `getMore`): a dry run reports one approval backfilled and does not create the SQLite file; the real run counts 2 accounts, 2 launcher accounts, 1 character, 1 loadout, 1 inventory, 1 party, 1 Guard finding; the change log is non-empty, its chain verifies, and its entries are by `system:migrate` |
| D7 | Read back | Through `rustic-isles-store`'s repositories: the account's `notes`; a launcher account without `approvalStatus` reads `approved` by `migration:0.1.73`, one with `pending` stays `pending`; identity, refresh session, email code attempts, game server and user keys found |
| D8 | RB11 | A journey with `LoadoutSlot_01` and `_02` at status 2 and a loadout of one slot with active index 99: three slots, `unlocked_total_slots` 3, active index 2; a starter weapon with `instance_id` `bad-id` gets a valid 26-character id; with a storage policy whose stacked entry is in `instancedItems`, it moves to `stackedItems` |
| D9 | Areas | Bounty, cooldown, entitlement, wallet (balances read back by catalog id), tracks, objectives, grant, escalation, journey, breadcrumbs and encountered content, party (two members), friend (favorite), invite code, notices version, policy `OBSERVE`, Guard session and finding all read back |
| D10 | Verify | 37 tables ok after migrate; a second migrate gives the same counts and verify still passes; one row changed in the target makes verify fail with `VerificationFailed(1)` |
| D11 | Export | A dry run counts and leaves the destination empty; a real export of 3 accounts (one added directly as the register path writes it) and 1 character writes documents with the `_id`s of §11.2 (`loadouts` `_id` is the character id); migrating that export into a fresh store and verifying it against the export passes for all 37 tables |
| D12 | Rehearsal | With the env of §16.1, the same source migrated into SQLite and Postgres gives equal counts and both verify |
| D13 | Probe | `collection_has_documents`: a collection with documents true, an empty one false, a missing one false |
| D14 | Invalid | (new) A `characters` document without `userId`: the run fails with `invalid document in characters: missing userId`; with `--skip-invalid` it is skipped, `skipped_invalid` 1, and the rest migrates |
| D15 | Safety | (new, for the proposed departures) a reply nested 10 000 levels deep is a decode error, not a crash; a SCRAM `i=` above the cap is refused; a SCRAM final without a matching `v=` is refused; `export-mongo --from <missing file>` is refused before any MongoDB connection |

---

## 17. Open questions, and behaviour that looks wrong but is kept

### 17.1 Questions for the coordinator

1. **Opening creates and migrates.** verify and export-mongo open their store through `Database::open`, which
   creates a missing SQLite file and migrates any database it opens. For export that means a mistyped `--from`
   empties every MongoDB collection of the selected areas. Proposed: refuse a missing SQLite path in both (a
   departure); migrating an existing older database on open stays.
2. **The banner and glider repair reshapes 1.14.7 data** (§8.3 step 4): a `persistent.banner`/`glider` that is a
   string (the 1.14.7 shape) becomes the legacy object. Parity specified; the maintainer should check a real 1.14.7
   MongoDB before running migrate, or decide to skip the step when the value is a string.
3. **Implicit accounts overwrite names** (§9.2): a run with areas that exclude `auth` against a store that has the
   accounts resets their `name` to the user id and `notes` to 0. Proposed fix: implicit account rows insert only.
   Parity until decided.
4. **verify prints nothing useful on failure** (§10.1): the per-table report is computed and dropped. Proposed: print
   it (text: `[DIFF]` lines with the mismatches; JSON: the report), then exit 1. Parity until decided; restricted's
   host wiring has an unreachable branch that expects this.
5. **SCRAM**: verify `v=`, cap iterations (§4.6). Proposed: both, as security departures.
6. **MongoDB off this machine** is accepted in plaintext. Proposed: refuse unless `--allow-remote-mongo`, as Postgres
   does. A new flag; a `reforged-host db` change in restricted's wiring.
7. **BSON depth cap** (§4.4). Proposed: 200.
8. **NUL in keys on export** (§4.5). Proposed: refuse before sending.
9. **Export fidelity**: times are exported as strings and `ttlAt` as numbers (MongoDB TTL indexes need dates), email
   purposes keep the store's names, `persistent` keeps whatever §8.3 made of it. Whether Node 1.14.7 reads these back
   correctly is unverified (no canary). A maintainer check on a rollback rehearsal; parity until then.
10. **Characters-only export replaces `playerJourney`** for the whole database. Parity; worth a warning line or a
    refusal.
11. **Library or binary.** Proposed: library plus `main_with` (§14.4); no binary here (it would need the 1.14.7
    starter values, which are restricted's). Texts keep `isles-db` while restricted's binary is the front.
12. **`migrate_reward_cache_currency.ts`** exists in Node's scripts with no counterpart. Is it obsolete for 1.14.7, or
    owed? Not ported here unless decided.
13. **RB15 and Mongo.** `export-mongo` is slated for deletion by RB15 (one release after the cutover). Proposed: port
    it behind the default-on `export-mongo` feature so RB15 deletes one module and a feature. migrate and verify are
    the import path for a host still on Node; RB15's "the host kit's MongoDB handling" may remove them too. Proposed:
    port them now; whether they outlive RB15 is the maintainer's call.
14. **Test env names**: restricted's `ISLES_TEST_MONGO_URI`, `ISLES_TEST_SEED_MONGO`, `ISLES_TEST_POSTGRES_URL`
    become `REFORGED_TEST_*` (§16.1), matching the store's tests.
15. **The MIT host kit's fallback** (`hostkit` `commands/db.rs`, not this unit) resolves export's `--from` as a MongoDB
    URI and `--to` as a folder and calls the hook with them; restricted's hook then treats the folder as the store.
    The fallback runs only when no wiring is registered; the integration pass may want it aligned or removed.

### 17.2 Parity list (odd but kept)

| # | Behaviour |
| --- | --- |
| P1 | Text-or-none treats whitespace-only text as present; numbers become text (`userId: 123` is user `123`) |
| P2 | An empty (after trim) user id aborts the run even with `--skip-invalid` |
| P3 | `--dry-run` is accepted and ignored by verify; export's `--node` is validated and ignored |
| P4 | Updates hit soft-deleted rows and leave them deleted; verify then reports them missing |
| P5 | The RB11 counters count documents visited, not repairs (except approvals) |
| P6 | Characters-only runs point loadouts at items seeded into inventories that are not written; a loadout with no inventory gets banner and glider ids that exist nowhere |
| P7 | New sequence numbers follow key order, not the source's order; character `ord` defaults to a counter across all accounts |
| P8 | `party_outbox.party` and `recipients` keep Node's ids; `admin_audit` keeps Node user ids; a grant's `result.userId` becomes the account id text |
| P9 | Export writes a missing account's global id text where a user id belongs |
| P10 | A storage policy that fails to load is skipped silently |
| P11 | Postgres URLs are not percent-decoded (a password with `@`, `:` or `/` must not be URL-encoded, and cannot contain `/` or `?`) |
| P12 | Verify's joined tuple keys can collide |
| P13 | Float text such as `inf` and `NaN` is accepted for `duration` and `riskScore` |
| P14 | Email purposes are mapped on import and not mapped back on export |
| P15 | JSON reports name areas by variant (`Auth`), text by `as_str` (`auth`) |
| P16 | `to_json_text` keeps a JSON string's own spacing; a string field holding plain text becomes the default |
| P17 | First-document-only for notices and policy; natural order decides which |
| P18 | No `hello` fallback: MongoDB servers that predate `hello` are refused |
| P19 | Field alternatives stop at the first key present: a present `null` or `""` hides the later keys (§2) |

---

## Coordinator decisions (2026-11-05, before implementation)

These settle §17.1 and override anything above that disagrees. Departures from restricted taken here guard against
data loss or a crash; each is a candidate decision row for the maintainer to ratify.

- **Item 1: taken.** Export and verify refuse a SQLite path that does not exist (no create, no migrate).
- **Item 3: taken.** The placeholder account rows a run without `auth` writes are insert-only.
- **Item 4: taken.** A failed verify prints its per-table report, then exits 1.
- **Item 5: taken.** SCRAM checks the server signature (`v=`) and refuses an iteration count above 1,000,000.
- **Item 6:** parity (no new flag here; the argument tree is hostkit's); a candidate row.
- **Item 7: taken.** BSON nesting is capped at 200 levels; a NUL in a key is refused on export.
- **Items 2, 8, 9:** parity, flagged for the maintainer to check against a real 1.14.7 MongoDB and the Node backend.
- **Items 10, 12, 13:** as proposed (library plus `main_with`; export-mongo behind a default-on feature RB15 removes;
  `REFORGED_TEST_*`).
- **Item 11:** a question for the maintainer; not ported.
- **Item 14:** the integration pass's.
- **Table declarations:** the crate keeps its own (from store §4), as written.
- **§6.1 vectors:** the implementer checks them against its own reading of the formulas and reports any disagreement;
  a comparison run in restricted confirms them later.
- **§17.2:** parity.
