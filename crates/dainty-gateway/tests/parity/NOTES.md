# Gateway parity: where the Rust answer departs from Node on purpose

`gateway.expected.json` was recorded from Reforged's Node gateway. The Rust gateway answers every recorded request
the same. Where it now answers a request Node never saw differently, the departure is listed here, with why; a
recording is never edited to hide one.

## The realm routes (`fix/gateway-realm-routes`, 2026-11-12)

Node refused every path under `/launcher` from off the machine as 403 `{"error":"forbidden"}` (`host_only_route`),
realms included. The recorded `POST /launcher/v1/auth/register` still is, and stays so.

A launcher reaches the server only through the gateway, so with that rule no player could host or join a realm. The
Rust gateway lets `/launcher/v1/realms`, `/launcher/v1/realm-hosting` and `/launcher/v1/p2p` through, `GET` and
`POST` only, with exactly one plausible `x-dainty-user-api-key` and no `Authorization`. Without the key the answer is
401 `{"error":"unauthorized"}` (`no_account_key`); with a bearer, 403 (`realm_bearer`); a wrong method 405; an
upgrade 400. Where Node answered 403 for these, Rust answers one of those or the backend's own reply.

No realm request was added to the recording, since Node has no answer to compare it to; the policy is covered by
`realm_routes_cross_with_the_account_key_only` (`src/policy.rs`, `tests/gateway.rs`) and the `gateway_policy` fuzz
target. The checklist is `docs/security/gateway.md`.
