# Clean-room Provenance Record: Docker at the root (`compose.yaml`, `docker/`)

## 1. Specification

- **Spec file**: `docs/port/spec-docker.md`, `8c69adbe` on `plan`.
- **Spec writer**: the session that read `restricted/docker/`, `restricted/compose.yaml`, `restricted/compose.tailscale.yaml`
  and `restricted/.env.example`. It wrote none of the root files.
- **Replaced files**: those four, all AGPL. They stay until the integration pass moves CI to the root (section 6).

## 2. Implementer isolation

- **Implementer**: a fresh session given the spec and the root's MIT crates. It reported that it never opened
  anything under `restricted/`. The data files of spec 2.4 (and the AGPL licence and notice files beside them) are
  named by path in `docker/Dockerfile` and copied into the image at build time, never read.
- **Pre-invocation removal**: none. Isolation rested on the instructions.
- **Slip**: while checking the server image, the implementer printed the first 60 bytes of `version_data.json` (Mystic
  Paradox's AGPL version data, D-10) into its session output. Nothing from it reached a file or a commit, and the
  start of a JSON data file carries no implementation. It is recorded here because the brief barred reading
  `restricted/`, and that file is under `restricted/`.

## 3. Commits

- Spec: `8c69adbe`. Implementation: `754f6a06`, then `5b66a0c8` (fixes found running it on Docker Desktop), on
  `feat/rd27-docker-mit`, merged as `1aa1d05d`. Integration: `a9bb2a57`.

## 4. Review findings

### 4.1 Similarity

Measured as in `similarity.md`: runs of 3 or more identical non-blank, non-comment lines, with `#` as the comment
prefix, against restricted's file of the same name.

| File | Matched / code lines | Similarity |
| --- | --- | --- |
| `compose.yaml` | 493 / 706 | 69.8 % |
| `compose.tailscale.yaml` | 21 / 24 | 87.5 % |
| `.env.example` | 8 / 8 | 100 % |
| `docker/host-entrypoint.sh` | 37 / 63 | 58.7 % |
| `docker/Dockerfile` | 42 / 130 | 32.3 % |
| `docker/smoke-test.sh` | 32 / 151 | 21.2 % |
| `docker/test-in-image.sh` | 8 / 38 | 21.1 % |
| `docker/network/netem.sh` | 13 / 69 | 18.8 % |
| `docker/director-entrypoint.sh` | 4 / 34 | 11.8 % |
| `docker/network/init.sh` | 9 / 84 | 10.7 % |
| `docker/remote/test.sh` | 11 / 169 | 6.5 % |
| `docker/network/test.sh` | 15 / 371 | 4.0 % |
| `docker/healthcheck.sh`, `docker/run-tests.sh` | 0 | 0 % |

The high numbers come from text the spec fixes, which is what a compose file is. Every long run in `compose.yaml` is a
service's image, profile, command, health check numbers, `depends_on`, capabilities or ports, as spec 3.2-3.9 tabulates
them, in the spec's order. That includes the MongoDB `rs.initiate` check and the Postgres entrypoint's last line,
which the spec quotes. `compose.tailscale.yaml` is spec 3.9 line by line. The eight live lines of `.env.example` are
spec 4's defaults. The matched runs in `host-entrypoint.sh` are the messages and flag handling spec 5.2 quotes. The
`Dockerfile`'s matched runs are the package lists, the `setcap` line, the `dainty` user and the cache mounts spec 2.2
and 2.3 give. The scripts with logic of their own (the harnesses, the health check, `init.sh`) share almost nothing.
Verdict: independent, with the declarative files as close as their spec makes them.

A provenance grep finds Mystic Paradox, Paradox and Reforged only in paths to restricted's data (`ParadoxBackend/...`,
spec 2.4), the version data comment, and the `reforged` MongoDB database name the `[RB15]` smoke steps seed.

### 4.2 The network harness's step 12 against restricted's

The spec's 6.4 step 12 left three things for the implementer to work out from the MIT crates. Against restricted's
`docker/network/test.sh`:

- **An instance is the candidate's `gameSessionId`.** Restricted reads the instance id the same way, from
  `/candidate/status` once `IN_PROGRESS`. Same.
- **Hunt rooms are `Hunt-<ULID>.<node>`, joined with the display name as `name`.** Restricted joins a fixed literal
  room, `Hunt-01J9Z3N5C8Q0V2X4Y6W8T0R2P4.oregon-1`, with `name` set to the display name, and does not tie it to a real
  hunt. The root derives the room from DN8's session (`hunt:<ULID>@<node>` to `Hunt-<ULID>.<node>`), which follows
  the same shape and also follows whichever node hosts the hunt. Equivalent; the root's is the stricter test. The spec
  should state the room name's shape.
- **The uk session ticket.** Same construction: a third player registers on uk and starts a hunt, germany's player
  joins that instance through germany (now `redirect`), and the redirect's `x-dainty-session-ticket` is the ticket.
  The last hop differed. Restricted logs into chat on uk with the ticket itself as the token, as the spec words it.
  The root first traded the ticket for a `sessiontoken` at `PUT /gamesession/epiceos` on uk and logged in with that.
  That passed, but it never exercised chat's ticket login (`rustic-isles-chat`'s `ticket_login`), the path a foreign
  node's chat takes. A probe run showed the raw ticket logs in on the root too, so this review changed the harness
  to restricted's form.

