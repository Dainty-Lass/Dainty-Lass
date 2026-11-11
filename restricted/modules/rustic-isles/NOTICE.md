# NOTICE

This product includes software developed by multiple parties. It is licensed
as a whole under the GNU Affero General Public License v3.0 (`AGPL-3.0-only`),
with additional terms under AGPLv3 Section 7 applying to the Mystic Paradox
contributions only (see `ADDITIONAL_TERMS.md`).

## Modification notice

This repository (`silentspud/rustic-isles` on `git.silentspud.com`) is a **modified, unofficial version** of Mystic
Paradox (`github.com/pranav158/Mystic-Paradox`, up to `21b9ff5`, with tag `dauntless-1.12.0` at `4d5dfaa`), maintained as
the Dauntless 1.14.7 game module of
[Dainty Lass](https://git.silentspud.com/silentspud/dainty-lass). It is **not** an official release of the Mystic Paradox
project, and Mystic Paradox does not endorse it. Every change is recorded in the commit history.

> "Mystic Paradox's Dauntless 1.12.0 and 1.14.7 ports and related modifications were developed by MysticFox / Pranav
> Karande. See NOTICE.md for contribution and provenance information."

Every crate and file in this repository (`crates/rustic-isles-plugin`, `crates/rustic-isles-mit-parity` with the goldens
recorded from the Node reference, `version-data/`, `reference/`, `native/` and `preset/`) is licensed under
`AGPL-3.0-only` ([LICENSE](LICENSE)) with Mystic Paradox's section 7 additional terms
([ADDITIONAL_TERMS.md](ADDITIONAL_TERMS.md)). The Rust port of the backend and Director (`isles-core`, `isles-director`,
`isles-chat`, `isles-db`, `isles-1-14-7`) was replaced by Dainty Lass's clean-room MIT crates and removed (RD27); it
stays in the commit history under the same terms. Mystic Paradox's own modification and provenance notices follow
below.

Mystic Paradox is a modified version of Undaunted (AGPLv3 §5).

Development of the Mystic Paradox modifications began on 5 July 2026. This public
repository is a sanitized and reorganized source release of work developed since
5 July 2026.

- Original project: Undaunted by gwog :3 / Gregory Morford (SyST3MDeV)
- Modified work: Mystic Paradox by Pranav Karande

## Provenance and Attribution

### Original work — Undaunted
- Copyright (C) 2026 gwog :3 (SyST3MDeV)
- Upstream: https://github.com/SyST3MDeV/Undaunted
- Licensed under AGPL-3.0-only.

The original Undaunted project provided the initial private-server implementation
(reverse-engineered against the `1.4.4_shipping` build). Its copyright notices are
retained in the headers of the source files it originated, as required by the AGPL.

### Modified work — Mystic Paradox
- Copyright (C) 2026 MysticFox / Pranav Karande
- Repository: https://github.com/pranav158/Mystic-Paradox

Mystic Paradox is a modified and substantially extended derivative. Work by
Pranav Karande includes, among other things:

- Port of the backend and injected DLL from `1.4.4` to the `1.12.0` (UE 4.26.2,
  changelist 392819) build, including hook re-offsetting and protocol updates.
- MongoDB persistence layer (migration off SQLite) and repository contracts.
- In-process HTTPS redirect / TLS termination replacing the previous nginx proxy.
- Launcher authentication, Discord OAuth, and signed launcher/runtime update
  distribution.
- Realtime XMPP presence/chat services.
- Progression, inventory, store, matchmaking, party, and guild systems for 1.12.0.

### Runtime loader

The runtime loader (Mystic Paradox's `tools/RuntimeLoader`, derived from
[coldloader-proxy](https://github.com/denuvosanctuary/coldloader-proxy) under the Apache License 2.0) was carried in
`restricted/native/runtime-loader` until 2026-11-11. Dainty Lass replaced it with an MIT clean-room rewrite at the
monorepo root's `native/runtime-loader`, and the original stays in the history only.

## Notes

- This repository does not distribute the game client, packaged game assets, generated SDK
  headers, or bulk extracted game-data tables. It does contain interoperability information
  (protocol/endpoint names, catalog identifiers, class names, engine hook offsets) as any
  compatibility layer must. See `README.md` and `docs/` for generating the SDK and game data
  from your own installation.
- Credentials, certificates, and private keys have been removed. Configuration is
  provided as `.env.example` templates only.
