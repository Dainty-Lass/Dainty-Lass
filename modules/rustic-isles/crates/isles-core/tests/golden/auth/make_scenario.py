# Writes scenario.json: the requests both Node (record.cjs) and the Rust port (tests/auth_golden.rs) make, in order.
#   python make_scenario.py > scenario.json
# `{{var}}` is a value captured from an earlier answer; `{{code:<email>}}` is not used, a `mailcode` step takes the code
# last mailed to an address. A capture is "field.path", "field.path~regex" (group 1 of the regex over that field's text)
# or "~header~regex" (over a response header).
import json
import sys

P = "correct horse battery"
EXE = "a" * 64
RT_MAIN = "b" * 64
RT_PROXY = "c" * 64
ARTIFACTS = [{"name": "MysticParadox.dll", "sha256": RT_MAIN}, {"name": "winmm.dll", "sha256": RT_PROXY}]
steps = []
GS = {"x-mysticparadox-gameserver-apikey": "gs-key-1"}


def step(name, method, path, json_=None, auth=None, headers=None, raw=None, content_type=None, capture=None,
         versions=None, form=None):
    s = {"name": name, "method": method, "path": path}
    h = dict(headers or {})
    if auth:
        h["authorization"] = "Bearer " + auth
    if h:
        s["headers"] = h
    if json_ is not None:
        s["json"] = json_
    if raw is not None:
        s["raw"] = raw
        s["contentType"] = content_type or "application/json"
    if form is not None:
        s["form"] = form
    if capture:
        s["capture"] = capture
    if versions:
        s["versions"] = versions
    steps.append(s)


def env(name, values):
    steps.append({"name": name, "env": values})


def mail(name, email, var):
    steps.append({"name": name, "mailcode": {"email": email, "var": var}})


def game_session(name, auth, extra=None, capture=None):
    body = {"buildChangelist": 647472, "executableSha256": EXE, "runtimeChannel": "stable",
            "runtimeManifestVersion": "0.1.78", "runtimeArtifacts": ARTIFACTS}
    body.update(extra or {})
    step(name, "POST", "/launcher/v1/game-sessions", body, auth=auth, capture=capture)


A = "{{a.access}}"

# ---- the game client's public routes
step("features", "GET", "/features/platform/win")
step("link epic", "GET", "/account/link/epic/ABC")
step("is banned", "GET", "/isbanned")
step("unknown route", "GET", "/nothing/here")

# ---- the launcher's public routes
step("status", "GET", "/launcher/v1/status")
step("legal", "GET", "/launcher/v1/legal")
step("notices", "GET", "/launcher/v1/notices")
step("username ok", "GET", "/launcher/v1/username?name=Slayer1")
step("username too short", "GET", "/launcher/v1/username?name=ab")
step("username symbol", "GET", "/launcher/v1/username?name=Slay%20er")
step("username reserved", "GET", "/launcher/v1/username?name=AdminBob")
step("username offensive", "GET", "/launcher/v1/username?name=Dick69")
step("username absent", "GET", "/launcher/v1/username")
step("username repeated", "GET", "/launcher/v1/username?name=Slayer1&name=Slayer2")

# ---- registration
step("register bad email", "POST", "/launcher/v1/auth/register", {"email": "nope", "password": P})
step("register short password", "POST", "/launcher/v1/auth/register",
     {"email": "slayer1@example.test", "password": "short"})
step("register bad name", "POST", "/launcher/v1/auth/register",
     {"displayName": "ab", "email": "slayer1@example.test", "password": P})
step("register reserved name", "POST", "/launcher/v1/auth/register",
     {"displayName": "AdminBob", "email": "slayer1@example.test", "password": P})
step("register offensive name", "POST", "/launcher/v1/auth/register",
     {"displayName": "Dick", "email": "slayer1@example.test", "password": P})
step("register no body", "POST", "/launcher/v1/auth/register")
step("register ok", "POST", "/launcher/v1/auth/register",
     {"displayName": "Slayer1", "email": "Slayer1@Example.Test", "password": P, "deviceId": "dev-a",
      "deviceName": "PC A"}, capture={"a.userId": "account.userId"})
