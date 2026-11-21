<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Signing in with Discord (or another OAuth provider)

A server can let players make their account, and get it back on another PC, by signing in with Discord instead of
typing a name and an invite code. The launcher shows **Continue with Discord** on the "Pick a name" step; the player
signs in in their browser and the launcher carries on by itself.

It is off until the host sets it up. The rest of this page is the host's half, then how it works.

## Set up the Discord preset

You need a Discord application. The one your [Discord bot](../restricted/docs/hosting.md#the-discord-bot) uses works,
or make a new one.

1. Open <https://discord.com/developers/applications>, pick or make the application, and go to **OAuth2**.
2. Under **Redirects**, add exactly this address and save:

   ```text
   http://127.0.0.1:61190/oauth/callback
   ```

   That is the player's own PC: the launcher listens there for the few minutes a sign-in takes. It is the same for
   every server, and Discord only sends a player back to it.

3. Copy the **Client ID**, and click **Reset Secret** to get the **Client Secret** (copy it into a file, such as
   `oauth-secret.txt`).
4. Run setup with the client id, and the secret on stdin (never as an argument, which other users and your shell's
   history can see):

   ```powershell
   Get-Content oauth-secret.txt | bin\dainty-host setup --oauth-discord 123456789012345678 --oauth-client-secret
   ```

   On Linux: `bin/dainty-host setup --oauth-discord 123456789012345678 --oauth-client-secret < oauth-secret.txt`.
   In Docker: `docker compose run --rm -T host setup --oauth-discord 123456789012345678 --oauth-client-secret
   < oauth-secret.txt`.

5. Restart the server. Delete `oauth-secret.txt`; the secret is kept in `secrets.env`.

That is all a server needs. `settings.toml` now has:

```toml
[oauth]
preset = "discord"
client_id = "123456789012345678"
required = false
```

### Only members of your Discord server

Add the server (guild) ids a player must be in, with Developer Mode on in Discord, then right-click the server and
**Copy Server ID**:

```powershell
bin\dainty-host setup --oauth-guild 123456789012345678
```

or `guild_ids = ["123456789012345678"]` in `[oauth]`. Discord then also asks the player to share which servers they
are in, and someone in none of them is refused with "That account isn't in the server's Discord".

### Discord as the only way in

```powershell
bin\dainty-host setup --oauth-required
```

(`required = true`) turns the name-and-invite form off: new accounts are made only by signing in. Signing up still
follows `registration`:

| `registration` | Who can make an account by signing in |
| --- | --- |
| `"open"` | Anyone the provider signs in, and with `guild_ids`, only members of those servers. The usual pairing with `required = true` |
| `"invite"` | Only a player whose invite carries a code, as with the form |
| `"none"` | Nobody new. Players who already signed in once still can |

`--oauth-optional` turns `required` back off, and `--no-oauth` removes sign-in (the secret stays in `secrets.env`).

## Another provider

`preset = "custom"` takes any OAuth 2.0 provider with authorization codes, PKCE (S256) and an endpoint that answers
the signed-in user as JSON. Register the same redirect, `http://127.0.0.1:61190/oauth/callback`, with it. GitHub,
for example:

```toml
[oauth]
preset = "custom"
provider_id = "github"          # accounts are linked under it; never change it once players have signed in
name = "GitHub"                 # the button says "Continue with GitHub"
client_id = "Iv1.0123456789abcdef"
authorize_url = "https://github.com/login/oauth/authorize"
token_url = "https://github.com/login/oauth/access_token"
userinfo_url = "https://api.github.com/user"
scopes = ["read:user"]
subject_field = "id"            # the userinfo field with the user's stable id (left out: "sub")
username_field = "login"        # the field with their name (left out: "preferred_username")
required = false
```

An OpenID Connect provider (Keycloak, Authentik, Google, Microsoft) needs only the three addresses: the defaults
(`scopes = ["openid", "profile"]`, `sub`, `preferred_username`) fit them. Every address must be `https://`. The
secret goes in the same way, `setup --oauth-client-secret`.

## What the player sees

1. "Pick a name" shows **Continue with Discord** under the name field. The name is optional: left empty, the account
   takes their Discord name (letters, digits and `_`, cut to 16).
2. Their browser opens Discord's page; they approve, and the tab says they can close it.
3. The launcher has the account key, as after a normal sign-up, and goes on to the game folder.

Signing in again on another PC gets the same account back: the server makes a new key for that PC, and the key on
the other PC stops working (an account has one key). A player who first made their account with a name and invite
code can't link it to Discord yet.

If port 61190 is taken on the player's PC, the launcher says so; closing the program that holds it fixes it.

## How it works

```text
launcher                        server (backend)                      Discord
   | POST /dainty/api/OAuthStart     |                                    |
   |-------------------------------->| state, PKCE verifier (5 min)       |
   |<--------------------------------| authorizeUrl, state, ticket        |
   | browser: authorizeUrl ------------------------------------------------>|
   |<-- 127.0.0.1:61190/oauth/callback?code&state --------------------------|
   | POST /dainty/api/OAuthFinish {State, Code, Ticket, Username?}         |
   |-------------------------------->| code + verifier + secret --------->|
   |                                 |<----------------------- the user --|
   |<--------------------------------| UUK (the account key), Username    |
```

- The server holds the client secret; it never reaches the launcher. The provider's code is useless without the
  PKCE verifier, which also never leaves the server.
- The `ticket` is the server's HMAC of the `state`, held only by the launcher that started the sign-in. Another
  program on the player's PC that catches the redirect has the code and the `state`, but not the ticket, so it can't
  trade them for the key.
- The launcher opens only an `https://` address, and for the Discord preset only one on `discord.com`. It takes the
  redirect only from loopback and only with its own `state`.
- `OAuthStart` and `OAuthFinish` are rate-limited at the gateway with `Register`. Accounts are linked in
  `auth_identities` (provider `discord`, or the custom `provider_id`), the same table the admin dashboard's Discord
  sign-in uses, and a `state` is spent once (`discord_oauth_transactions`).

The security checklist is [security/oauth.md](security/oauth.md).
