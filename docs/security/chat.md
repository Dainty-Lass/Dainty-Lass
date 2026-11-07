<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the chat plugin (RX3)

The checklist for the chat core plugin, `crates/dainty-chat` (Dainty Lass plan DD21, DN10a). It covers one node;
federation between nodes is DN10b's and goes in `network.md` (DX2).

How to use it: before chat is turned on for players, copy this list into the change that turns it on, check each
item in the code (not from memory), link the code or test that shows it, and sign at the end with the date and the
commit reviewed. An item that does not hold is a bug to fix first, not a box to leave open.

## What it is and where the boundary is

The 1.14.7 and 1.4.4 clients chat over XMPP on a WebSocket. Players send it free text, nicknames and room names,
and log in with the game's own token. Whoever gets chat to accept a login without the right token speaks as that
player; whoever gets it to misroute a stanza reads others' whispers or rooms.

- **In:** WebSocket upgrades on `127.0.0.1:61006` (`Settings::port`), forwarded by the gateway from :443 with
  `X-Reforged-Gateway: <GATEWAY_SECRET>` and `X-Forwarded-For`. Paths `//`, `/`, `/__ws/xmpp` only.
- **Out, to the host:** `ChatHost::login` (the game module checks its token), `ChatHost::ask` (the module's JID and
  room rules over the module protocol), `ChatHost::blocked`.
- **Out, to storage:** chat's own database on `store` (`chat.db` beside the game's on SQLite), every write through
  `Database::write` and the change log.
- **Out, to other nodes:** `Federation::publish` and `resolve`; nothing with `LocalOnly`.

Code: `connection.rs` (listener, upgrade, gateway check), `server.rs` (login, bind, IQs, presence, whispers,
limits), `muc.rs` (rooms), `naming.rs` (room names), `nick.rs`, `xmpp.rs` (the frame parser, escaping, redaction),
`history.rs`, `hooks.rs`.

Fuzz target: `chat_stanza` in `fuzz` (the frame parser, JIDs, room names, authcids and nicknames). It checks
that nothing panics, that a parsed tree holds only what the input held, and that a room name maps to an instance
room only through a JID-safe id.

## Checklist

### Who can connect

- [ ] The listener binds 127.0.0.1 or ::1 only; any other address refuses to start (`Settings::problems`; test
      `chat_stays_on_loopback`).
- [ ] The player's address is `X-Forwarded-For` only from a loopback peer that presents the gateway secret,
      compared in constant time; otherwise it is the peer (`connection::client_address`; test
      `only_the_gateway_names_the_player`). Per-address limits and the SASL throttle use that address.
- [ ] An upgrade on any other path is a 404; a non-upgrade request is a 404 (`connection::upgrade`; test
      `players_log_in_join_a_room_and_talk`).
- [ ] 512 sockets, 8 connections per address that have not bound, 32 KiB frames (1009 on more), 4 frames before
      login, 15 s to log in and 10 s to bind (`server.rs` constants, DR's).

### Login

- [ ] SASL PLAIN only, one attempt per connection; authzid empty or equal to authcid; the authcid a JID local part
      (`is_authcid_shape`); the token never logged.
- [ ] The token is checked by the game module it belongs to (`ChatHost::login`); the JID's local part must be the
      module's for that account, so a valid token can't log in under another name (test
      `a_module_gives_the_jid_and_the_rules`, `impostor`).
- [ ] A module that can't be asked refuses with `temporary-auth-failure`, never a default identity; only
      `unsupported` gives the default (`hooks::is_unsupported`).
- [ ] 10 failed logins per address in 10 minutes throttle it; replacement, abuse, oversize and backlog hold the
      account's next login back 60 s.

### Rooms

- [ ] A room is a `RoomKind` only through `naming::room_kind_of`; Guild and unknown rooms are refused; instance
      ids are `<ULID>.<node>` or a UUID (tests `rooms_are_named_as_the_clients_name_them`).
- [ ] Who may join and speak is the module's (`ChatQuery::Room`), asked at the join and every 60 s; a module that
      can't be asked refuses the join; someone no longer admitted is removed with status 307.
- [ ] Nicknames must be the player's own `name:id:resource` (`nick.rs`), another account's id refused even with
      `NickCheck::Log`.
- [ ] Two sessions of one account never see each other; a ghost is replaced.
- [ ] Room messages skip recipients who blocked the sender (`ChatHost::blocked`).

### Whispers and history

- [ ] A whisper goes only to the sessions of the account the JID names (`account_named`); the sender's JID is its
      own session's, never the stanza's `from`.
- [ ] Either player having blocked the other drops it; a muted sender's whisper is dropped; nothing comes back to
      the sender either way (DR).
- [ ] A waiting whisper is delivered once, to the recipient's next bound session, and marked delivered (test
      `a_whisper_to_someone_offline_arrives_at_their_next_login`).
