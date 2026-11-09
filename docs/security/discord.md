<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the Discord bot (RX3)

The checklist for the Discord bot, `crates/dainty-discord`, composed into the server by
`controller::modules::discord` (`docs/module-sharing-plan.md`, A7, row MS7). `SECURITY_REVIEW.md` indexes it.

How to use it: before the bot is turned on for a public server, copy this list into the review, check each item in
the code (not from memory), link the code or test that shows it, and sign at the end with the date and the commit
reviewed. An item that does not hold is a bug to fix first, not a box to leave open.

## What it is and where the boundary is

The bot hands out ways into the game: `/key` replies with dainty-dash's signup link when signups are on, otherwise
with a single-use login URI holding a fresh invite code, and `/status` says whether the game is up. It is the first
component that turns a message from outside the server's own players into an invite, so what reaches it from Discord
is untrusted, and what it sends back is a credential.

- **In:** Discord's gateway, a WebSocket over TLS the bot dials out to through a twilight `Shard` (webpki roots on
  aws-lc-rs; twilight heartbeats, resumes and reconnects). Nothing listens: the bot uses no HTTP interactions
  endpoint, so the gateway stays the server's only listener off loopback. Interactions arrive authenticated by that
  TLS connection, so there is no Ed25519 signature to check as an HTTP endpoint would.
- **Out:** Discord's REST API through `twilight_http::Client` (https only, webpki roots, its rate limiter) to
  register the commands and answer interactions; dainty-dash's public `GET /api/public/signup` (A8, D3), http or
  https as `[discord] dashboard_url` (or host.toml's `dashboard_url`) says.
- **Inside the process:** `GameCall::Availability` and `GameCall::CreateInvite` to the 1.14.7 module through the
  registry, and one `discord.key.requested` event per `/key` on the registry's bus, which the recorder keeps (A5,
  D6).
- **Files:** `discord.env`, which `dainty-host setup` writes from `[discord]` and secrets.env's `DISCORD_BOT_TOKEN`.

Code: `crates/dainty-discord/src/` (`interaction.rs`, `bot.rs`, `discord.rs`, `signup.rs`, `config.rs`, `catalog.rs`,
`http.rs`), `crates/dainty-controller/src/modules.rs` (`discord`, `DiscordAccess`), and the host kit's
`crates/dainty-hostkit/src/hostconfig/discord.rs`, `hostconfig.rs` (`PROVIDED_SECRET_KEYS`, `set_provided_secret`,
`build_env_files`), `commands/setup.rs` and `serve.rs` (`compose`, `discord_invite`).

Fuzz target in `fuzz`:

- `discord_interaction`: an interaction's body (read into twilight's model, then `Interaction::from_model`) and a
  dashboard answer of any bytes parse or are refused, never panic; an interaction that parses has ids that are not 0
  and a token that is `[A-Za-z0-9._-]`, at most 512 bytes and absent from `Debug`; a signup link is an http(s) URL
  without credentials.

## Checklist

### The token

- [ ] `DISCORD_BOT_TOKEN` lives only in secrets.env (`SECRET_KEYS`) and reaches only `discord.env`
      (`build_env_files`; `discord_round_trips_and_its_token_reaches_only_its_env_file`). It is never a host.toml
      field: `[discord]` refuses any key but `enabled`, `application_id`, `guild_ids`, `dashboard_url`.
- [ ] `setup --discord-bot-token` reads the token from stdin, never from the command line, and checks it before
      anything is written (`commands/setup.rs`). It is listed in `PROVIDED_SECRET_KEYS`, so an empty one is never
      made up or counted missing (`the_discord_token_is_set_not_made`).
- [ ] The `[env]` passthrough refuses it, and an override from the host's shell is logged with the value redacted
      (`env::denied`, `overrides::sensitive`).
- [ ] `BotToken`'s `Debug` prints a placeholder; the token goes only into the `Authorization` header and Identify
      and Resume (`config.rs`, `discord.rs`). A REST error names the call and twilight's error, whose text never
      holds the request, its path (which holds an interaction token) or its headers (`Rest`).
- [ ] A child module never gets it (D5): it is a built-in's env file, and `[[modules.external]]` still gets only
      `system_environment`.

### What Discord sends