step("register same email", "POST", "/launcher/v1/auth/register",
     {"displayName": "Other", "email": "slayer1@example.test", "password": P})
step("register same name", "POST", "/launcher/v1/auth/register",
     {"displayName": "SLAYER1", "email": "other@example.test", "password": P})
step("register without a name", "POST", "/launcher/v1/auth/register",
     {"email": "slayer2@example.test", "password": P}, capture={"b.userId": "account.userId"})
step("register blank name", "POST", "/launcher/v1/auth/register",
     {"displayName": "  ", "email": "slayer3@example.test", "password": P})
step("username taken", "GET", "/launcher/v1/username?name=slayer1")

# ---- password sign-in
step("login wrong password", "POST", "/launcher/v1/auth/login",
     {"email": "slayer1@example.test", "password": "wrong password"})
step("login unknown email", "POST", "/launcher/v1/auth/login", {"email": "nobody@example.test", "password": P})
step("login missing fields", "POST", "/launcher/v1/auth/login", {})
step("login email not a string", "POST", "/launcher/v1/auth/login", {"email": 5, "password": P})
step("login no body", "POST", "/launcher/v1/auth/login")
step("login ok", "POST", "/launcher/v1/auth/login",
     {"email": " SLAYER1@example.test ", "password": P, "deviceId": "dev-a", "deviceName": "PC A"},
     capture={"a.access": "accessToken", "a.refresh": "refreshToken"})
step("login b", "POST", "/launcher/v1/auth/login", {"email": "slayer2@example.test", "password": P},
     capture={"b.access": "accessToken"})
step("malformed json", "POST", "/launcher/v1/auth/login", raw='{"email":')
step("json that is not an object", "POST", "/launcher/v1/auth/login", raw='"text"')
step("form body", "POST", "/launcher/v1/auth/login",
     form="email=slayer1%40example.test&password=correct+horse+battery")
step("me", "GET", "/launcher/v1/me", auth=A)
step("me without a token", "GET", "/launcher/v1/me")
step("me with a bad token", "GET", "/launcher/v1/me", auth="not.a.token")
step("me with a wrong scheme", "GET", "/launcher/v1/me", headers={"authorization": "Basic abc"})
step("me b needs a username", "GET", "/launcher/v1/me", auth="{{b.access}}")

# ---- usernames
step("set username too short", "POST", "/launcher/v1/username", {"username": "x"}, auth="{{b.access}}")
step("set username reserved", "POST", "/launcher/v1/username", {"username": "Moderator"}, auth="{{b.access}}")
step("set username taken", "POST", "/launcher/v1/username", {"username": "slayer1"}, auth="{{b.access}}")
step("set username not a string", "POST", "/launcher/v1/username", {"username": 5}, auth="{{b.access}}")
step("set username ok", "POST", "/launcher/v1/username", {"username": "Slayer2"}, auth="{{b.access}}")
step("set username again", "POST", "/launcher/v1/username", {"username": "Slayer2"}, auth="{{b.access}}")
step("set username without a token", "POST", "/launcher/v1/username", {"username": "Slayer9"})

# ---- refresh sessions
step("refresh missing", "POST", "/launcher/v1/auth/refresh", {})
step("refresh unknown", "POST", "/launcher/v1/auth/refresh", {"refreshToken": "f" * 64})
step("refresh ok", "POST", "/launcher/v1/auth/refresh", {"refreshToken": "{{a.refresh}}", "deviceName": "PC A2"},
     capture={"a.access2": "accessToken", "a.refresh2": "refreshToken"})
step("refresh reuse revokes the family", "POST", "/launcher/v1/auth/refresh", {"refreshToken": "{{a.refresh}}"})
step("the family is gone", "GET", "/launcher/v1/me", auth="{{a.access2}}")
step("the new token is revoked too", "POST", "/launcher/v1/auth/refresh", {"refreshToken": "{{a.refresh2}}"})
step("login again", "POST", "/launcher/v1/auth/login", {"email": "slayer1@example.test", "password": P},
     capture={"a.access": "accessToken", "a.refresh": "refreshToken"})
