# Clean-Room Similarity Report & Specification Audit (F6)

## 1. Measurement Methodology

To verify that every clean-room rewrite in `store`, `protocol`, `proxy`, `service-kit`, `hostkit`, and `launcher-kit` is an independent implementation of its behaviour specification rather than a copy or mechanical edit of the pre-existing AGPL code, we measure **3+-line non-blank, non-comment similarity** against the pre-rewrite baseline commit `b190010`:

1. Strip all blank lines and comment-only lines (`//`, `/*`, `*`, `*/`), and trim leading/trailing horizontal whitespace on each remaining code line.
2. For each file in the rewritten crate (`newLines`) and its pre-rewrite counterpart at `b190010` (`oldLines`), mark every line in `newLines` that belongs to a contiguous run of **3 or more** identical lines appearing anywhere in `oldLines`.
3. Compute `similarity = (matched_lines / total_new_code_lines) * 100%`.

### Runnable Measurement Script (`measure-similarity.mjs`)

Run from the repository root with Node.js:

```bash
node measure-similarity.mjs . b190010 25
```

```javascript
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const repoRoot = process.argv[2] || process.cwd();
const baseRev = process.argv[3] || "b190010";
const showRunsAbove = Number(process.argv[4] || "25");

function codeLines(text) {
  return text
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter(
      (l) =>
        l.length > 0 &&
        !l.startsWith("//") &&
        !l.startsWith("/*") &&
        !l.startsWith("*") &&
        !l.startsWith("*/")
    );
}

function measureSimilarity(oldText, newText) {
  const oldLines = codeLines(oldText);
  const newLines = codeLines(newText);
  if (newLines.length === 0) {
    return { oldCount: oldLines.length, newCount: 0, matched: 0, pct: 0, runs: [] };
  }
  const matchedFlags = new Array(newLines.length).fill(false);
  const runs = [];

  for (let i = 0; i < newLines.length; i++) {
    let bestLen = 0;
    for (let j = 0; j < oldLines.length; j++) {
      let k = 0;
      while (
        i + k < newLines.length &&
        j + k < oldLines.length &&
        newLines[i + k] === oldLines[j + k]
      ) {
        k++;
      }
      if (k > bestLen) bestLen = k;
    }
    if (bestLen >= 3) {
      for (let k = 0; k < bestLen; k++) {
        matchedFlags[i + k] = true;
      }
      runs.push({ start: i, len: bestLen, sample: newLines.slice(i, i + bestLen) });
      i += bestLen - 1;
    }
  }

  const matched = matchedFlags.filter(Boolean).length;
  const pct = (matched / newLines.length) * 100;
  return { oldCount: oldLines.length, newCount: newLines.length, matched, pct, runs };
}

function gitShow(revAndPath) {
  try {
    return execFileSync("git", ["-c", "core.fsmonitor=false", "show", revAndPath], {
      cwd: repoRoot,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    });
  } catch {
    return null;
  }
}

const pairs = [
  // 1. store (7 rewritten files)
  ["crates/store/src/lib.rs", "crates/store/src/lib.rs"],
  ["crates/store/src/adopt.rs", "crates/store/src/adopt.rs"],
  ["crates/store/src/backup.rs", "crates/store/src/backup.rs"],
  ["crates/store/src/drizzle.rs", "crates/store/src/drizzle.rs"],
  ["crates/store/src/readonly.rs", "crates/store/src/readonly.rs"],
  ["crates/store/src/snapshot.rs", "crates/store/src/snapshot.rs"],
  ["crates/store/tests/store.rs", "crates/store/tests/store.rs"],

  // 2. protocol (6 rewritten source files)
  ["crates/protocol/src/lib.rs", "crates/protocol/src/lib.rs"],
  ["crates/protocol/src/account.rs", "crates/protocol/src/account.rs"],
  ["crates/protocol/src/invite.rs", "crates/protocol/src/invite.rs"],
  ["crates/protocol/src/manifest.rs", "crates/protocol/src/manifest.rs"],
  ["crates/protocol/src/status.rs", "crates/protocol/src/status.rs"],
  ["crates/protocol/src/text.rs", "crates/protocol/src/text.rs"],

  // 3. proxy (1 rewritten file)
  ["crates/proxy/src/lib.rs", "crates/proxy/src/lib.rs"],

  // 4. service-kit (12 rewritten source files + 3 rewritten test files)
  ["crates/service-kit/src/lib.rs", "crates/service-kit/src/lib.rs"],
  ["crates/service-kit/src/body.rs", "crates/service-kit/src/body.rs"],
  ["crates/service-kit/src/collate.rs", "crates/service-kit/src/collate.rs"],
  ["crates/service-kit/src/env.rs", "crates/service-kit/src/env.rs"],
  ["crates/service-kit/src/js.rs", "crates/service-kit/src/js.rs"],
  ["crates/service-kit/src/jsjson.rs", "crates/service-kit/src/jsjson.rs"],
  ["crates/service-kit/src/json.rs", "crates/service-kit/src/json.rs"],
  ["crates/service-kit/src/linelog.rs", "crates/service-kit/src/linelog.rs"],
  ["crates/service-kit/src/log.rs", "crates/service-kit/src/log.rs"],
  ["crates/service-kit/src/ratelimit.rs", "crates/service-kit/src/ratelimit.rs"],
  ["crates/service-kit/src/routing.rs", "crates/service-kit/src/routing.rs"],
  ["crates/service-kit/src/serve.rs", "crates/service-kit/src/serve.rs"],
  ["crates/service-kit/tests/body.rs", "crates/service-kit/tests/body.rs"],
  ["crates/service-kit/tests/ratelimit.rs", "crates/service-kit/tests/ratelimit.rs"],
  ["crates/service-kit/tests/routing.rs", "crates/service-kit/tests/routing.rs"],

  // 5. hostkit (33 Category B rewritten files against services/host at b190010)
  ["services/host/src/paths.rs", "crates/hostkit/src/paths.rs"],
  ["services/host/src/envfile.rs", "crates/hostkit/src/envfile.rs"],
  ["services/host/src/hostconfig.rs", "crates/hostkit/src/hostconfig.rs"],
  ["services/host/src/run.rs", "crates/hostkit/src/run.rs"],
  ["services/host/src/services.rs", "crates/hostkit/src/services.rs"],
  ["services/host/src/mongo.rs", "crates/hostkit/src/mongo.rs"],
  ["services/host/src/mongo/wire.rs", "crates/hostkit/src/mongo/wire.rs"],
  ["services/host/src/doctor.rs", "crates/hostkit/src/doctor.rs"],
  ["services/host/src/captures.rs", "crates/hostkit/src/captures.rs"],
  ["services/host/src/gamedata.rs", "crates/hostkit/src/gamedata.rs"],
  ["services/host/src/gamepack.rs", "crates/hostkit/src/gamepack.rs"],
  ["services/host/src/invite.rs", "crates/hostkit/src/invite.rs"],
  ["services/host/src/preset.rs", "crates/hostkit/src/preset.rs"],
  ["services/host/src/progression.rs", "crates/hostkit/src/progression.rs"],
  ["services/host/src/publish.rs", "crates/hostkit/src/publish.rs"],
  ["services/host/src/linux.rs", "crates/hostkit/src/linux.rs"],
  ["services/host/src/selfupdate.rs", "crates/hostkit/src/selfupdate.rs"],
  ["services/host/src/wine.rs", "crates/hostkit/src/wine.rs"],
  ["services/host/src/cli.rs", "crates/hostkit/src/cli.rs"],
  ["services/host/src/commands/doctor.rs", "crates/hostkit/src/commands/doctor.rs"],
  ["services/host/src/commands/gamedata.rs", "crates/hostkit/src/commands/gamedata.rs"],
  ["services/host/src/commands/invite.rs", "crates/hostkit/src/commands/invite.rs"],
  ["services/host/src/commands/multipliers.rs", "crates/hostkit/src/commands/multipliers.rs"],
  ["services/host/src/commands/preset.rs", "crates/hostkit/src/commands/preset.rs"],
  ["services/host/src/commands/publish_runtime.rs", "crates/hostkit/src/commands/publish_runtime.rs"],
  ["services/host/src/commands/setup.rs", "crates/hostkit/src/commands/setup.rs"],
  ["services/host/src/commands/setup/firewall.rs", "crates/hostkit/src/commands/setup/firewall.rs"],
  ["services/host/src/commands/setup/install.rs", "crates/hostkit/src/commands/setup/install.rs"],
  ["services/host/src/commands/setup/options.rs", "crates/hostkit/src/commands/setup/options.rs"],
  ["services/host/src/commands/setup/paradox.rs", "crates/hostkit/src/commands/setup/paradox.rs"],
  ["services/host/src/commands/start.rs", "crates/hostkit/src/commands/start.rs"],
  ["services/host/src/commands/start/control.rs", "crates/hostkit/src/commands/start/control.rs"],
  ["services/host/src/commands/update.rs", "crates/hostkit/src/commands/update.rs"],

  // 6. launcher-kit (19 Category B rewritten files against launcher/src-tauri at b190010)
  ["launcher/src-tauri/src/bridge/types.rs", "crates/launcher-kit/src/bridge/types.rs"],
  ["launcher/src-tauri/src/core/pins.rs", "crates/launcher-kit/src/core/pins.rs"],
  ["launcher/src-tauri/src/core/text.rs", "crates/launcher-kit/src/core/text.rs"],
  ["launcher/src-tauri/src/net/hostapi.rs", "crates/launcher-kit/src/net/hostapi.rs"],
  ["launcher/src-tauri/src/net/http.rs", "crates/launcher-kit/src/net/http.rs"],
  ["launcher/src-tauri/src/net/http/to_file.rs", "crates/launcher-kit/src/net/http/to_file.rs"],
  ["launcher/src-tauri/src/net/pinned.rs", "crates/launcher-kit/src/net/pinned.rs"],
  ["launcher/src-tauri/src/net/runtime_files.rs", "crates/launcher-kit/src/net/runtime_files.rs"],
  ["launcher/src-tauri/src/net/runtime_files/manifest.rs", "crates/launcher-kit/src/net/runtime_files/manifest.rs"],
  ["launcher/src-tauri/src/net/runtime_files/tests.rs", "crates/launcher-kit/src/net/runtime_files/tests.rs"],
  ["launcher/src-tauri/src/net/verify.rs", "crates/launcher-kit/src/net/verify.rs"],
  ["launcher/src-tauri/src/net/verify/cache_file.rs", "crates/launcher-kit/src/net/verify/cache_file.rs"],
  ["launcher/src-tauri/src/platform/dialogs.rs", "crates/launcher-kit/src/platform/dialogs.rs"],
  ["launcher/src-tauri/src/platform/hostkit.rs", "crates/launcher-kit/src/platform/hostkit.rs"],
  ["launcher/src-tauri/src/platform/hostkit/parse.rs", "crates/launcher-kit/src/platform/hostkit/parse.rs"],
  ["launcher/src-tauri/src/platform/hostserver.rs", "crates/launcher-kit/src/platform/hostserver.rs"],
  ["launcher/src-tauri/src/platform/linux_runtime.rs", "crates/launcher-kit/src/platform/linux_runtime.rs"],
  ["launcher/src-tauri/src/platform/requirements.rs", "crates/launcher-kit/src/platform/requirements.rs"],
  ["launcher/src-tauri/src/platform/system.rs", "crates/launcher-kit/src/platform/system.rs"],
];

for (const [oldRel, newRel] of pairs) {
  const oldText = gitShow(`${baseRev}:${oldRel}`);
  const fullNew = path.join(repoRoot, newRel);
  if (!oldText || !fs.existsSync(fullNew)) continue;
  const newText = fs.readFileSync(fullNew, "utf8");
  const res = measureSimilarity(oldText, newText);
  console.log(
    `${newRel.padEnd(58)} | old=${String(res.oldCount).padStart(4)} | new=${String(res.newCount).padStart(4)} | matched=${String(res.matched).padStart(4)} | ${res.pct.toFixed(1).padStart(5)}%`
  );
  if (res.pct >= showRunsAbove) {
    for (const r of res.runs) {
      console.log(`    [run len=${r.len}] ${r.sample.join("  ///  ")}`);
    }
  }
}
```

