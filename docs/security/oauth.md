<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: OAuth sign-in (RX3)

The checklist for the launcher's OAuth sign-in ([oauth.md](../oauth.md)): the backend's `OAuthStart` and
`OAuthFinish`, the provider client, the host kit's `[oauth]`, and the launcher's loopback redirect listener.

How to use it: before sign-in is turned on for a public server, copy this list into the review, check each item in
the code (not from memory), link the code or test that shows it, and sign at the end with the date and the commit
reviewed. An item that does not hold is a bug to fix first, not a box to leave open.

## What it is and where the boundary is

Sign-in hands out account keys to whoever the provider vouches for, so the provider's answer is the credential and
everything around it is untrusted.

- **In (server):** `POST /dainty/api/OAuthStart` and `POST /dainty/api/OAuthFinish` from anyone, through the
  gateway; the provider's token and userinfo answers (and Discord's guild list), over HTTPS only.
- **In (launcher):** one TCP listener on `127.0.0.1:61190` for the length of a sign-in, which any local program and
  any web page the browser opens can reach; `OAuthStart`'s and `OAuthFinish`'s replies from the joined server.
- **Out:** the provider's authorize URL opened in the player's browser; the code, the PKCE verifier and the client
  secret from the server to the provider's token endpoint.
- **Files:** the backend's env file (`DAINTY_OAUTH_*`), written by `dainty-host setup` from `[oauth]` and
  secrets.env's `DAINTY_OAUTH_CLIENT_SECRET`.

Code: `crates/rustic-isles-auth/src/oauth.rs`, `crates/rustic-isles-launcher-api/src/oauth.rs` and
`launcher_api.rs` (`create_account`, `Register`'s `oauth_required`), `crates/rustic-isles-store/src/repo/auth.rs`
(`find_for_provider`, `create_for_provider`), `crates/dainty-gateway/src/policy.rs` (`PUBLIC_LAUNCHER_API`, the
`register` limit class), `crates/dainty-hostkit/src/hostconfig/oauth.rs`, `crates/dainty-launcher-kit/src/net/
oauth_redirect.rs`, `net/hostapi.rs` and `hostapi/parse.rs` (`oauth_*`), `platform/links.rs` (`check_oauth`) and
`controller/account.rs` (`sign_in`).

Fuzz target in `fuzz`:

- `launcher_oauth`: a request head of any bytes to the redirect listener, and `OAuthStart`'s and `OAuthFinish`'s
  replies, parse or are refused, never panic; a code comes back only for the expected `state` and the callback path,
  is non-empty printable ASCII of at most 2048 bytes; an `OAuthFinish` key comes only from a 200 and is a plausible
  key; an authorize URL `check_oauth` passes is `https` without credentials, and for `discord` on `discord.com`.

## Checklist

### Server

- [ ] The client secret is read only from `DAINTY_OAUTH_CLIENT_SECRET`, sent only to the provider's token endpoint,
      absent from `OAuthProvider`'s `Debug`, from every answer and from every log line.
- [ ] Every provider URL is `https://` without credentials (`OAuthProvider::from_settings`, the host kit's
      `is_https`), and the HTTP client is `Schemes::HttpsOnly`, 15 s per call, 256 KiB per answer.
- [ ] `OAuthStart` makes `state` from 24 random bytes and the verifier from 32, stores both with a five-minute
      expiry, and answers the verifier's S256 challenge only.
- [ ] The ticket is an HMAC-SHA-256 of the `state` under `API_KEY_HASH_SECRET` and is compared in constant time
      before the `state` is spent or the provider called.
- [ ] A `state` is spent once (`consume_by_state` is atomic: unconsumed and unexpired), so a code can't be replayed.
- [ ] Guild membership is checked from Discord's own answer, before any account is found or made.
- [ ] An existing link signs in only an eligible account (banned or disabled are refused with the reason, as
      `GetUserInfo` refuses), and its new key replaces the old one.
- [ ] A new account follows `REGISTRATION_MODE` (an invite is spent in the same write), the name rules, and links the
      identity in that write; a subject linked meanwhile rolls it back (`oauth_linked`).
- [ ] `Register` refuses with `oauth_required` while `DAINTY_OAUTH_REQUIRED` is on.
- [ ] A link (`OAuthFinish` with a key) checks the key's holder is eligible before the `state` is spent, links only
      to that account, refuses an identity linked elsewhere (`oauth_linked_elsewhere`) or an account linked to
      another identity (`oauth_account_has_link`), and answers no key.
- [ ] The gateway passes only `POST` to the two routes and limits them as `register`.
- [ ] Erasure removes identities at every provider (`delete_every_provider_for_account`).

### Host kit

- [ ] `DAINTY_OAUTH_CLIENT_SECRET` is a provided secret: never made, never a settings.toml field or an argument
      (`setup --oauth-client-secret` reads stdin), and written only into the backend's file.
- [ ] Every `DAINTY_OAUTH_*` is written, empty when off, so none comes from the shell; they are refused in `[env]`
      passthrough, and an override from the environment is redacted in the start-up warning.
- [ ] Sign-in stays off until the secret is set, and setup says so.

### Launcher

- [ ] The listener binds `127.0.0.1` only, for one sign-in at a time, and closes when it ends (code, refusal,
      five-minute timeout or `cancel_task`).
- [ ] A request head is read up to 8 KiB with a 5 s timeout; only `GET /oauth/callback` with the expected `state`
      (compared first, each parameter once) ends the wait; anything else is answered and ignored.
- [ ] The page answered holds only catalog text, under `default-src 'none'`, `no-store` and `no-referrer`.
- [ ] Only an `https` authorize URL without credentials is opened, and for the Discord preset only on
      `discord.com` without a port (`check_oauth`).
- [ ] The ticket and the code go only to the server that started the sign-in, over the endpoint (and pin) the key
      goes to; the key is stored in that server's slot as `register` stores it.

## Sign-off

Reviewed by: ____________ Date: ____________ Commit: ____________