step("logout", "POST", "/launcher/v1/auth/logout", auth=A)
step("after logout", "GET", "/launcher/v1/me", auth=A)
step("login once more", "POST", "/launcher/v1/auth/login", {"email": "slayer1@example.test", "password": P},
     capture={"a.access": "accessToken", "a.refresh": "refreshToken"})
step("login on another device", "POST", "/launcher/v1/auth/login",
     {"email": "slayer1@example.test", "password": P, "deviceId": "dev-b"}, capture={"a.accessB": "accessToken"})
step("logout all", "POST", "/launcher/v1/auth/logout-all", auth=A)
step("the other device is out", "GET", "/launcher/v1/me", auth="{{a.accessB}}")
step("login for the rest", "POST", "/launcher/v1/auth/login", {"email": "slayer1@example.test", "password": P},
     capture={"a.access": "accessToken", "a.refresh": "refreshToken"})
step("login b again", "POST", "/launcher/v1/auth/login", {"email": "slayer2@example.test", "password": P},
     capture={"b.access": "accessToken"})

# ---- policy, consent, deletion
step("policy", "GET", "/launcher/v1/policy", auth=A)
step("consent before", "GET", "/launcher/v1/account/consent", auth=A)
step("consent without age", "POST", "/launcher/v1/account/consent",
     {"termsVersion": "2026-10-14", "privacyVersion": "2026-10-17"}, auth=A)
step("consent missing versions", "POST", "/launcher/v1/account/consent", {"ageConfirmed": True}, auth=A)
step("consent outdated", "POST", "/launcher/v1/account/consent",
     {"ageConfirmed": True, "termsVersion": "2020-01-01", "privacyVersion": "2026-10-17"}, auth=A)
step("consent ok", "POST", "/launcher/v1/account/consent",
     {"ageConfirmed": True, "termsVersion": "2026-10-14", "privacyVersion": "2026-10-16"}, auth=A)
step("consent after", "GET", "/launcher/v1/account/consent", auth=A)
step("deletion before", "GET", "/launcher/v1/account/deletion", auth=A)
step("deletion mismatch", "POST", "/launcher/v1/account/deletion", {"confirmUsername": "nope"}, auth=A)
step("deletion no body", "POST", "/launcher/v1/account/deletion", auth=A)
step("deletion asked", "POST", "/launcher/v1/account/deletion", {"confirmUsername": " slayer1 "}, auth=A)
step("deletion asked again", "POST", "/launcher/v1/account/deletion", {"confirmUsername": "Slayer1"}, auth=A)
game_session("a deactivated account cannot play", A)
step("deletion cancelled", "DELETE", "/launcher/v1/account/deletion", auth=A)
step("deletion cancelled again", "DELETE", "/launcher/v1/account/deletion", auth=A)
step("deletion after", "GET", "/launcher/v1/account/deletion", auth=A)

# ---- game sessions and the game's token
step("game session without a token", "POST", "/launcher/v1/game-sessions", {})
step("game session, bad channel", "POST", "/launcher/v1/game-sessions", {"runtimeChannel": "preview"}, auth=A)
step("game session, no changelist", "POST", "/launcher/v1/game-sessions", {"runtimeChannel": "stable"}, auth=A)
step("game session, no executable", "POST", "/launcher/v1/game-sessions",
     {"runtimeChannel": "stable", "buildChangelist": 647472}, auth=A)
game_session("game session, wrong channel for the account", A, {"runtimeChannel": "beta"})
game_session("game session, wrong build", A, {"buildChangelist": 1})
game_session("game session, unapproved executable", A, {"executableSha256": "d" * 64})
game_session("game session, stale manifest version", A, {"runtimeManifestVersion": "0.1.77"})
game_session("game session, a changed runtime", A, {"runtimeArtifacts": [
    {"name": "MysticParadox.dll", "sha256": "e" * 64}, {"name": "winmm.dll", "sha256": RT_PROXY}]})
