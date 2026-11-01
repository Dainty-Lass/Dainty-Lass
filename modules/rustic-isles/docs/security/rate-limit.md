# Security: `isles-core` Request Budget and Rate Limiting

This document covers the security boundaries and invariants of the 1.14.7 HTTP request budget (`CreateRequestRateLimiter`, `crates/isles-core/src/http.rs` and `crates/isles-core/src/areas/auth/runtime.rs`) and the per-route `IsRateLimited` sliding windows (**RU5**).

---

## 1. What Is Limited

- **1.14.7 Backend Request Budget (`CreateRequestRateLimiter`)**:
  - Mounted on `Service::Backend` when serving `RouteSet::V1147` (`isles-1-14-7`), matching `reference/1.14.7/ParadoxBackend/src/app.ts`:
    1. Early mount on `/mystic/v1/hunts` (`app.ts` line 196) ahead of the body parsers and early routes.
    2. Main mount (`app.ts` line 245) ahead of `metricsRouter`, `launcherAuthRouter` (including CORS), `/admin`, all game and realm routers, and the 404 fallback.
  - A request that passes both the `/mystic/v1/hunts` mount and the main mount is counted at most once (`CountedFlag`).
  - Fixed 60-second window (`REQUEST_BUDGET_WINDOW_MS = 60_000`), matching `express-rate-limit` `8.7.0` `MemoryStore`.
  - Every counted response carries IETF `draft-8` `RateLimit` (`"<limit>-in-1min"; r=<remaining>; t=<reset_seconds>`) and `RateLimit-Policy` (`"<limit>-in-1min"; q=<limit>; w=60; pk=:<base64(sha256(key)[0..12])>:`) headers.
  - Requests exceeding the bucket quota are refused with HTTP `429 Too Many Requests`, `Retry-After: <reset_seconds>`, and `{"error":"Too many requests. Please retry later."}`.
  - Controlled by `MYSTICPARADOX_RATE_LIMIT` (enabled unless `0`, `off`, `false`, or `no`, case-insensitive and trimmed).
- **Per-Route Sliding Windows (`IsRateLimited`)**:
  - Enforced independently inside the auth, launcher, and realm directory handlers (such as login, registration, email verification, password reset, and realm creation/invitation).

## 2. Bucket Keys and Quotas

Each counted 1.14.7 backend request is assigned to one of three independent buckets in `check_request_budget` (`crates/isles-core/src/http.rs`):

1. **`gameserver:<address>`**:
   - Selected when `x-mysticparadox-gameserver-apikey` is present and non-empty after trimming.
   - Quota: `MYSTICPARADOX_GAMESERVER_RATE_LIMIT_PER_MINUTE` (positive integer, default `6000`).
   - Selecting this bucket does not authenticate the caller; `game_auth` still verifies the API key against the configured key hashes.
2. **`host:<capability_id>`**:
   - Selected when `x-mystic-host-capability` resolves through `rifts_seam::GuardHooks::rate_limit_bucket` (`rifts_guard::Guard` backed by `RealmsHuntSessions::capability_by_digest` and a 10-second, 1,024-entry cache) to an active, unrevoked, non-expired host capability.
   - Quota: `MYSTICPARADOX_HOST_RATE_LIMIT_PER_MINUTE` (positive integer, default `6000`).
   - Missing, malformed, unknown, revoked, or expired capabilities—or any resolver error—fall back to the `client:<address>` bucket.
3. **`client:<address>`**:
   - Selected for all other requests.
   - Quota: `MYSTICPARADOX_RATE_LIMIT_PER_MINUTE` (positive integer, default `600`).

### Client Address Resolution and IPv6 Subnet Grouping

- Client addresses follow Express's `trust proxy: loopback` (`crates/isles-core/src/areas/auth/util.rs`, `forwarded_client_address`): `X-Forwarded-For` is honoured only when the socket peer is a loopback address (`127.0.0.0/8`, `::1`, or `::ffff:127.0.0.0/104`), taking the rightmost non-loopback hop; non-loopback peers always use their own socket address.
- `ip_key_generator` (`express-rate-limit` `8.7.0` `ipKeyGenerator(ip, 56)`) normalizes IPv4-mapped IPv6 addresses (`::ffff:a.b.c.d`) to IPv4 and masks all other IPv6 addresses to their `/56` network prefix (`<network>/56`), preventing a single client with an IPv6 prefix allocation from rotating interface identifiers to bypass the `client:` or `gameserver:` limit.

## 3. What Is Exempt

- **Probes mounted before `app.use(RequestLimiter)` (`app.ts` lines 214–242)**:
  - `GET` and `HEAD` `/`
  - `GET` and `HEAD` `/health/live`
  - `GET` and `HEAD` `/health/ready`
  - `GET` and `HEAD` `/health/release-gates` when `MYSTICPARADOX_ADMIN_API` is enabled (when disabled, `/health/release-gates` is not mounted above the limiter and falls through to the counted 404 path).
- **Pre-limiter origin validation (`app.ts` lines 167–185)**:
  - Requests with a malformed or disallowed `/__origin/<host>/` prefix are refused with empty HTTP `400 Bad Request` before reaching the limiter.
- **`Service::Director`**:
  - The Director does not install `CreateRequestRateLimiter`, so it does not apply the per-minute request budget.

## 4. Memory Bound and Secret Redaction

- **Bounded In-Memory State (`crates/isles-core/src/areas/auth/runtime.rs`)**:
  - Both the fixed-window request-budget map and the sliding-window `IsRateLimited` map inside `RateLimiter` sweep expired entries every 60 seconds (`SWEEP_INTERVAL_MS = 60_000`) and cap tracked keys at `MAX_RATE_LIMIT_KEYS = 16_384`.
  - If a simultaneous flood of distinct addresses or `/56` subnets exceeds `16_384` live keys within a single window after sweeping expired keys, the entry with the earliest expiration timestamp is evicted before inserting the new key.
- **No Sensitive Values Logged or Exposed**:
  - Raw `x-mysticparadox-gameserver-apikey` values, `x-mystic-host-capability` secrets, and full client IP addresses are never logged by the rate limiter.
  - The `RateLimit-Policy` `pk` parameter exposes only the base64 encoding of the first 12 lowercase hex characters of `SHA-256(key)`.
