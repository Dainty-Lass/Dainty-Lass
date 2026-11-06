<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: Rustic Isles chat (RX3)

The checklist for `crates/rustic-isles-chat`, the MIT chat rules and chat area routes of Rustic Isles (RD27,
`docs/port/spec-rustic-isles-chat.md` §12). It opens no listener of its own: its routes run behind core's pipeline on
the backend, and its rules answer the chat plugin's queries, which carry what a game client wrote (a SASL authcid and
token, room names, JID local parts).

How to use it: before a change to this crate merges, and before its cutover is signed (RD10), copy this list into the
change, check each item in the code (not from memory), link the code or test that shows it, and sign at the end with the
date and the commit reviewed.

## What it is and where the boundary is

- **In, from the chat plugin** (`module_api::chat::ChatQuery`): the login's authcid and token, room local parts
  (`RoomOf`), JID local parts (`AccountOf`), and account ids it already bound to a session.
- **In, from players over the backend**: the friends routes' path params and query, and the Slayer link JSON body.
- **In, from other nodes**: the `chat.*` home operations (`HomeInvocation`, run only for accounts homed here; the
  network plugin authenticated the link).
- **Out**: the store (`IslesStore` reads and home-checked account writes), the home API (`forward`,
  `verify_session_ticket`), and auth's game token check.

Code: `crates/rustic-isles-chat/src/` (`login.rs`, `rules.rs`, `source.rs`, `ids.rs`, `hubs.rs`, `headline.rs`,
`home.rs`, `routes.rs`).

Fuzz target in `fuzz`: `rustic_isles_chat_names`: any bytes (lossy UTF-8) as a `RoomOf` name, a party or hunt id, an
`includePending` query and a JID local part never panic; an id longer than 300 bytes before `_` is never read; every
id read writes back the room spelling it came from; `slayer_link_availability` over any JSON answers at most 256
entries of 1 to 128 UTF-16 units; the headline stanza never holds an unescaped `<`, `>`, `&` or quote inside an
attribute or the body.

## Checklist

### Identity is the token's

- [ ] The login binds the player from a valid **game** token (`Auth::validate_game_token`: RS256, `iss`/`aud`
  `paradox-backend`, not expired), so a launcher or store token is refused by audience (`login.rs` `authenticate`;
  test `login_over_real_tokens_and_a_store`).
- [ ] The token's `userId` must equal the authcid byte for byte; the JID, nickname and room name are never read for
  identity (test `login_rules`: both mismatch directions).
- [ ] The client-credentials userId (`__launcher_client_credentials__`) is never a player.
- [ ] An ineligible launcher account (banned, disabled, pending, rejected, timed out, no username) cannot sign in:
  `player_by_user_id` applies `is_account_eligible` at core's replay-aware now (test: the banned account).
- [ ] A store failure during the login is `Unavailable` (`temporary-auth-failure`), never an open door.
- [ ] The cross-node login answers a player only when the home verified a session ticket for this module's game
  version, or the account's own home answered `chat.login` for that token; every error there is "no player".
- [ ] The `chat.login` home operation refuses a valid token for another account (`token account does not match target
  account`; test `home_operations`).

### Room authorization is by account

- [ ] Party rooms: open only when the account's stored party, read back through the id reader, is the room's party
  (`rules.rs` `room`; test `party_rooms`: member, non-member, other party, failing lookup).
- [ ] Region rooms: open only for the account's live hub session in that region (7 days, inclusive), or the dev
  switch `REALTIME_XMPP_DEV_CITY_MUC`, which is on only for the exact value `true` (test `settings_table`).
- [ ] Hunt rooms: the dev switch only (default on, see weak spots).
- [ ] The network room is refused; history is never kept (`history: false` on every answer).
- [ ] A store failure is an error answer (`Unavailable`), so the plugin refuses the join rather than opening it.

### Friends routes act only for the caller

- [ ] Every friends route ignores `:accountId`: the owner is game auth's caller (`routes.rs`; test
  `friends_invite_accept_remove` reads bob's path as alice and gets alice's list).
- [ ] A caller with no userId writes nothing (`[]`, 204).
- [ ] An invite needs both userIds to be local game accounts: no edge for a stranger (DD18).
- [ ] Each edge write is an `account_write` on its owner's home with the caller as actor, so a write for an account
  homed elsewhere is `NotHome` (503), never a local write of another node's data (test
  `a_friend_homed_elsewhere_is_503`).

### Secrets

- [ ] The token is never logged and never put in an error text; refusal texts are the six fixed reasons.
- [ ] `StoreChat`'s `Debug` prints the node, the module id, whether a database is open and the two switches only.
- [ ] Nothing here formats a `ChatQuery` with `{:?}`: module-api derives `Debug` for `ChatQuery::Login` with the
  token in it (a module-api follow-up).
- [ ] The routes log userIds and counts only.

### Inputs are bounded

- [ ] Ids are read from at most 300 bytes before `_`; a ULID must be canonical upper case and a node a valid node
  name; a UUID is exactly 8-4-4-4-12 hex digits (`ids.rs`; unit tests and the fuzz target).
- [ ] `includePending` is parsed in one linear pass (`include_pending`).
- [ ] The Slayer link body is read with a 100 KiB limit (413 over it) before auth, as Express did; the list is capped
  at 256 entries of at most 128 UTF-16 units, counted with a bounded iterator (test `slayer_link_availability`).
- [ ] The hub registry is capped at 50,000 accounts, the oldest recorded going first (test `the_oldest_go_first`).
- [ ] The headline frame escapes `&`, `<`, `>`, `"` and `'` in the userId, the event id and the body.

### Known weak spots (kept for parity, spec §12 and §13)

- Hunt rooms are open to any signed-in player who names one (the switch defaults on, as Node's); the id has 80
  random bits.
- The hub registry is memory only: a restart shuts every region room until players are sent to a hub again.
- Friend invites to any existing userId are limited only by the request budget.
- An invite's two edge writes are two transactions: a failure of the second leaves a one-sided edge (test
  `a_friend_homed_elsewhere_is_503` shows it).
- The Slayer link body reader is restricted's (coordinator decision): no charset or content-encoding check, and a BOM
  is malformed JSON.
- Nobody delivers party headlines yet: the chat plugin has no subscriber for `isles.party.headline`.

## Sign-off

Reviewed by: ______ on ______ at commit ______.