game_session("game session, an unknown file", A, {"runtimeArtifacts": ARTIFACTS + [
    {"name": "inject.dll", "sha256": "e" * 64}]})
game_session("game session, a realm with realms off", A, {"realm": {"name": "friends"}})
game_session("game session ok", A, capture={"a.code": "exchangeCode"})
game_session("game session for b", "{{b.access}}", capture={"b.code": "exchangeCode"})
step("oauth token without a body type", "POST", "/account/api/oauth/token")
step("oauth token without a code", "POST", "/account/api/oauth/token", {"grant_type": "exchange_code"})
step("oauth client credentials", "POST", "/account/api/oauth/token", {"grant_type": "client_credentials"})
step("oauth unknown code", "POST", "/account/api/oauth/token", {"exchange_code": "f" * 64})
step("oauth code", "POST", "/account/api/oauth/token", {"exchange_code": "{{a.code}}"},
     capture={"a.game": "access_token"})
step("oauth code reused", "POST", "/account/api/oauth/token", {"exchange_code": "{{a.code}}"})
step("oauth code of b", "POST", "/account/api/oauth/token", {"exchange_code": "{{b.code}}"},
     capture={"b.game": "access_token"})
step("oauth as form", "POST", "/account/api/oauth/token", form="grant_type=client_credentials")
step("oauth verify", "GET", "/account/api/oauth/verify", auth="{{a.game}}")
step("oauth verify, launcher token", "GET", "/account/api/oauth/verify", auth=A)
step("oauth verify without a token", "GET", "/account/api/oauth/verify")
step("oauth verify, two-part rule", "GET", "/account/api/oauth/verify", headers={"authorization": "Bearer a b"})
step("kill sessions", "DELETE", "/account/api/oauth/sessions/kill")
step("kill one session", "DELETE", "/account/api/oauth/sessions/kill/abc")
step("public account", "GET", "/account/api/public/account/{{a.userId}}")
step("public account, with a token", "GET", "/account/api/public/account/{{a.userId}}", auth="{{a.game}}")
step("public account unknown", "GET", "/account/api/public/account/nobody")
step("public account, the legacy alias", "GET", "/account/api/public/account/dev-user", auth="{{a.game}}")
step("external auths", "GET", "/account/api/public/account/{{a.userId}}/externalAuths")
step("external auths, unknown", "GET", "/account/api/public/account/nobody/externalAuths")
step("by display name", "GET", "/account/api/public/account/displayName/%20SLAYER1%20")
step("by display name, unknown", "GET", "/account/api/public/account/displayName/Nobody")
step("bulk without a token", "GET", "/account/api/public/account")
step("bulk", "GET", "/account/api/public/account?accountId={{a.userId}}&accountId={{b.userId}}&accountId=nobody",
     auth="{{a.game}}")
step("bulk of oneself", "GET", "/account/api/public/account", auth="{{a.game}}")
step("bulk, the legacy alias", "GET", "/account/api/public/account?accountId=dev-user", auth="{{a.game}}")

# ---- the game's login routes
step("game login", "POST", "/login", {"email": "{{a.userId}}"}, auth="{{a.game}}")
step("game login, another email", "POST", "/login", {"email": "someone"}, auth="{{a.game}}")
step("game login, no body", "POST", "/login", auth="{{a.game}}")
step("game login without a token", "POST", "/login", {"email": "{{a.userId}}"})
step("game login, bad token", "POST", "/login", {"email": "{{a.userId}}"}, auth="garbage")
step("game login, launcher token", "POST", "/login", {"email": "{{a.userId}}"}, auth=A)
step("game login, lower-case scheme", "POST", "/login", {"email": "{{a.userId}}"},
     headers={"authorization": "bearer {{a.game}}"})