---

## 2. Full Similarity Table Across All Rewritten Files

> **Note on Category A (`Born-MIT`) files:** Files written originally for DaintyLass in Rust with no `Dauntless Revived` (`DR`) or `ReforgedHost` derivation (`store/src/{connection,database,declared,engine,history,portable,recovery,revert,transfer}.rs`, `store/tests/migrate.rs`, `service-kit/src/{context,telemetry}.rs`, the 17 Category A files in `hostkit` listed in `spec-dl3-hostkit.md` §1.1, and `launcher-kit`'s `bridge/spec.rs`, `bridge/modules.rs`, `net/regionping*`, `net/versions*`, `platform/loader_session.rs`) were already MIT-eligible and moved as-is per `spec-dl3-hostkit.md` §1.1 and `spec-dl4-launcher.md` §1.1. The table below measures all **81 rewritten files** (Category B).

### 2.1 `crates/store` (7 files — redone in `R:\source\dauntless\fresh-store-cleanroom`)

| File | Old Lines (`b190010`) | New Lines | Matched (3+-line runs) | Similarity |
| --- | ---: | ---: | ---: | ---: |
| `crates/store/src/lib.rs` | 189 | 201 | 110 | **54.7%** |
| `crates/store/src/adopt.rs` | 84 | 97 | 26 | **26.8%** |
| `crates/store/src/backup.rs` | 53 | 89 | 11 | **12.4%** |
| `crates/store/src/drizzle.rs` | 101 | 108 | 34 | **31.5%** |
| `crates/store/src/readonly.rs` | 66 | 42 | 6 | **14.3%** |
| `crates/store/src/snapshot.rs` | 22 | 32 | 8 | **25.0%** |
| `crates/store/tests/store.rs` | 250 | 373 | 22 | **5.9%** |

### 2.2 `crates/protocol` (6 rewritten source files)

| File | Old Lines (`b190010`) | New Lines | Matched (3+-line runs) | Similarity |
| --- | ---: | ---: | ---: | ---: |
| `crates/protocol/src/lib.rs` | 6 | 6 | 5 | **83.3%** |
| `crates/protocol/src/account.rs` | 128 | 119 | 34 | **28.6%** |
| `crates/protocol/src/invite.rs` | 597 | 593 | 86 | **14.5%** |
| `crates/protocol/src/manifest.rs` | 58 | 49 | 27 | **55.1%** |
| `crates/protocol/src/status.rs` | 368 | 446 | 122 | **27.4%** |
| `crates/protocol/src/text.rs` | 53 | 104 | 10 | **9.6%** |

*(Note: `crates/protocol/tests/{invite,manifest,tables,username_status}.rs` are the JSON table-driven parity test harnesses over `tests/cases/*.json` and `tests/fixtures/*.json` retained to verify 100% wire parity of the rewritten `src/*.rs` modules.)*

### 2.3 `crates/proxy` (1 rewritten file)

| File | Old Lines (`b190010`) | New Lines | Matched (3+-line runs) | Similarity |
| --- | ---: | ---: | ---: | ---: |
| `crates/proxy/src/lib.rs` | 176 | 196 | 43 | **21.9%** |

### 2.4 `crates/service-kit` (12 rewritten source files + 3 rewritten test files)

| File | Old Lines (`b190010`) | New Lines | Matched (3+-line runs) | Similarity |
| --- | ---: | ---: | ---: | ---: |
| `crates/service-kit/src/lib.rs` | 13 | 14 | 13 | **92.9%** |
| `crates/service-kit/src/body.rs` | 227 | 251 | 73 | **29.1%** |
| `crates/service-kit/src/collate.rs` | 70 | 69 | 3 | **4.3%** |
| `crates/service-kit/src/env.rs` | 99 | 120 | 30 | **25.0%** |
| `crates/service-kit/src/js.rs` | 151 | 221 | 40 | **18.1%** |
| `crates/service-kit/src/jsjson.rs` | 184 | 243 | 45 | **18.5%** |
| `crates/service-kit/src/json.rs` | 43 | 45 | 9 | **20.0%** |
| `crates/service-kit/src/linelog.rs` | 137 | 198 | 55 | **27.8%** |
| `crates/service-kit/src/log.rs` | 201 | 209 | 44 | **21.1%** |
| `crates/service-kit/src/ratelimit.rs` | 283 | 271 | 89 | **32.8%** |
| `crates/service-kit/src/routing.rs` | 282 | 280 | 137 | **48.9%** |
| `crates/service-kit/src/serve.rs` | 106 | 115 | 37 | **32.2%** |
| `crates/service-kit/tests/body.rs` | 101 | 138 | 0 | **0.0%** |
| `crates/service-kit/tests/ratelimit.rs` | 66 | 75 | 0 | **0.0%** |
| `crates/service-kit/tests/routing.rs` | 73 | 75 | 0 | **0.0%** |

### 2.5 `crates/hostkit` (33 Category B rewritten files vs `services/host` at `b190010`)

| File | Old Lines (`b190010`) | New Lines | Matched (3+-line runs) | Similarity |
| --- | ---: | ---: | ---: | ---: |
| `crates/hostkit/src/paths.rs` | 277 | 299 | 115 | **38.5%** |
| `crates/hostkit/src/envfile.rs` | 253 | 346 | 36 | **10.4%** |
| `crates/hostkit/src/hostconfig.rs` | 1798 | 2851 | 586 | **20.6%** |
| `crates/hostkit/src/run.rs` | 278 | 222 | 49 | **22.1%** |
| `crates/hostkit/src/services.rs` | 533 | 507 | 75 | **14.8%** |
| `crates/hostkit/src/mongo.rs` | 246 | 584 | 40 | **6.8%** |
| `crates/hostkit/src/mongo/wire.rs` | 305 | 582 | 47 | **8.1%** |
| `crates/hostkit/src/doctor.rs` | 1067 | 1071 | 149 | **13.9%** |
| `crates/hostkit/src/captures.rs` | 188 | 352 | 45 | **12.8%** |
| `crates/hostkit/src/gamedata.rs` | 604 | 586 | 131 | **22.4%** |
| `crates/hostkit/src/gamepack.rs` | 295 | 354 | 107 | **30.2%** |
| `crates/hostkit/src/invite.rs` | 299 | 567 | 49 | **8.6%** |
| `crates/hostkit/src/preset.rs` | 292 | 455 | 95 | **20.9%** |
| `crates/hostkit/src/progression.rs` | 286 | 496 | 63 | **12.7%** |
| `crates/hostkit/src/publish.rs` | 198 | 256 | 67 | **26.2%** |
| `crates/hostkit/src/linux.rs` | 523 | 583 | 80 | **13.7%** |
| `crates/hostkit/src/selfupdate.rs` | 970 | 1426 | 285 | **20.0%** |
| `crates/hostkit/src/wine.rs` | 376 | 397 | 50 | **12.6%** |
| `crates/hostkit/src/cli.rs` | 586 | 652 | 191 | **29.3%** |
| `crates/hostkit/src/commands/doctor.rs` | 36 | 45 | 3 | **6.7%** |
| `crates/hostkit/src/commands/gamedata.rs` | 128 | 139 | 18 | **12.9%** |
| `crates/hostkit/src/commands/invite.rs` | 206 | 253 | 14 | **5.5%** |
| `crates/hostkit/src/commands/multipliers.rs` | 88 | 149 | 9 | **6.0%** |
| `crates/hostkit/src/commands/preset.rs` | 137 | 114 | 11 | **9.6%** |
| `crates/hostkit/src/commands/publish_runtime.rs` | 82 | 69 | 9 | **13.0%** |
| `crates/hostkit/src/commands/setup.rs` | 454 | 353 | 54 | **15.3%** |
| `crates/hostkit/src/commands/setup/firewall.rs` | 77 | 201 | 3 | **1.5%** |
| `crates/hostkit/src/commands/setup/install.rs` | 159 | 143 | 9 | **6.3%** |
| `crates/hostkit/src/commands/setup/options.rs` | 580 | 646 | 48 | **7.4%** |
| `crates/hostkit/src/commands/setup/paradox.rs` | 419 | 193 | 23 | **11.9%** |
| `crates/hostkit/src/commands/start.rs` | 672 | 480 | 64 | **13.3%** |
| `crates/hostkit/src/commands/start/control.rs` | 211 | 261 | 26 | **10.0%** |
| `crates/hostkit/src/commands/update.rs` | 756 | 629 | 78 | **12.4%** |

### 2.6 `crates/launcher-kit` (19 Category B files vs `launcher/src-tauri` at `b190010`)

| File | Old Lines (`b190010`) | New Lines | Matched (3+-line runs) | Similarity |
| --- | ---: | ---: | ---: | ---: |
| `crates/launcher-kit/src/bridge/types.rs` | 2007 | 795 | 719 | **90.4%** |
| `crates/launcher-kit/src/core/pins.rs` | 9 | 60 | 9 | **15.0%** |
| `crates/launcher-kit/src/core/text.rs` | 88 | 170 | 5 | **2.9%** |
| `crates/launcher-kit/src/net/hostapi.rs` | 220 | 50 | 4 | **8.0%** |
| `crates/launcher-kit/src/net/http.rs` | 407 | 227 | 45 | **19.8%** |
| `crates/launcher-kit/src/net/http/to_file.rs` | 382 | 181 | 29 | **16.0%** |
| `crates/launcher-kit/src/net/pinned.rs` | 211 | 230 | 51 | **22.2%** |
| `crates/launcher-kit/src/net/runtime_files.rs` | 468 | 539 | 149 | **27.6%** |
| `crates/launcher-kit/src/net/runtime_files/manifest.rs` | 133 | 86 | 15 | **17.4%** |
| `crates/launcher-kit/src/net/runtime_files/tests.rs` | 673 | 479 | 35 | **7.3%** |
| `crates/launcher-kit/src/net/verify.rs` | 221 | 353 | 85 | **24.1%** |
| `crates/launcher-kit/src/net/verify/cache_file.rs` | 88 | 77 | 14 | **18.2%** |
| `crates/launcher-kit/src/platform/dialogs.rs` | 155 | 53 | 36 | **67.9%** |
| `crates/launcher-kit/src/platform/hostkit.rs` | 799 | 782 | 200 | **25.6%** |
| `crates/launcher-kit/src/platform/hostkit/parse.rs` | 374 | 516 | 37 | **7.2%** |
| `crates/launcher-kit/src/platform/hostserver.rs` | 754 | 841 | 141 | **16.8%** |
| `crates/launcher-kit/src/platform/linux_runtime.rs` | 666 | 528 | 32 | **6.1%** |
| `crates/launcher-kit/src/platform/requirements.rs` | 324 | 311 | 55 | **17.7%** |
| `crates/launcher-kit/src/platform/system.rs` | 171 | 142 | 14 | **9.9%** |

---

## 3. Analysis of Files at or Above ~25% Similarity

Every file at or above 25.0% is listed below with its matching 3+-line runs and why the public API contract, wire schema, CLI flags, or error strings force those lines.

### 3.1 `store/src/lib.rs` (54.7% — 110 / 201 lines)
- **Run 1–2 (29 lines):** `pub mod adopt; pub mod backup; mod connection; ...` and `pub use adopt::{DrizzleBaseline, Schema}; pub use connection::{AnyConnection, MultiBackend}; ...` — forced by the crate's submodule layout (including the 9 untouched Born-MIT submodules in `store/src/`) and public re-export surface (`spec-store.md` §2).
- **Run 3–8 (33 lines):** `Store::open`, `Store::read`, `Store::write`, `Store::checkpoint`, and `pub fn migrate` method signatures and `where F: FnOnce(&mut Connection) -> Result<T, E> + Send + 'static` trait bounds — forced by callers across `admin`, `hostkit`, and `isles`.
- **Run 9 (14 lines):** `pub struct Options { pub read_connections: u32, pub busy_timeout: Duration }` and its `Default` impl (`read_connections: 4`, `busy_timeout: Duration::from_secs(5)`) — forced by `spec-store.md` §2.
- **Run 10 (31 lines):** `#[derive(Debug, thiserror::Error)] pub enum Error` with the 14 exact variant names and `#[error("...")]` display strings — forced because both Born-MIT submodules (`database.rs`, `history.rs`, `revert.rs`, `transfer.rs`) and callers match on `Error` variants and assert on their formatted display strings.

### 3.2 `store/src/drizzle.rs` (31.5% — 34 / 108 lines)
- **Run 1 (8 lines):** `pub struct Migration { pub tag: String, pub created_at: i64, pub sql: String }` — public struct signature (`spec-store.md` §2.4).
- **Run 2–3 (9 lines):** `pub fn hash_matches` and `pub fn last_applied` signatures plus Diesel's `#[diesel(sql_type = Nullable<BigInt>)]` / `#[diesel(sql_type = Text)]` `QueryableByName` annotations for reading `__drizzle_migrations`.
- **Run 4–6 (17 lines):** `CREATE TABLE IF NOT EXISTS "__drizzle_migrations" (id SERIAL PRIMARY KEY, hash text NOT NULL, created_at numeric)` and `.bind::<Text, _>(hash(&migration.sql)).bind::<BigInt, _>(migration.created_at).execute(conn)?;` — forced by Drizzle Kit's exact SQLite migration table DDL and column types (`spec-store.md` §3.4).

### 3.3 `store/src/adopt.rs` (26.8% — 26 / 97 lines)
- **Run 1–3 (15 lines):** Diesel imports (`MigrationSource`, `EmbeddedMigrations`, `MigrationHarness`) and public struct definitions `pub struct Schema { pub migrations: fn() -> EmbeddedMigrations, pub drizzle: Option<DrizzleBaseline> }` and `pub struct DrizzleBaseline { pub created_at: i64, pub sql: &'static str, pub snapshot: &'static str }` (`spec-store.md` §2.2).
- **Run 4–6 (11 lines):** `#[diesel(sql_type = BigInt)]` count row and `conn.run_pending_migrations((schema.migrations)()).map_err(|e| Error::Migration(e.to_string()))?;` — standard `diesel_migrations::MigrationHarness` call.

### 3.4 `store/src/snapshot.rs` (25.0% — 8 / 32 lines)
- **Run 1–2 (8 lines):** `use diesel::prelude::*; use diesel::sql_types::Text; use crate::{Connection, Error}; #[derive(QueryableByName)] ... #[diesel(sql_type = Text)] sql: String, } pub fn schema_snapshot(conn: &mut Connection) -> Result<String, Error>` — Diesel `QueryableByName` single-column `Text` row and public function signature (`spec-store.md` §2.3).

### 3.5 `protocol/src/lib.rs` (83.3% — 5 / 6 lines)
- **Run 1 (5 lines):** `pub mod account; pub mod invite; pub mod manifest; pub mod status; pub mod text;` — 5-line crate root module list (`spec-protocol.md` §2).

### 3.6 `protocol/src/manifest.rs` (55.1% — 27 / 49 lines)
- **Run 1–3 (12 lines):** Public wire structs `RuntimeManifest`, `RuntimeBuild`, and `RuntimeFile` with `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]` and `#[cfg_attr(feature = "ts", derive(ts_rs::TS))]` — forced by JSON wire serialization and TypeScript bridge generation (`spec-protocol.md` §6).
- **Run 4–5 (15 lines):** `RuntimeManifest::new` (sorts `files` by `name` and sets `build.name = RUNTIME_BUILD_NAME.to_owned()`) and `to_json(&self)` — forced by byte-for-byte JSON parity with Node's `/undaunted/runtime/manifest.json`.

### 3.7 `protocol/src/account.rs` (28.6% — 34 / 119 lines)
- **Run 1 (6 lines):** `REGISTER_PATH = "/reforged/api/Register"`, `ACCOUNT_KEY_PREFIX = "RRK_"`, `USERNAME_MIN = 3`, `USERNAME_MAX = 16` — protocol constants (`spec-protocol.md` §4).
- **Run 2–4 (19 lines):** `pub struct Registered { #[serde(rename = "UUK")] pub key: String }`, `pub enum UsernameCheck { Ok, Empty, TooShort, TooLong, BadChars }`, and `UsernameCheck::as_str` snake_case mapping — forced by JSON wire schema and parity test table `tests/cases/username.json`.
- **Run 5–7 (9 lines):** Closing braces and `if is_plausible_account_key(trimmed) { return Some(trimmed); }`.

### 3.8 `protocol/src/status.rs` (27.4% — 122 / 446 lines)
- **Run 1–6 (82 lines):** Public wire types `RegistrationMode`, `PlayerWhere`, `InstanceKind`, `GameDataKind`, `StatusPlayer` (`#[serde(rename = "where")]`), `StatusInstance` (`#[serde(rename_all = "camelCase")]`), and `ServerStatus` (13 public fields with `ts_rs::TS` derives) — forced by `/undaunted/api/status` JSON schema and UI TypeScript bridge generation (`spec-protocol.md` §5).
- **Run 7–11 (27 lines):** String-to-enum match arms (`"menu" => PlayerWhere::Menu`, `"city" => Some(InstanceKind::City)`, `"real" => GameDataKind::Real`, etc.) — forced by the wire enum values.
- **Run 12–13 (13 lines):** `pub fn limited_view(status: ServerStatus) -> ServerStatus` clearing `players_online: 0, players: Vec::new(), instances: Vec::new(), limited: true, ..status`.

### 3.9 `service-kit/src/lib.rs` (92.9% — 13 / 14 lines)
- **Run 1 (13 lines):** `pub mod body; pub mod collate; ... pub mod telemetry;` — 13 module declarations (`spec-service-kit.md` §1).

### 3.10 `service-kit/src/routing.rs` (48.9% — 137 / 280 lines)
- **Run 1–5 (31 lines):** Standard `axum` / `std::task` imports, `Pattern` struct, `BadParam(pub String)`, and `Pattern::parse` / `as_str` signatures (`spec-service-kit.md` §10).
- **Run 6–11 (24 lines):** `pub struct Params`, `impl<S: Send + Sync> FromRequestParts<S> for Params`, and `pub struct MatchedPattern(pub Arc<str>)` — `axum` extractor trait implementation.
- **Run 12–21 (64 lines):** `Router<S>` builder methods (`new`, `route`, `get`, `post`, `put`, `patch`, `delete`, `layer`, `merge`, `table`, `into_service`) and their `axum::handler::Handler<T, S>` and `tower::Layer` / `tower::Service<Request<Body>>` trait bounds.
- **Run 22–25 (18 lines):** `impl Service<Request<Body>> for RouterService` (`type Response = Response; type Error = Infallible; ... fn poll_ready ... fn call`).

### 3.11 `service-kit/src/ratelimit.rs` (32.8% — 89 / 271 lines)
- **Run 1–4 (20 lines):** Imports and `pub struct Decision { pub remaining: u64, pub reset_secs: u64, pub limited: bool, ... }`.
- **Run 5–8 (22 lines):** `Limiter` public methods (`skip_successful`, `name`, `undo`, `sweep`, `headers`).
- **Run 9–10 (13 lines):** IETF draft-7 `ratelimit` (`"\"{}\"; r={}; t={}"`) and `ratelimit-policy` header construction required by `spec-service-kit.md` §9.
- **Run 11–12 (31 lines):** `pub struct Limited`, `Limited::new`, `pub async fn enforce(State(limited): State<Limited>, request: Request, next: Next) -> Response`, and `pub async fn around` — `axum::middleware::from_fn_with_state` signature.

### 3.12 `service-kit/src/serve.rs` (32.2% — 37 / 115 lines)
- **Run 1–5 (32 lines):** `pub struct ConnectionInfo { pub peer: SocketAddr, pub local: SocketAddr }`, `impl Connected<IncomingStream<'_, TcpListener>> for ConnectionInfo`, `impl Connected<IncomingStream<'_, NoDelay>> for ConnectionInfo`, `pub struct NoDelay(pub TcpListener)`, `impl Listener for NoDelay`, and `pub async fn serve(...)` signature.
- **Run 6 (5 lines):** `axum::serve(NoDelay(listener), app.into_make_service_with_connect_info::<ConnectionInfo>()).with_graceful_shutdown(...)`.

### 3.13 `service-kit/src/body.rs` (29.1% — 73 / 251 lines)
- **Run 1–3 (47 lines):** `DEFAULT_LIMIT = 50 * 1024 * 1024`, `PARAMETER_LIMIT = 1000`, `pub enum BodyError` + `BodyError::status(self) -> StatusCode` + `IntoResponse`, `pub type OnBodyError`, and `pub struct BodyConfig` + `Default` impl (`spec-service-kit.md` §8).
- **Run 4–11 (26 lines):** `pub struct ExpressBody(pub Option<Value>)`, `FromRequestParts` impl header, `pub fn parse_json` empty-string check, and `PARAMETER_LIMIT` guard.

### 3.14 `service-kit/src/linelog.rs` (27.8% — 55 / 198 lines)
- **Run 1–4 (23 lines):** `pub enum Level { Debug = 10, Info = 20, Warn = 30, Error = 40 }`, `Level::as_str`, and `tracing::{debug,info,warn,error}!("{msg} ({text})")` dispatch (`spec-service-kit.md` §6).
- **Run 5–10 (32 lines):** Convenience functions `pub fn info`, `pub fn warn`, `pub fn error`, `pub fn debug` and `pub struct Throttle` (`new`, `allow`).

### 3.15 `service-kit/src/env.rs` (25.0% — 30 / 120 lines)
- **Run 1–2 (13 lines):** `pub enum EnvError { #[error("{path}: {message}")] File { ... }, #[error("{name}: {message}")] Value { ... } }` and `Env::load` / `Env::parse` signatures (`spec-service-kit.md` §5).
- **Run 3 (17 lines):** `pub fn flag(&self, name: &str, default: bool) -> Result<bool, EnvError>` matching `"1"` / `"0"` and formatting `"{other:?} is not 0 or 1"`.

### 3.16 `hostkit/src/paths.rs` (38.5% — 115 / 299 lines)
- **Run 1 (6 lines):** `RUNTIME_FILE_NAMES`, `GAME_CHANGELIST`, `GAME_VERSION_TEXT`, `PINNED_EXE_SHA256` (`spec-dl3-hostkit.md` §2.1).
- **Run 2 (49 lines):** `pub struct Ports`, `pub const PORTS`, `pub struct LegacyPorts`, and `pub const BOTH_LEGACY_PORTS` (fixed TCP/UDP port numbers `61000..61100`, `27017`, `8766..8787`).
- **Run 3–6 (60 lines):** `pub struct Paths` with all 45 public `PathBuf` fields (`repo`, `preset`, `host_kit`, `gateway_dir`, ..., `systemd`) accessed by name across every `hostkit` and `services/host` command.

### 3.17 `hostkit/src/gamepack.rs` (30.2% — 107 / 354 lines)
- **Run 1–5 (31 lines):** `PACK_FORMAT`, `PACK_VERSION`, and public structs `PackFile`, `Pack`, `Exported`, `Imported` (`spec-dl3-hostkit.md` §2.10).
- **Run 6–11 (31 lines):** `pack_entries` loop over `BACKEND_GAME_DATA` and `DIRECTOR_GAME_DATA`, `pack_json` JSON object layout (`format`, `version`, `game`, `created`, `files`), and `parse_game_data_pack` signature.
- **Run 12–26 (45 lines):** 12 three-line error-formatting tails (`file.display() ))); }`) in `parse_game_data_pack` where every validation failure formats `"{}: <reason>", file.display()`.

### 3.18 `hostkit/src/cli.rs` (29.3% — 191 / 652 lines)
- **Run 1–6 (85 lines):** `pub struct Cli`, `pub enum Command` variants (`Setup`, `Start`, `Serve`, `Invite`, `Doctor`, `Gamedata`, `PublishRuntime`, `InstallGame`, `Preset`, `Multipliers`, `Update`, `Backup`, plus hidden service entrypoints `Gateway`, `Allowlist`, `Content`, `Admin`, `Backend`, `Director`), and `Command::name(&self) -> &'static str` (`spec-dl3-hostkit.md` §2.13).
- **Run 7–42 (106 lines):** `clap` derive struct fields (`#[arg(long)] pub json: bool`, `#[arg(long)] pub force: bool`, `DbCommand`, `NetworkCommand`, `HistoryCommand`) forced by CLI flag names.

### 3.19 `hostkit/src/publish.rs` (26.2% — 67 / 256 lines)
- **Run 1–3 (29 lines):** Public structs `RuntimeFile`, `PublishedRuntime`, `Replaced`, and `InstalledRuntime` (`spec-dl3-hostkit.md` §2.12).
- **Run 4–13 (38 lines):** `default_sources(p: &Paths)` and `publish_runtime` function signatures plus short 3-line error/cleanup tails.

### 3.20 `launcher-kit/src/bridge/types.rs` (90.4% — 719 / 795 lines)
- Pure wire-type definitions (`#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]` structs and enums: `RuntimeState`, `ErrorCode`, `NoticeCode`, `LauncherError`, `ActionResult`, `HostKit`, `HostServerView`, `RegionPing`, `GameVersionEntry`, `ModuleView`, `AdminPlayerView`, etc.) extracted from `src-tauri/src/bridge/types.rs` so `launcher-kit` and `src-tauri` share the exact IPC wire types without circular dependencies (`spec-dl4-launcher.md` §4.6). Contains no business logic beyond `Serialize` wrapper impls (`{ ok: true, ... }` / `{ ok: false, error }`).

### 3.21 `launcher-kit/src/platform/dialogs.rs` (67.9% — 36 / 53 lines)
- **Run 1–5 (27 lines):** `pub enum FolderPurpose`, `pub enum KeepOpen`, `pub const DIALOG_*` translation key constants, `TEXT_FILES` / `DLL_FILES` / `GAMEDATA_PACKS` filter tuples, and the 5-arm `folder_title(purpose)` match (`spec-dl4-launcher.md` §2.6).
- **Run 6–7 (9 lines):** Unit test asserting `folder_title` returns the 4 `dialog_*_folder` keys.

### 3.22 `launcher-kit/src/net/runtime_files.rs` (27.6% — 149 / 539 lines)
- **Run 1–8 (58 lines):** Public constants (`DOWNLOAD_SUFFIX = ".rr-download"`, `ASIDE_MARK = ".rr-old-"`), public enums/structs (`RuntimeManifestResult`, `RuntimeErrorCode`, `RuntimeError`, `RuntimeProgress`, `RuntimeInstallResult`, `Replaced`), and `pub trait ReplaceOps` + `DiskOps` (`spec-dl4-launcher.md` §2.1).
- **Run 9–19 (60 lines):** `pub struct InstallRequest<'a>`, `file_infos`, `runtime_set_id`, `accept`, `is_complete_runtime_dir`, and `check_runtime` state return arms (`RuntimeState::Ok`, `Untrusted`, `Missing`, `Outdated`).
- **Run 20–28 (31 lines):** `InstallRequest` destructuring and `DownloadOptions` field initialization.

### 3.23 `launcher-kit/src/platform/hostkit.rs` (25.6% — 200 / 782 lines)
- **Run 1–4 (50 lines):** Imports, `RUNTIME_FILE_NAMES`, `BACKEND_GAME_DATA` (7 filenames), `DIRECTOR_GAME_DATA` (7 filenames), `HOST_BINARY`, and the 18-field `pub struct WorkspacePaths` (`spec-dl4-launcher.md` §2.3).
- **Run 5–10 (37 lines):** `pub struct Toolchain`, `Toolchain::view()`, `pub enum SdkError` + `SdkError::detail()`, `pub struct Logged`, `find_host_binary`, and `find_dumps`.
- **Run 11–17 (35 lines):** Fixed `vswhere.exe` CLI flag arrays (`-latest`, `-products`, `*`, `-requires`, `Microsoft.VisualStudio.Component.VC.Tools.x86.x64`, `-format`, `json`, `-utf8`).
- **Run 18–28 (78 lines):** Default/empty `HostKit` struct literal field initializers (`workspace_ok: false`, `upstream: HostUpstream { present: false, patched: false }`, etc.).

---

## 4. Audit of Behaviour Specifications (`spec-dl3-hostkit.md`, `spec-dl4-launcher.md`, `spec-service-kit.md`)

Per F6, we audited `docs/port/spec-dl3-hostkit.md`, `docs/port/spec-dl4-launcher.md`, and `docs/port/spec-service-kit.md` (in addition to `docs/port/spec-store.md`, which was rewritten to observable behaviour only in F5) to remove internal private helper names and private struct fields, while documenting any `pub(crate)` or cross-module items that must remain in the spec because untouched Category A (`Born-MIT`) or shell modules call them directly.

### 4.1 Private Functions & Private Struct Fields Removed

1. **`docs/port/spec-dl3-hostkit.md`**:
   - **§1.1 (L37):** Removed private `repl.rs` helper function names `parse_module`, `module_table`, and `run_module`.
   - **§2.2 (`envfile`, L239 & L269–271):** Removed the internal field comment `{ /* preserves lines, line ending, BOM */ }` on `EnvDocument` and replaced references to private enum variants `Line::Entry` and `Line::Raw("")` and private field names `bom` / `line_ending` with observable round-trip behaviour.
   - **§2.3 (`hostconfig`, L389):** Replaced `FileForm`'s three private field names (`document: Map<String, Value>`, `read: Map<String, Value>`, `toml: Option<String>`) with an opaque description of its round-trip preservation role.
   - **§2.3 (`hostconfig`, L499 & L503):** Removed private helper function names `database_env(c, s, p)` and `node_link_env`.
   - **§2.8 (`mongo`, L790):** Replaced the call-site reference to `first_host(uri)` inside `ensure_replica_set`'s behavioral prose with `"the first host parsed from uri"`.
   - **§2.10 (`gamepack`, L968):** Removed the private helper name `is_example` from `export_game_data_pack`'s prose.
   - **§2.21 (`commands::update`, L1532):** Removed the internal field comment `{ /* paths for current and legacy workspace locations */ }` on `pub struct Layout`.

2. **`docs/port/spec-dl4-launcher.md`**:
   - **§2.1 (`net::runtime_files`, L320):** Removed `is_dir()` implementation detail from `install_runtime`.
   - **§2.2 (`net::verify`, L404 & L445–454):** Removed `HashCache`'s three private fields `{ /* path: PathBuf, entries: Vec<(String, CacheEntry)>, dirty: bool */ }` and replaced references to `self.entries`, `self.dirty`, and `entry.mtime_ms` in the behavioural prose with observable cache hit/invalidation and atomic save semantics.
   - **§2.4 (`platform::hostserver`, L704):** Removed `{ /* ... */ }` from `pub struct HostServer`.
   - **§2.7 (`controller::host`, L896 & L938–951):** Removed `HostState`'s three private fields `{ /* server: Arc<HostServer>, dump_root: PathBuf, inner: Mutex<Inner> */ }` and removed private helper method names `require_host_tools`, `begin_job`, `emit_host_kit`, `start_dump`, `find_running_clients`, and `start_export`.
   - **§2.8 (`controller::runtime`, L981–994) & §4.5 (L1164):** Replaced private `Controller` state struct field references (`st.runtime_manifest`, `st.runtime.state`, `st.task`, `st.download_cancel`, `st.key_rejected`, `st.last_error`, `compute_phase`) with observable `RuntimeView` / `ServerView` / `Phase` transitions.

3. **`docs/port/spec-service-kit.md`**:
   - **§11 (`serve`) & §12 (`jsjson`):** Replaced the `NoDelay` `Listener::accept` code block and `write_json_number` step-by-step algorithm walkthrough with concise behavioural invariants.

### 4.2 `pub(crate)` and Cross-Module Items Kept in the Specs (and Why)

1. **`docs/port/spec-dl3-hostkit.md`**:
   - **`bad`, `shown`, `integer`, `path_text` in `hostconfig.rs` (§2.3):** Kept as `pub(crate)` functions because the untouched Category A (`Born-MIT`) submodules `hostconfig/guard.rs`, `hostconfig/modules.rs`, and `hosttoml.rs` import `crate::hostconfig::{bad, shown, integer, path_text}` directly.
   - **`newest_installed`, `program_files`, `search_path` in `mongo.rs` (§2.8):** Kept as `pub` functions because the untouched Category A (`Born-MIT`) module `commands/backup.rs` calls `crate::mongo::{newest_installed, program_files, search_path}` to locate `mongodump`.
   - **`http_request` in `invite.rs` (§2.6):** Kept as `pub` because `services/host/src/lib.rs` and integration tests call `hostkit::invite::http_request` directly.

2. **`docs/port/spec-dl4-launcher.md`**:
   - **`arg_accepts` (`bridge/spec.rs`, §3.1) and `host_setup_request`, `host_multipliers`, `invite_for`, `realm_id`, `guarded_id` (`bridge/commands.rs`, §3.1):** Kept because `src-tauri/src/bridge/commands.rs` and its unit tests call these validators directly across module boundaries.
   - **`refresh_runtime_offer` and `inspect_runtime` (`controller/runtime.rs`, §2.8):** Kept as `pub(super)` controller methods because `controller/mod.rs`, `controller/servers.rs`, and `controller/host.rs` in `src-tauri` invoke them across submodules of `controller`.

---

## 5. `services/content` (RD27)

Measured by the integration pass of the content rewrite (`feat/rd27-content-mit`, new text at `16e66e0`) against two
baselines outside dainty-lass: dainty-restricted's `services/content` at `ee7d90b` and DR's `rust/services/content` at
`24b7624`, by counterpart file and against each whole baseline crate. The full table, with the "all" columns, and its
reading are in `content.md` section 6; the counterpart figures:

| File | New lines | Restricted `ee7d90b` | DR `24b7624` |
| --- | ---: | ---: | ---: |
| `services/content/src/lib.rs` | 44 | 29.5% | 6.8% |
| `services/content/src/app.rs` | 626 | 4.0% | 2.1% |
| `services/content/src/auth.rs` | 359 | 2.2% | 2.2% |
| `services/content/src/branding.rs` | 289 | 10.4% | 10.4% |
| `services/content/src/cli.rs` | 98 | 25.5% | 19.4% |
| `services/content/src/client.rs` | 124 | 23.4% | 23.4% |
| `services/content/src/config.rs` | 303 | 18.8% | 12.2% |
| `services/content/src/input.rs` | 632 | 17.9% | 4.1% |
| `services/content/src/limits.rs` | 96 | 19.8% | 16.7% |
| `services/content/src/log.rs` | 1 | 0.0% | 0.0% |
| `services/content/src/news.rs` | 332 | 16.3% | 16.3% |
| `services/content/src/runtime.rs` | 321 | 5.3% | — |
| `services/content/src/server.rs` | 204 | 24.5% | 18.6% |
| `services/content/src/service.rs` | 77 | 49.4% (Category A baseline) | — |
| `services/content/tests/app.rs` | 534 | 6.7% | 1.3% |
| `services/content/tests/parity.rs` | 49 | 36.7% | 6.1% |
| `services/content/tests/parity/harness.rs` | 379 | 4.2% | 3.2% |
| `fuzz/fuzz_targets/content_request.rs` | 45 | 6.7% | — |
| `fuzz/fuzz_targets/content_runtime_index.rs` | 21 | 52.4% | — |
| `fuzz/fuzz_targets/content_news.rs` | 23 | 0.0% | — |

The script, run with Node from anywhere (paths are the maintainer's checkouts): `node measure-content.mjs 16e66e0 25`.

```javascript
// similarity.md's method (3+-line runs of trimmed non-blank, non-comment lines), for the content rewrite.
// Old text from two baselines; new text from the clean-room commit. Also measured against each baseline's whole
// crate concatenated, so code moved between files is not missed.
import { execFileSync } from "node:child_process";

const LASS = "R:/source/dauntless/.wt/content-int";
const RESTRICTED = "R:/source/dauntless/dainty-restricted";
const DR = "R:/source/dauntless/legacy-revived";
const NEW_REV = process.argv[2] || "16e66e0";
const SHOW = Number(process.argv[3] || "25");

function codeLines(text) {
  return text.split(/\r?\n/).map((l) => l.trim()).filter(
    (l) => l.length > 0 && !l.startsWith("//") && !l.startsWith("/*") && !l.startsWith("*") && !l.startsWith("*/"));
}

function measure(oldText, newText) {
  const oldLines = codeLines(oldText);
  const newLines = codeLines(newText);
  if (newLines.length === 0) return { oldCount: oldLines.length, newCount: 0, matched: 0, pct: 0, runs: [] };
  const flags = new Array(newLines.length).fill(false);
  const runs = [];
  for (let i = 0; i < newLines.length; i++) {
    let best = 0;
    for (let j = 0; j < oldLines.length; j++) {
      let k = 0;
      while (i + k < newLines.length && j + k < oldLines.length && newLines[i + k] === oldLines[j + k]) k++;
      if (k > best) best = k;
    }
    if (best >= 3) {
      for (let k = 0; k < best; k++) flags[i + k] = true;
      runs.push({ len: best, sample: newLines.slice(i, i + best) });
      i += best - 1;
    }
  }
  const matched = flags.filter(Boolean).length;
  return { oldCount: oldLines.length, newCount: newLines.length, matched, pct: (matched / newLines.length) * 100, runs };
}

function show(repo, rev, path) {
  try {
    return execFileSync("git", ["-C", repo, "show", `${rev}:${path}`], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"], maxBuffer: 1 << 26 });
  } catch { return null; }
}
function list(repo, rev, dir) {
  return execFileSync("git", ["-C", repo, "ls-tree", "-r", "--name-only", rev, dir], { encoding: "utf8" })
    .split(/\n/).filter((p) => p.endsWith(".rs"));
}
const concat = (repo, rev, dirs) => dirs.flatMap((d) => list(repo, rev, d)).map((p) => show(repo, rev, p) || "").join("\n");

const R_ALL = concat(RESTRICTED, "ee7d90b", ["services/content", "services/content-plugin", "fuzz/fuzz_targets"])
  + "\n" + (show(RESTRICTED, "75e0552", "services/content/src/service.rs") || "");
const DR_ALL = concat(DR, "24b7624", ["rust/services/content"]);

// new path -> [restricted counterpart(s) @ee7d90b, DR counterpart(s) @24b7624]
const S = "services/content/src/", T = "services/content/tests/", F = "fuzz/fuzz_targets/";
const pairs = [
  [S + "lib.rs", [S + "lib.rs"], ["rust/services/content/src/lib.rs"]],
  [S + "app.rs", [S + "app.rs"], ["rust/services/content/src/app.rs"]],
  [S + "auth.rs", [S + "auth.rs"], ["rust/services/content/src/auth.rs"]],
  [S + "branding.rs", [S + "branding.rs"], ["rust/services/content/src/branding.rs"]],
  [S + "cli.rs", [S + "cli.rs"], ["rust/services/content/src/main.rs"]],
  [S + "client.rs", [S + "client.rs"], ["rust/services/content/src/client.rs"]],
  [S + "config.rs", [S + "config.rs"], ["rust/services/content/src/config.rs"]],
  [S + "input.rs", [S + "input.rs"], ["rust/services/content/src/app.rs", "rust/services/content/src/news.rs"]],
  [S + "limits.rs", [S + "limits.rs"], ["rust/services/content/src/limits.rs"]],
  [S + "log.rs", [S + "log.rs"], ["rust/services/content/src/log.rs"]],
  [S + "news.rs", [S + "news.rs"], ["rust/services/content/src/news.rs"]],
  [S + "runtime.rs", [S + "runtime.rs"], []],
  [S + "server.rs", [S + "server.rs"], ["rust/services/content/src/server.rs"]],
  [S + "service.rs", ["@75e0552:" + S + "service.rs", "services/content-plugin/src/lib.rs"], []],
  [T + "app.rs", [T + "app.rs"], ["rust/services/content/tests/streams.rs"]],
  [T + "parity.rs", [T + "parity.rs"], ["rust/services/content/tests/parity.rs"]],
  [T + "parity/harness.rs", [T + "parity/harness.rs"], ["rust/services/content/tests/parity.rs"]],
  [F + "content_request.rs", [F + "content_request.rs"], []],
  [F + "content_runtime_index.rs", [F + "content_runtime_index.rs"], []],
  [F + "content_news.rs", [F + "content_news.rs"], []],
  ["services/content/Cargo.toml", ["services/content/Cargo.toml"], ["rust/services/content/Cargo.toml"]],
];

const rOld = (p) => p.startsWith("@") ? show(RESTRICTED, p.slice(1, 8), p.slice(9)) : show(RESTRICTED, "ee7d90b", p);
const fmt = (r) => r ? `${String(r.oldCount).padStart(5)} ${String(r.matched).padStart(4)} ${r.pct.toFixed(1).padStart(5)}%` : "    -    -      -";
const detail = [];
console.log("file | new | restricted(old matched pct) | restricted-all | DR(old matched pct) | DR-all");
for (const [np, rp, dp] of pairs) {
  const newText = show(LASS, NEW_REV, np);
  if (!newText) { console.log(`${np}: missing at ${NEW_REV}`); continue; }
  const rText = rp.map(rOld).filter(Boolean).join("\n");
  const dText = dp.map((p) => show(DR, "24b7624", p)).filter(Boolean).join("\n");
  const r = rText ? measure(rText, newText) : null;
  const ra = measure(R_ALL, newText);
  const d = dText ? measure(dText, newText) : null;
  const da = measure(DR_ALL, newText);
  console.log(`${np.padEnd(44)} | ${String(codeLines(newText).length).padStart(4)} | ${fmt(r)} | ${ra.matched} ${ra.pct.toFixed(1)}% | ${fmt(d)} | ${da.matched} ${da.pct.toFixed(1)}%`);
  for (const [tag, res] of [["restricted", r], ["DR", d]]) {
    if (res && res.pct >= SHOW) detail.push(`\n## ${np} vs ${tag} (${res.pct.toFixed(1)}%)\n` + res.runs.map((x) => `  [${x.len}] ${x.sample.join("  ///  ")}`).join("\n"));
  }
  // long runs regardless of pct
  for (const [tag, res] of [["restricted-all", ra], ["DR-all", da]]) {
    const long = res.runs.filter((x) => x.len >= 8);
    if (long.length) detail.push(`\n## ${np} long runs (>=8) vs ${tag}\n` + long.map((x) => `  [${x.len}] ${x.sample.join("  ///  ")}`).join("\n"));
  }
}
console.log(detail.join("\n"));
```
