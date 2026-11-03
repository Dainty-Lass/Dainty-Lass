<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: Rustic Rifts' co-op transport (RD27)

The checklist for `crates/rifts-transport`, the co-op transport the launcher links in process (umbrella D1): ICE links
between a host and its joiners, the sealed frames on them, admission over the link, the host's backend calls, the
Guard pipe from the hosted game server and the UDP bridges the game sees. Written with the clean-room MIT rewrite from
`docs/port/spec-rd27-rifts-transport.md` (and the umbrella's D3 renames). `SECURITY_REVIEW.md` indexes it.

How to use it: before the launcher switches to this crate, copy this list into the cutover commit, check each item
in the code (not from memory), link the code or test that shows it, and sign at the end with the date and the
commit reviewed.

## What it is and where the boundary is

- **In, from a remote peer (untrusted):** UDP datagrams on the link sockets and through TURN allocations: STUN checks
  (the agent's), sealed `RX` frames, fragments, `RPC1` admission frames carrying `rpt1.` tickets.
- **In, from the signalling path (untrusted text the launcher relays):** `DESCRIPTION`, `CANDIDATE` and
  `GATHERING_DONE` signals.
- **In, from the launcher (trusted, but files on disk):** `bridge.conf`, `transport.secret`, the capability and ticket
  files, the raw X‖Y admission key; roster commands with pair keys.
- **In, from the backend:** answers to consume, host-ready, lease, Guard batch and completion calls.
- **In, from the hosted game server:** `RGE1` messages on the Guard named pipe.
- **In, from local processes:** datagrams on the 127.0.0.1 bridges.
- **Out:** sealed frames, signals, HTTPS calls carrying the host capability, datagrams to the game, ready, event and
  diagnostic lines.

Fuzz targets in `fuzz`: `transport_frame` (sealed-frame open and fragment reassembly), `transport_files` (the
key=value parser, ICE material, the relay-only filter), `transport_pipe` (the `RGE1` parser). Tickets and `RPC1`
frames are `rifts-admission`'s parsers, fuzzed by `admission_ticket` and `admission_control`.

## Checklist

### Frames and admission

- [ ] Every datagram from a peer is opened with AES-256-GCM over the 12-byte header as associated data; no frame is
  acted on before its tag verifies (`seal.rs`).
- [ ] Nonces never repeat for a key: per-direction prefixes, a random 63-bit counter start per cipher, sealing stops at
  `u64::MAX` (`seal.rs`, test `crypto`).
- [ ] The replay window refuses a counter seen before and one 1024 or more below the highest, before decrypting.
- [ ] Gameplay from a peer that is not admitted is dropped and counted; only admitted peers are attached to the router.
- [ ] A ticket is checked for signature, lifetime, key, session, generation, party, hunt, joiner, host, build and
  protocol, then the account must be the roster's account for that pair key, then the backend consumes it once.
- [ ] A jti admitted once is refused locally (`local_replay`), and a consume answered after the link went is undone
  with a disconnect (test `late_consume`).
- [ ] High-S signatures verify, so the transport and the issuer agree (test `high_s_signatures_pass`).
- [ ] A joiner that does not finish admission in 60 s is refused; the host consumes at most once per link.

### Signals and ICE

- [ ] Relay-only links offer the agent only relay candidates, send only relay candidate lines and pass on no remote
  non-relay candidate; relay candidates carry no local address in `raddr`.
- [ ] The host refuses admission over a non-relayed pair when relay-only (`transport_not_relayed`); the joiner fails
  the same way at connection.
- [ ] Held signals from accounts not on the roster are bounded (64 accounts, 16 each, 20 s).
- [ ] Signal data is bounded to 4096 printable bytes at the API and in the link.

### Files and secrets

- [ ] Input files must be regular, non-symlink files within their size limits; unknown keys are ignored, repeated keys
  refused.
- [ ] No line the transport writes contains a ticket, capability, key, credential, packet content or address
  (ready, event, diagnostic and stats lines).
- [ ] The ICE crates log through `log` at debug and warn level with addresses and, at debug, the remote ICE password.
  The transport does not touch the process's logger, so the launcher's subscriber must turn the `rtc_ice`, `rtc_turn`,
  `rtc_stun` and `rtc_mdns` targets off (the C++ turned libjuice's logging off at `Initialize`).
- [ ] The capability is sent only to the configured backend, over HTTPS except to `127.0.0.1`/`localhost`; the lax
  certificate check applies to those two hosts only.
- [ ] Backend answers are read up to 16 KiB and matched by substring, never parsed or retried.

### Guard pipe (Windows)

- [ ] **Known gap:** the pipe has Windows' default security descriptor and the client's process id is not checked,
  because the safe tokio API offers neither (provenance record §6.4). Decide before shipping whether to add the
  SDDL and `GetNamedPipeClientProcessId` check behind a scoped `unsafe` allowance.
- [ ] The pipe is inbound only, message mode, one instance, remote clients refused; messages over 8 KiB and malformed
  ones are dropped and counted, and at most 256 wait.
- [ ] Pipe events are only queued as Guard evidence; a refused batch ends the hunt's evidence rather than altering it.

### Bridges

- [ ] Every bridge socket is bound to 127.0.0.1; the first local sender is pinned, others are dropped as foreign; the
  host's local client is re-pinned only after 2 s of silence and on a fresh server-side port.
- [ ] Datagrams over 2048 bytes are dropped and counted; one socket per joiner, at most `maximum_peers`.

### Failures

- [ ] A panic in a session run is caught and reported as `SESSION_FAILURE code=exception`, result 3.
- [ ] The session thread never waits on the network: backend calls go through bounded lanes; a full lane refuses.

Reviewed by: ______________________ Date: __________ Commit: __________