step("game login, odd scheme", "POST", "/login", {"email": "{{a.userId}}"},
     headers={"authorization": "BeArEr {{a.game}}"})
step("game server login", "POST", "/login", {"email": "{{a.userId}}"}, headers=GS)
step("game server key refused", "POST", "/login", {"email": "x"},
     headers={"x-mysticparadox-gameserver-apikey": "wrong"})
step("game server key and a good token", "POST", "/login", {"email": "{{a.userId}}"}, auth="{{a.game}}", headers=GS)
step("game server key and a bad token", "POST", "/login", {"email": "x"}, auth="garbage", headers=GS)
step("accountinfo", "GET", "/accountinfo", auth="{{a.game}}")
step("accountinfo of a game server", "GET", "/accountinfo", headers=GS)
step("tags", "GET", "/tags", auth="{{a.game}}")
step("tags of a game server", "GET", "/tags", headers=GS)
step("game session", "PUT", "/gamesession/epic", auth="{{a.game}}")
step("game session epiceos", "PUT", "/gamesession/epiceos", auth="{{a.game}}", versions=["1.14.7"])
step("game session, game server only", "PUT", "/gamesession/epic", headers=GS)
step("public accountinfo", "POST", "/accountinfo/public", {"accountId": "{{b.userId}}"}, auth="{{a.game}}")
step("public accountinfo, unknown", "POST", "/accountinfo/public", {"accountId": "nobody"}, auth="{{a.game}}")
step("public accountinfo, the alias", "POST", "/accountinfo/public", {"accountId": "mystpax"}, auth="{{a.game}}")
step("public accountinfo, no id", "POST", "/accountinfo/public", {}, auth="{{a.game}}")
step("public accountinfo, no body", "POST", "/accountinfo/public", auth="{{a.game}}")
step("public accountinfo without a token", "POST", "/accountinfo/public", {"accountId": "x"})
step("migration trigger", "POST", "/migration/trigger", auth="{{a.game}}")
step("migration status", "GET", "/migration/status", auth="{{a.game}}")
step("migration status without a token", "GET", "/migration/status")

# ---- the network keys
step("key generate", "POST", "/key/generate", {"candidateId": "cand-1"}, auth="{{a.game}}")
step("key generate, no candidate", "POST", "/key/generate", {}, auth="{{b.game}}")
step("key generate without a token", "POST", "/key/generate", {})
step("key consume by candidate", "POST", "/key/consume", {"candidateId": "cand-1"}, headers=GS, versions=["1.14.7"])
step("key consume by token", "POST", "/key/consume", {"token": "Bearer {{b.game}}"}, headers=GS, versions=["1.14.7"])
step("key consume by query", "POST", "/key/consume?token={{a.game}}", headers=GS, versions=["1.14.7"])
step("key consume, an unknown token", "POST", "/key/consume", {"token": "who"}, headers=GS, versions=["1.14.7"])
step("key consume without a body", "POST", "/key/consume", headers=GS, versions=["1.14.7"])

# ---- Discord
step("discord start, not configured", "POST", "/launcher/v1/auth/discord/start")
step("discord callback, nothing", "GET", "/launcher/v1/auth/discord/callback")
step("discord callback, unknown state", "GET", "/launcher/v1/auth/discord/callback?code=code-1&state=nope")
step("discord callback, repeated parameter", "GET",
     "/launcher/v1/auth/discord/callback?code=a&code=b&state=nope")
step("discord callback for a browser", "GET", "/launcher/v1/auth/discord/callback?code=code-1&state=nope",
     headers={"accept": "text/html,application/xhtml+xml"})
step("discord link, not configured", "POST", "/launcher/v1/account/discord/link/start", {}, auth=A)
env("discord on", {"DISCORD_CLIENT_ID": "client-1", "DISCORD_CLIENT_SECRET": "secret-1",
                   "DISCORD_REDIRECT_URI": "https://paradox.example.test/launcher/v1/auth/discord/callback"})