- [ ] Gateway messages are read by twilight (`Shard`, `EventTypeFlags::READY | INTERACTION_CREATE`); one it can't
      read is logged by error and dropped.
- [ ] An interaction is taken from twilight's model (`Interaction::from_model`): ids are not 0, the command name
      is at most 128 bytes, the locale picks a catalog and is used for nothing else, the token is
      `[A-Za-z0-9._-]{1,512}` so it can't change the REST path it goes into. One that doesn't parse is logged by
      reason, never by content (`ParseError`).
- [ ] An interaction for another application is ignored (`Gateway::interaction`); READY's application must match
      `[discord] application_id` when that is set, else the bot stops (`Gateway::ready`).
- [ ] Commands are answered only in a guild, and with `guild_ids` set only in those guilds; a direct message gets
      `command.wrong_guild` and nothing else (`Bot::answer`, `answers_only_in_its_guilds`). Global commands are
      registered for guild channels only (`InteractionContextType::Guild`).
- [ ] At most 32 interactions are answered at once; more are left for Discord to time out (`MAX_IN_FLIGHT`).
- [ ] A close code twilight may not reconnect from (4004, a bad token; 4010 to 4014) stops the bot with its reason
      (`discord::fatal`); twilight reconnects from anything else, resuming where it can.

### `/key`

- [ ] Every reply is ephemeral: the interaction is acknowledged with a deferred response flagged `EPHEMERAL`, and the
      reply edits that message (`Rest::defer`, `Rest::reply`), so only the member who asked sees a link or a URI.
- [ ] The REST client's default `allowed_mentions` is empty, so nothing the bot writes pings anyone (`Rest::new`).
- [ ] Availability first, through the registry (`GameCall::Availability`): a game that is down gets the module's
      reason, translated, and no invite (`key_and_status_say_why_the_game_is_down`).
- [ ] With signups on, the dashboard's link and no invite (`key_sends_the_signup_link_when_signups_are_on`). The link
      must be an http(s) URL without credentials, at most 512 bytes, and is passed on as parsed
      (`signup::parse_answer`). Any failure, a non-2xx answer, more than 4 KiB or 5 s reads as off; the answer is kept
      60 s, a failure too, so a member can't make the bot hammer the dashboard.
- [ ] Otherwise `CreateInvite { uses: 1, expires_in_secs: 1800, require_up: true }`: one use, expiring in the game
      module's `expires_at` (D4), and the module rechecks the hub inside the call (A3). An `Unavailable` answer
      replies as a down game.
- [ ] One live code per member: a repeat with more than 5 minutes left gets the same URI, still ephemeral; at most 5
      `/key` per member in 15 minutes, counted before anything is asked; one `/key` per member at a time, so two at
      once can't make two codes (`key_hands_out_one_live_code_per_member`, `key_is_rate_limited_per_member`).
- [ ] The URI is this server's invite with the code replaced and checked by `format_invite`: in public mode it pins the
      gateway certificate's fingerprint, read from the certificate at start (`serve::discord_invite`); a public
      server without a certificate gets `key.failed`, never an unpinned invite.
- [ ] The code and the URI never reach a log line or an event: `GameCall` and `GameAnswer` hide the code from
      `Debug`, and `discord.key.requested` records the outcome (and a down game's reason) only. The invite's note,
      `Discord /key <member id>`, is in the game's database for staff, as every invite's note is.

### Text

- [ ] Every sentence is in `catalog/en.json` and `catalog/pt-BR.json`, with the same keys and placeholders
      (`both_catalogs_have_the_same_keys_and_placeholders`); a placeholder's value is never searched for more
      placeholders (`fill`).

### What is left open

- [ ] A member who can see the bot in a guild can ask for keys: the guild's own membership is the gate. A server that
      wants a smaller circle sets `guild_ids` and keeps the bot out of other guilds. `moderator_role_ids` (A7) is not
      read yet; it waits for the moderation commands.
- [ ] A dashboard that answers `enabled: true` with another site's link sends members there. The dashboard is the
      host's own and trusted; point `dashboard_url` only at it.
- [ ] Nothing here was run against Discord in CI: the gateway client and the REST calls are tested by hand.

## Sign-off

Reviewed at commit `________` on `________` by `________`.