Other differences, none wrong: restricted queries `__chat/history/*` and `__chat/mute` by the `x-dainty-account-id`
value, the root by `UserId`. The root's form passes; restricted's was not tried on the root. Restricted's spacing `sleep`s are replaced by the
root's `drain_for` waits.

### 4.3 Germany's region report answered 401 after its gateway restarted

The implementer saw this and dropped the request from step 12. **It is real.** A probe run on the root's images
showed `POST /network/v1/regions` with germany's player's key answering **401** after `compose restart
gateway-germany`, then **204** once the same key had called `GET /dainty/api/GetUserInfo`.

The cause: the ring authenticates a region report against an in-memory map from the key's SHA-256 to the account
(`RingNode::local_account_keys`, `crates/dainty-network/src/ring.rs`). Only the module fills that map, when a keyed
launcher call announces the key: register, `GetUserInfo`, a keyed `ServerStatus`, or the token exchange
(`rustic-isles-launcher-api`'s `announce`, `rustic-isles-auth`). A restart empties it. The store keeps only a keyed
HMAC of each key, which the ring can't compute, so nothing can refill the map at start. A report that comes before any
other keyed call is refused. The harness polls `ServerStatus` without a key, so in the harness that is every report.

**Fixed here, on the launcher's side** (`crates/dainty-launcher-kit/src/net/regionping.rs`). When the home answers a
report with 401, the launcher sends `GetUserInfo` with the key, which announces it again, and retries the report once.
Test: `a_restarted_home_that_forgot_the_key_gets_it_again_and_takes_the_order`. A home that forgot the key answers
the report 401 until the key has been to the user-info route. The test checks the order is reported, and that the
calls go nodes list, report, user info, report, all keyed except the first.

**Not fixed: the server side.** Any other client of the route still gets 401 until it makes a keyed call. The proper
fix has the ring ask the module who holds a key it doesn't know, a new network-to-module call (a `HomeBackend`
operation or a `GameCall`). That changes the module contract in `dainty-module-api`, `dainty-network` and
`rustic-isles`, a seam beyond this review, so it is reported, not made.

### 4.4 Other observations

- `compose.yaml` keeps the `[RB15]` MongoDB import steps (Q-6), as the spec asks.
- `docs/port/spec-docker.md` section 8's oracle runs 1 and 2 (new images under restricted's compose and harness) were
  not run. Run 2 needs restricted's three scripts to take `COMPOSE_FILE` from the environment (Call D-8). Run 3 was run
  in part (section 5).

## 5. Verification

- The implementer: built the four images and ran `docker/run-tests.sh` and the network harness on Docker Desktop.
- This review, on Docker Desktop (Windows) with the images built from lane D's branch:
  `NETWORK_NO_BUILD=1 NETWORK_REGION_PING=skip bash docker/network/test.sh` passed as merged, and passed again after
  section 4.2's change. The region ping step is skipped on Docker Desktop (the nodes' addresses are not reachable from
  the host). A probe copy of the harness confirmed 4.3's 401 and the raw-ticket chat login.
- `cargo test -p dainty-launcher-kit --lib regionping`: 14 passed, 1 ignored (the harness's test).
- Not run: `docker/run-tests.sh` and the smoke test against current `plan` images, the oracle runs 1 and 2, and
  anything on a Linux host with `CAP_NET_ADMIN`. The smoke test plays no game: nothing here checks Wine or a game
  server.

## 6. What in `restricted/` can go

**Can be deleted now**, in the same change that points CI's Docker and Network families at the root (the lines are
in lane D's hand-over and in this review's report):

- `restricted/compose.yaml`, `restricted/compose.tailscale.yaml`, `restricted/.env.example`. Restricted's compose has
  no rollback profile any more (`restricted/docs/canary.md`: "Docker's `rollback` profile is gone"), so nothing
  Node-related lives in them.
- `restricted/docker/smoke-test.sh`, `run-tests.sh`, `test-in-image.sh`, `network/netem.sh`, `network/test.sh`,
  `remote/test.sh`.
- In `restricted/docker/Dockerfile`, the `front`, `netem` and `test` stages.

**Stays until RB15**: the Node rollback stages in `restricted/docker/Dockerfile` (`node-build`, `mongo-tools`,
`server-rollback`). `server-rollback` copies from `rust-sources`/`rust-build` and takes `director-entrypoint.sh`,
`host-entrypoint.sh`, `healthcheck.sh` and `network/init.sh` from `restricted/docker/`. Either keep those four scripts
and the two Rust stages until RB15, or point the stage at the root's `docker/` copies (MIT, fine in an AGPL image)
and the root's build, then delete them now. The `server` stage can go once nothing builds it: the root's `server`
image replaces it.

Docs that still describe restricted's Docker and move with the deletion: `restricted/docs/hosting-docker.md` (its
services table still lists the removed rollback profile), the root `AGENTS.md` Docker paragraph and firewall-test
line, `restricted/AGENTS.md`'s `docker/test-in-image.sh` line, and `.github/workflows/ci.yml`'s image scan of
`restricted/compose.yaml`.
