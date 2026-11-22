<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: admission tickets (RX3)

> **Retired** (RD27): this checklist is for the AGPL `rustic-rifts-*` crate, which was replaced by the root's MIT
> clean-room crate and removed. Kept for reference; the root's `docs/security/rifts-*.md` is the live checklist.

The checklist for `crates/rustic-rifts-admission` (Dainty Lass plan RF4, DD14, DX2): the MPT2 admission ticket, the P-256
key pair and its generator, the MPC1 control frames and the admission routes (self ticket, join bootstrap,
consume, disconnect, the hub's peer list).

How to use it: before the 1.14.7 module serves realms from Rust, copy this list into the cutover notes, check each
item in the code (not from memory), link the code or test that shows it, and sign at the end with the date and the
commit reviewed. An item that does not hold is a bug to fix before the cutover, not a box to leave open.

## What it is and where the boundary is

A ticket is the only thing that lets a joiner into a hosted hunt or hub. The backend signs it for one account, one
peer id, one host, one session generation and at most 180 seconds; the host's transport checks the signature
against `p2p-admission-public.xy` and asks the backend to consume it, once. Whoever can mint a ticket, replay one or
get one accepted for another binding joins a game they were not admitted to, and acts there as another account.

- **In, launcher:** `POST /mystic/v1/hunts/:sessionId/tickets/self`, `/launcher/v1/p2p/hunts/:sessionId/join-bootstrap`
  and `.../peers`, with an account isles authenticated. The session id is the route's; nothing in the body is
  trusted beyond the hub's `generation`.
- **In, host runtime:** `POST /mystic/v1/hunts/:sessionId/tickets/consume` and `.../peers/:admissionId/disconnect`,
  with a host capability isles checked (`HostCapability`). The ticket text, the authenticated peer id, the bridge
  instance and the connection handle come from the host's PC, which the host controls.
- **In, peers:** MPC1 frames (`control.rs`), which reach the transport inside sealed control frames; the Rust codec
  is the transport's, for `rustic-rifts-host` (RF7).
- **Secret:** `P2P_ADMISSION_SIGNING_PRIVKEY_B64` in `secrets.env`.
- **Out:** the ticket text (a bearer credential until consumed) to the joiner's launcher, the store
  (`AdmissionStore`, the jti's digest only), realms' hunt and presence records (`AdmissionHunts`).

Code: `ticket.rs` (encode, decode, sign, verify), `keys.rs` (key loading, the generator, the key set on disk),
`control.rs`, `service.rs` (the five operations), `store.rs`.

Fuzz targets in `fuzz/`: `admission_ticket` (token text and raw payloads; only the canonical payload for its claims
decodes, accepted claims are in the transport's ranges, a verification agrees with the unverified parse) and
`admission_control` (MPC1 frames re-encode to themselves, an admission carries a 64-byte id).

## Checklist

### The ticket

- [ ] The payload is byte for byte Node's and the transport's: the oracle's vectors (deleted at RB15) replayed 349
      verdicts of Mystic's `admissionTickets.ts`, message for message, and re-encodes every ticket Node issued to the same
      bytes. The transport's own verifier and `ParadoxTransport.dll`'s joiner accept Rust-minted tickets (RF4's
      harness run, recorded in the commit report).
- [ ] Only canonical encodings pass: unpadded base64url that re-encodes to itself, exactly three parts, prefix
      `mpt1`, a 64-byte P1363 signature, at most 4096 bytes (`only_canonical_encodings_pass`).
- [ ] The signature is checked before the payload is decoded, and r or s out of range is a bad signature, as
      OpenSSL answers. A high-S signature passes, as it does for Node and BCrypt (malleability does not matter: the
      jti, not the signature, is what is consumed).
- [ ] The decoder re-encodes what it read and refuses any difference: lengths that do not add up, a non-zero
      reserved field, identifiers outside their character sets, a virtual port outside 1 to 999, a zero peer id, a
      session that is not a version 1 to 5 UUID, a party revision past 2^53 - 1, a time with no exact double
      (`the_payload_framing_refuses_anything_noncanonical`).
- [ ] The key id must be the active one; the lifetime at most 180 seconds with `not_before` and `expires_at` not
      before `issued_at`; 30 seconds of skew either side (`only_the_active_key_and_a_sane_lifetime_pass`).
- [ ] The jti is 16 bytes from the system's random source, and only its SHA-256 is stored.

### Single use and the bindings

- [ ] Consume checks, in Node's order: the body's types and lengths, the signature and clock, the ticket's session
      and joiner against the route and the authenticated peer, the capability's generation, host peer and bridge
      instance, the emergency stop, then the hunt's generation, host peer, bridge instance and a ticketable state.
- [ ] The store consumes by compare-and-set on the ticket's status and answers Node's outcomes: the same request
      from the same transport is idempotent, anything else after a consume is a replay, an unknown or revoked jti is
      invalid (`a_ticket_is_consumed_once_then_idempotent_for_its_request_and_a_replay_for_anyone_else`).
- [ ] One live admission per peer and session: the active key is unique while connected; a hub member coming back
      supersedes its old admission, a hunt participant does not.
- [ ] A disconnect is accepted only from the generation and bridge instance that admitted the peer, once
      (`one_live_admission_per_peer_and_disconnects_by_the_reporting_host_only`).
- [ ] A self ticket needs the account on the frozen roster (or an open hub entry), a verified peer link whose peer
      matches, and realms' reservation within the rejoin window and reconnect limit.
- [ ] The hub's peer list goes to the hub's host only, for the current generation, at most 64 live members, 3600
      lists an hour per generation.

### Keys and secrets

- [ ] The signing key is read only from `P2P_ADMISSION_SIGNING_PRIVKEY_B64` and never written to a log, a `Debug`
      (`Es256Tickets`' `Debug` redacts it, the PEMs and env lines are `Secret`), a reply or a URL.
- [ ] A missing signing key leaves a verify-only authority; minting then fails at the first ticket, as Node's does.
- [ ] The generator refuses an unsafe key id, a missing parent folder and an existing output folder, writes into a
      temporary folder renamed into place, and removes it on failure; the private files are created owner-only on
      Unix (`the_key_set_is_written_once_and_never_over_an_existing_folder`).
- [ ] The ticket text is a `Secret` everywhere in Rust, and the minting tool prints only its digest and expiry.

## Open points

These do not block RF4 but must be settled, or accepted in writing, before the cutover:

- **The seam's `RuntimeAuth`** carries neither the host peer nor the bridge instance, so consume takes a
  `HostCapability` that isles builds from the capability record. Until RF5 and the seam carry them, the capability
  match leans on that construction.
- **No transaction yet.** Node runs each operation in one Mongo transaction; here the writes run in Node's order
  without one, until `store`'s transaction API (see `rifts_seam`'s store notes). A crash between consuming a ticket
  and recording the admission leaves a consumed ticket with no admission (a replay to the transport, not an
  admission).
- **Node allows a zero generation, build, bridge protocol and party revision; the transport refuses them.** Rust
  follows Node; realms must not issue zeros.
- **Key rotation** is one active key id, as in Node: a rotation fails every ticket in flight for up to 180 seconds.
- **The pair cipher** (`pair_crypto.cpp`, the sealed frames MPC1 travels in) is the transport's and gets its page
  with RF7, which ports the launcher's side.

## Fuzzing

From `fuzz/`, on nightly (on Windows with `clang_rt.asan_dynamic-x86_64.dll`'s folder on PATH):

```bash
cargo +nightly fuzz run admission_ticket -- -max_total_time=86400
cargo +nightly fuzz run admission_control -- -max_total_time=86400
```

## Changes from Node's admission

- Signatures are deterministic (RFC 6979) where OpenSSL's were random; both verify everywhere.
- A ticket whose time Node's `Number` rounds up to 2^64 is refused as noncanonical, where Node threw a `RangeError`;
  both answer `P2P_TICKET_INVALID`.
- `disconnectedAt` from the host is parsed by the route layer rather than stored as text.
- The private key may also be SEC1 (`EC PRIVATE KEY`), which Node's `createPrivateKey` reads too; the public key
  must be SPKI.

Signed: ________ (date, commit)
