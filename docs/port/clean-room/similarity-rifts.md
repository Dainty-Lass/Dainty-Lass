<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# Clean-Room Similarity Report: Rustic Rifts (RD27)

Measured on 2026-11-02 by the spec writer, outside the clean room, on dainty-lass `feat/rd27-rifts-mit` against
dainty-restricted `4efc8b5`'s `modules/rustic-rifts`. **The spec writer is not an independent reviewer**: the
maintainer or a second reviewer should repeat this before the integration pass accepts the crates.

## 1. Method

[`similarity.md`](similarity.md) §1's measure (trimmed, non-blank, non-comment lines inside runs of 3 or more lines
identical to the old code), made **stricter** in one way: each new file is compared with the whole AGPL crate it
replaces at once, not with one counterpart file. `rifts-transport` is compared with the C++ `src/` plus
`rifts-host`, and `rifts-host` with `rifts-host` plus `src/`.

Two figures per file:

- **Raw**: the measure as written.
- **Adjusted**: the same measure after dropping, from both sides, the lines a spec fixes by name: `use` lines,
  attributes, lone closing brackets, struct field and enum variant declarations, `fn` signatures, lines that are a
  string literal alone, `Ok(())` and `None`.

## 2. Result after the restructure pass

The first measurement (`2e06bda`) failed criterion 5: 61 files were at or above 30% raw and 28 after the adjustment;
the whole set was 31.5% raw and 20.2% adjusted. The maintainer chose a second clean-room pass (2026-11-02). Four
implementers restructured the listed files from the percentages alone, never seeing the old code. They kept every
public item and all behaviour, and every test still passes. The pass is recorded in each crate's clean-room page.

Measured again on `feat/rd27-rifts-mit` after the merge: **18.4% raw and 8.9% adjusted** over the whole set (139
files). 16 files remain at or above 30% raw and 3 after the adjustment. Whole-crate figures, migrations excluded:

| Crate | Raw % | Adjusted % |
| --- | ---: | ---: |
| `rifts-admission` | 17.7 | 6.9 |
| `rifts-guard` | 19.4 | 7.4 |
| `rifts-host` | 16.3 | 7.5 |
| `rifts-realms` | 20.0 | 9.6 |
| `rifts-relay` | 19.0 | 6.5 |
| `rifts-seam` | 33.3 | 4.9 |
| `rifts-transport` | 4.6 | 0.1 |

The migrations are still 100%: the specs give their DDL exactly, as a contract. The transport, written from a C++
source, is near 0%.

### 2.1 Files at or above 25% (raw or adjusted)

| File | Raw before → after | Adjusted before → after |
| --- | ---: | ---: |
| `crates/rifts-realms/src/repo/mod.rs` | 29.6 → 66.7 | 0.0 → 0.0 |
| `crates/rifts-host/src/session/mod.rs` | 77.2 → 62.1 | 47.8 → 27.9 |
| `crates/rifts-host/src/transport.rs` | 100.0 → 53.5 | 90.0 → 0.0 |
| `crates/rifts-seam/src/hooks.rs` | 70.4 → 52.6 | 44.1 → 31.9 |
| `crates/rifts-realms/src/repo/hub.rs` | 56.4 → 48.2 | 46.7 → 35.0 |
| `crates/rifts-seam/src/store.rs` | 78.9 → 45.8 | 25.0 → 0.0 |
| `crates/rifts-realms/src/hunt.rs` | 65.0 → 41.3 | 61.0 → 0.0 |
| `crates/rifts-seam/src/http.rs` | 68.4 → 39.3 | 31.2 → 0.0 |
| `crates/rifts-guard/src/store.rs` | 47.0 → 37.5 | 22.4 → 12.6 |
| `crates/rifts-realms/src/repo/hunts.rs` | 59.9 → 37.2 | 49.5 → 31.5 |
| `crates/rifts-realms/src/realm.rs` | 63.0 → 36.7 | 43.1 → 10.7 |
| `crates/rifts-seam/src/error.rs` | 63.4 → 35.9 | 36.4 → 6.7 |
| `crates/rifts-realms/src/repo/realms.rs` | 62.3 → 34.8 | 50.7 → 24.8 |
| `crates/rifts-realms/src/db.rs` | 54.0 → 34.2 | 27.3 → 10.5 |
| `crates/rifts-guard/src/ports.rs` | 45.3 → 32.5 | 18.8 → 4.7 |
| `crates/rifts-realms/src/control.rs` | 64.4 → 30.1 | 35.4 → 0.0 |
| `crates/rifts-host/src/session/backend.rs` | 67.1 → 29.3 | 57.9 → 18.2 |
| `crates/rifts-relay/examples/mint-relay-file.rs` | 36.5 → 28.9 | 29.6 → 17.9 |
| `crates/rifts-relay/src/lib.rs` | 67.2 → 28.1 | 45.7 → 12.9 |
| `crates/rifts-seam/src/dedicated.rs` | 32.9 → 27.7 | 19.3 → 5.1 |
| `crates/rifts-guard/src/errors.rs` | 53.1 → 26.6 | 38.3 → 22.0 |
| `crates/rifts-guard/src/lib.rs` | 42.2 → 26.0 | 29.0 → 13.3 |
| `crates/rifts-relay/src/config.rs` | 31.5 → 25.6 | 12.3 → 4.5 |

## 3. What the remaining runs are

Every file still at or above 30% raw is one of these:

- **Public declarations dainty-restricted compiles against**: `rifts-seam`'s hook traits, routes, store shapes and
  errors; `rifts-host`'s `TransportAbi` and the session records (`PeerEvent`, `HostBootstrapSpec`,
  `CommonBootstrapConfig`, `LaunchEnvironmentSpec`); `rifts-realms`' `hunt.rs` and `realm.rs` records; Guard's
  `store.rs` and `ports.rs` records. Their fields, variants and order are the API.
- **Public repository signatures**, `repo/hub.rs` and `repo/hunts.rs`, where rustfmt puts each parameter on its own
  line and a 7-parameter signature is a 9-line run. These two files are also the only ones above 30% after the
  adjustment, apart from `seam/hooks.rs`, which is all trait declarations.
- `repo/mod.rs` is 9 code lines, so one 6-line run is 67%.

None of the sampled runs in these files is a function body: the bind chains, enum tables and refusal sequences
behind the first report's figures are gone.

## 4. For the maintainer

1. The remaining files are at the limit their public API allows. Lowering them further would mean changing the API
   restricted compiles against, which is a different decision.
2. An independent reviewer should repeat this measurement. The script used is beside the session notes for this row
   (`rifts-similarity.py`); run it with `NEWROOT` set to the dainty-lass checkout and, for the adjusted figure,
   `ADJ=1`.