- [ ] Room history is stored only for rooms whose rules say so, and sent only to a player the rules admitted.

### Mutes

- [ ] A mute is stored before live sessions follow it, and read at every login (test
      `a_muted_player_is_not_heard`); an expired mute no longer holds (`Mute::holds`).
- [ ] Only the plugin's handle sets mutes (`Running::mute`), with the admin's actor in the change log.

### Logs

- [ ] No token, SASL payload or message body reaches a log; `trace` frames go through `redact_frame` (DR's tests
      `the_trace_is_redacted`, `credentials_never_reach_the_log`).

### The staff dashboard (`moderation.rs`)

Out, when `dashboard_url` is set: every room line and whisper with its sender (original text), and `/report`s, to
`CHAT_DASHBOARD_URL`; in, the chat filter from it. The dashboard is trusted with chat's content, not with chat.

- [ ] `CHAT_DASHBOARD_INGEST_KEY` (`secrets.env`'s `DASHBOARD_INGEST_KEY`) goes only in the `Authorization` header to
      that URL, and `DashboardSettings`' `Debug` leaves it out (test `settings_need_a_url_and_a_long_key`).
- [ ] Use `https://` for a dashboard that is not on loopback or the tailnet: lines are sent in clear over `http://`.
- [ ] Delivery never waits on the dashboard: rating reads the last good filter, and lines go through a bounded queue
      that drops when full (`Moderation::record`). A dashboard that is down costs warnings, not chat.
- [ ] A filter term is compiled by the `regex` crate (linear time, `size_limit` 1 MiB), so a hostile pattern
      cannot stall chat's task; one that does not compile is skipped.
- [ ] A blocked room line goes back to its sender alone and is neither stored nor federated; a blocked whisper goes
      nowhere (test `the_dashboards_filter_applies_and_chat_and_reports_reach_it`).
- [ ] `/report` is one per 30 s per player and is never delivered to anyone else.

## Open points

- **The gateway route.** The gateway's `ws` upstream must point at the plugin's port for the 1.14.7 game and keep
  sending the secret; DF4a owns that change.
- **Postgres schema.** On Postgres chat shares the game's database until `store` takes a schema, and every write
  there takes the same advisory lock as the game's, so DD15's separation holds on SQLite only.
- **The login seam.** `ChatQuery` has no login query; the controller has to give `ChatHost::login` the module's
  token check until `module-api` grows one.
- **Retention.** History and delivered whispers are kept forever; the change log doubles them.

## Fuzzing

From `fuzz`:

```bash
cargo +nightly fuzz run chat_stanza -- -max_total_time=86400
```

## Changes from DR's metagame chat

- Logins are checked by the game module, not the metagame's token keys; the identity is the module's JID.
- Rooms are DD21's kinds, with the module's rules; whispers to someone offline are stored and delivered later.
- Friends' online status is not carried.
- Only the clients' WebSocket paths are accepted.

Signed: ________ (date, commit)
