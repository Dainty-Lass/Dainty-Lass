# NOTICE

Rustic Rifts is licensed as a whole under the GNU Affero General Public License, version 3 only
(`AGPL-3.0-only`); see [LICENSE](LICENSE). Additional terms under AGPLv3 section 7 apply to the Mystic Paradox
material; see [ADDITIONAL_TERMS.md](ADDITIONAL_TERMS.md). This file records where each part of the repository
comes from.

## Modification notice

This repository, `silentspud/rustic-rifts` on `git.silentspud.com`, is a **modified, unofficial version** of
Mystic Paradox's co-op transport. It is **not** an official release of the Mystic Paradox project, and Mystic
Paradox does not endorse it. Every change is recorded in the commit history.

## Where the parts come from

### Mystic Paradox: ParadoxTransport and realms

- By Pranav Karande (MysticFox). Repository:
  [github.com/pranav158/Mystic-Paradox](https://github.com/pranav158/Mystic-Paradox), AGPL-3.0-only with
  additional terms. Mystic Paradox is itself a modified version of Undaunted by gwog :3 (SyST3MDeV),
  [github.com/SyST3MDeV/Undaunted](https://github.com/SyST3MDeV/Undaunted), AGPL-3.0-only.
- **"Mystic Paradox's Dauntless 1.12.0 and 1.14.7 ports and related modifications were developed by
  MysticFox / Pranav Karande. See NOTICE.md for contribution and provenance information."**
- The repository root is Mystic Paradox's `ParadoxTransport/` folder, taken with its history (up to `c1c5dca`,
  17 October 2026): the transport (`src/`), its self-test (`src/test/`), the build and dependency scripts and
  `tools/`. Its files keep Mystic Paradox's copyright headers.
- The Rust workspace (`crates/rustic-rifts-seam`, `crates/rustic-rifts-admission`, `crates/rustic-rifts-relay`, `crates/rustic-rifts-realms`,
  `crates/rustic-rifts-guard`, `crates/rustic-rifts-host`) ported Mystic Paradox's `p2p/`, `realms/`, Guard and launcher realm-hosting
  modules (`21b9ff5`, `AGPL-3.0-only`) to Rust on `store`. Those crates were removed on 2026-11-05 (RD27): Dainty Lass
  now builds dainty-lass's clean-room MIT `rustic-rifts-*` crates in their place, which contain none of this code. The
  removed crates stay in the history, under `AGPL-3.0-only` and Mystic Paradox's additional terms.
- `oracle/tests/vectors/` (`node-admission.json`, `node-relay.json` and their generator `make-vectors.mts`) are data
  recorded from Mystic Paradox's own admission, relay and ICE modules, and `oracle/tests/*.rs` the ported tests that
  replay them; both stay under `AGPL-3.0-only` and Mystic Paradox's additional terms.
- `LICENSE` and `ADDITIONAL_TERMS.md` are copied unchanged from Mystic Paradox.

### libjuice

- Copyright (c) 2020-2022 Paul-Louis Ageneau. Mozilla Public License 2.0 (`MPL-2.0`),
  [mozilla.org/MPL/2.0](https://mozilla.org/MPL/2.0/).
- Source: [github.com/paullouisageneau/libjuice](https://github.com/paullouisageneau/libjuice), tag `v1.7.4`.
- Not stored in this repository. `prepare-dependencies.ps1` downloads the tag's source archive, checks its
  SHA-256 (`02CCA69649A3163CE27BD59D9E04A5B69E39EB88BBE6BE89FC5F03B2D9A178FD`) and unpacks it into
  `third_party/`; `libjuice.vcxproj` builds it unmodified as a static library linked into `ParadoxTransport.dll`
  and `ParadoxTransportTest.exe`. Its source code is available at the link above, and anyone who distributes
  those binaries passes this notice on with them (MPL-2.0 section 3.2).