step("discord start", "POST", "/launcher/v1/auth/discord/start",
     capture={"d.signInState": "authorizeUrl~state=([0-9a-f]+)"})
step("discord callback, unlinked user", "GET",
     "/launcher/v1/auth/discord/callback?code=code-9000000000000001&state={{d.signInState}}")
step("discord link without a token", "POST", "/launcher/v1/account/discord/link/start", {})
step("discord link start", "POST", "/launcher/v1/account/discord/link/start", {}, auth=A,
     capture={"d.linkState": "authorizeUrl~state=([0-9a-f]+)"})
step("discord link callback, a failed exchange", "GET",
     "/launcher/v1/auth/discord/callback?code=bad&state={{d.linkState}}")
step("discord link start again", "POST", "/launcher/v1/account/discord/link/start", {}, auth=A,
     capture={"d.linkState": "authorizeUrl~state=([0-9a-f]+)"})
step("discord link callback", "GET",
     "/launcher/v1/auth/discord/callback?code=code-9000000000000001&state={{d.linkState}}")
step("discord link callback replayed", "GET",
     "/launcher/v1/auth/discord/callback?code=code-9000000000000001&state={{d.linkState}}")
step("me with a discord link", "GET", "/launcher/v1/me", auth=A)
step("discord link twice", "POST", "/launcher/v1/account/discord/link/start", {}, auth=A)
step("discord link for b", "POST", "/launcher/v1/account/discord/link/start", {}, auth="{{b.access}}",
     capture={"d.linkStateB": "authorizeUrl~state=([0-9a-f]+)"})
step("discord link of a linked user", "GET",
     "/launcher/v1/auth/discord/callback?code=code-9000000000000001&state={{d.linkStateB}}")
step("discord sign-in start", "POST", "/launcher/v1/auth/discord/start",
     capture={"d.signInState": "authorizeUrl~state=([0-9a-f]+)"})
step("discord sign-in callback", "GET",
     "/launcher/v1/auth/discord/callback?code=code-9000000000000001&state={{d.signInState}}",
     capture={"d.completion": "~location~code=([0-9a-f]+)"})
step("discord complete", "POST", "/launcher/v1/auth/discord/complete",
     {"code": "{{d.completion}}", "deviceName": "PC D"})
step("discord complete again", "POST", "/launcher/v1/auth/discord/complete", {"code": "{{d.completion}}"})
step("discord complete without a code", "POST", "/launcher/v1/auth/discord/complete", {})
step("discord unlink start without mail", "POST", "/launcher/v1/account/discord/unlink/start", {}, auth=A)

# ---- CORS
step("preflight", "OPTIONS", "/launcher/v1/auth/login", headers={"origin": "http://localhost:1420"})
step("preflight, unknown origin", "OPTIONS", "/launcher/v1/auth/login", headers={"origin": "https://evil.example"})
step("preflight, no origin", "OPTIONS", "/launcher/v1/auth/login")
step("preflight, any launcher path", "OPTIONS", "/launcher/v1/nothing", headers={"origin": "https://tauri.localhost"})
step("allowed origin on a read", "GET", "/launcher/v1/legal", headers={"origin": "https://tauri.localhost"})
step("unknown origin on a read", "GET", "/launcher/v1/legal", headers={"origin": "https://evil.example"})
step("origin on a game route", "GET", "/isbanned", headers={"origin": "https://evil.example"})

# ---- code mail: verification, resend, reset, unlink
step("resend without mail", "POST", "/launcher/v1/auth/email/resend", {"email": "slayer1@example.test"})
step("forgot without mail", "POST", "/launcher/v1/auth/password/forgot", {"email": "slayer1@example.test"})
env("mail on, verification required", {
    "MYSTICPARADOX_EMAIL_VERIFICATION": "required", "MYSTICPARADOX_SMTP_HOST": "smtp.example.test",
    "MYSTICPARADOX_SMTP_USER": "postmaster", "MYSTICPARADOX_SMTP_PASSWORD": "fixture-password"})
