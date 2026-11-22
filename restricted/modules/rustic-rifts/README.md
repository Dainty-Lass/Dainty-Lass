# Rustic Rifts: the AGPL remainder

The co-op transport is MIT code at the root: `crates/rustic-rifts-transport`, linked into the launcher through
`rustic-rifts-host`'s `in-process` feature (umbrella spec D1, `docs/port/spec-rd27-rifts.md`). Its C++ predecessor,
Mystic Paradox's `ParadoxTransport.dll`, was deleted at the cutover (`docs/port/spec-paradox-transport.md`, C4); it
stays in the history. So does `oracle/`, Mystic Paradox's Node answers replayed against the MIT
`rustic-rifts-admission` and `rustic-rifts-relay`, deleted at RB15. What is left here:

- `run-relay-selftest.ps1`: two ICE links on this PC, relay only, through a real relay (below).
- `tools/mint-relay-test-file.cjs`: mints Cloudflare TURN credentials for that self-test.
- `docs/`: the RF1 inventory and the AGPL security checklists.

## The relay self-test

```powershell
.\run-relay-selftest.ps1 -EnvFile <backend .env>                    # Cloudflare TURN (P2P_CLOUDFLARE_TURN_*)
.\run-relay-selftest.ps1 -EnvFile <backend .env> -Provider turn-rest  # the server's own relay (P2P_TURN_REST_*)
```

It mints 10-minute credentials into a file, runs `rustic-rifts-transport`'s ignored `relay_file` test against it
(`RIFTS_RELAY_FILE`) from the repository root and always deletes the file. It prints no secret. Needs Node for
Cloudflare and the Rust toolchain.

## Licence

AGPL-3.0-only ([LICENSE](LICENSE)), with Mystic Paradox's additional terms ([ADDITIONAL_TERMS.md](ADDITIONAL_TERMS.md)).
This is a modified, unofficial version of Mystic Paradox's co-op transport, not an official Mystic Paradox release.
"Mystic Paradox's Dauntless 1.12.0 and 1.14.7 ports and related modifications were developed by MysticFox / Pranav
Karande. See NOTICE.md for contribution and provenance information."