step("register under verification", "POST", "/launcher/v1/auth/register",
     {"displayName": "Slayer4", "email": "Slayer4@example.test", "password": P}, capture={"c.userId": "account.userId"})
step("login before verification", "POST", "/launcher/v1/auth/login", {"email": "slayer4@example.test", "password": P})
step("login before verification, second", "POST", "/launcher/v1/auth/login",
     {"email": "slayer4@example.test", "password": P})
step("login before verification, wrong password", "POST", "/launcher/v1/auth/login",
     {"email": "slayer4@example.test", "password": "wrong password"})
step("verify without a password", "POST", "/launcher/v1/auth/email/verify",
     {"email": "slayer4@example.test", "code": "000000"})
step("verify with a wrong password", "POST", "/launcher/v1/auth/email/verify",
     {"email": "slayer4@example.test", "password": "wrong password", "code": "000000"})
step("verify with a malformed code", "POST", "/launcher/v1/auth/email/verify",
     {"email": "slayer4@example.test", "password": P, "code": "12ab"})
mail("the registration mail", "slayer4@example.test", "c.code")
step("verify with the code", "POST", "/launcher/v1/auth/email/verify",
     {"email": "slayer4@example.test", "password": P, "code": "{{c.code}}"}, capture={"c.access": "accessToken"})
step("verify with the code again", "POST", "/launcher/v1/auth/email/verify",
     {"email": "slayer4@example.test", "password": P, "code": "{{c.code}}"})
step("me after verification", "GET", "/launcher/v1/me", auth="{{c.access}}")
step("resend, malformed address", "POST", "/launcher/v1/auth/email/resend", {"email": "nope"})
step("resend for a verified account", "POST", "/launcher/v1/auth/email/resend", {"email": "slayer4@example.test"})
step("resend for nobody", "POST", "/launcher/v1/auth/email/resend", {"email": "nobody@example.test"})
step("forgot", "POST", "/launcher/v1/auth/password/forgot", {"email": "slayer4@example.test"})
step("forgot, malformed", "POST", "/launcher/v1/auth/password/forgot", {"email": ""})
mail("the reset mail", "slayer4@example.test", "c.reset")
step("reset with a short password", "POST", "/launcher/v1/auth/password/reset",
     {"email": "slayer4@example.test", "code": "{{c.reset}}", "newPassword": "short"})
step("reset with a wrong code", "POST", "/launcher/v1/auth/password/reset",
     {"email": "slayer4@example.test", "code": "000000", "newPassword": "a brand new password"})
step("reset for nobody", "POST", "/launcher/v1/auth/password/reset",
     {"email": "nobody@example.test", "code": "{{c.reset}}", "newPassword": "a brand new password"})
step("reset", "POST", "/launcher/v1/auth/password/reset",
     {"email": "slayer4@example.test", "code": "{{c.reset}}", "newPassword": "a brand new password"})
step("old sessions end at a reset", "GET", "/launcher/v1/me", auth="{{c.access}}")
step("login with the new password", "POST", "/launcher/v1/auth/login",
     {"email": "slayer4@example.test", "password": "a brand new password"})
step("login with the old password", "POST", "/launcher/v1/auth/login", {"email": "slayer4@example.test", "password": P})
step("an account from before verification is asked for a code", "POST", "/launcher/v1/auth/login",
     {"email": "slayer1@example.test", "password": P})
step("discord unlink start", "POST", "/launcher/v1/account/discord/unlink/start", {}, auth=A)
mail("the unlink mail", "slayer1@example.test", "a.unlink")
step("discord unlink, wrong code", "POST", "/launcher/v1/account/discord/unlink", {"code": "000000"}, auth=A)
step("discord unlink", "POST", "/launcher/v1/account/discord/unlink", {"code": "{{a.unlink}}"}, auth=A)
step("discord unlink again", "POST", "/launcher/v1/account/discord/unlink/start", {}, auth=A)
step("me after the unlink", "GET", "/launcher/v1/me", auth=A)

json.dump({"steps": steps}, sys.stdout, indent=1)
