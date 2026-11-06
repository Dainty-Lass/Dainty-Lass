#!/usr/bin/env node
// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud
//
// Convenience wrapper for `dainty-host db replay` (or `cargo run -p isles-db -- replay`).
// Usage:
//   node scripts/replay-capture.mjs --corpus <path> [--left recorded|<url>] [--right <url>] [--key-map <file.json>] [--out <report.json>] [--json]

import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, '..');

const rawArgs = process.argv.slice(2);
if (rawArgs.length === 0 || rawArgs.includes('-h') || rawArgs.includes('--help')) {
  console.log(`Usage:
  node scripts/replay-capture.mjs --corpus <path> [options]

Options:
  --corpus <path>           Capture JSONL file or directory of JSONL files (required)
  --left <url|recorded>     Left target URL or "recorded" (default: "recorded")
  --right <url>             Right target URL (default: "http://127.0.0.1:61000")
  --left-director <url>     Optional Director URL for left target
  --right-director <url>    Optional Director URL for right target
  --left-chat <host:port>   Optional XMPP chat address for left target
  --right-chat <host:port>  Optional XMPP chat address for right target
  --key-map <file.json>     Optional JSON map or array of raw account keys for <key:sha256>
  --signing-key <b64>       Optional RS256 AUTH_SIGNING_PRIVKEY_B64 (defaults from secrets.env)
  --gs-key <key>            Optional GAMESERVER_API_KEY (defaults from secrets.env)
  --gateway-secret <sec>    Optional GATEWAY_SECRET (defaults from secrets.env)
  --pin-clock / --no-pin-clock
                            Pin clock & RNG via x-isles-replay-* headers (default: --pin-clock)
  --filter <substr>         Only replay records matching <substr>
  --out <report.json>       Write full JSON diff report to <report.json>
  --json                    Print full JSON diff report to stdout
`);
  process.exit(0);
}

const isWin = process.platform === 'win32';
const exeSuffix = isWin ? '.exe' : '';
const candidates = [
  path.join(repoRoot, `dainty-host${exeSuffix}`),
  path.join(repoRoot, 'rust', 'target', 'release', `dainty-host${exeSuffix}`),
  path.join(repoRoot, 'rust', 'target', 'debug', `dainty-host${exeSuffix}`),
];

const hostBin = candidates.find((p) => fs.existsSync(p));
let result;
if (hostBin) {
  result = spawnSync(hostBin, ['db', 'replay', ...rawArgs], {
    cwd: repoRoot,
    stdio: 'inherit',
  });
} else {
  result = spawnSync(
    'cargo',
    [
      'run',
      '--manifest-path',
      path.join(repoRoot, 'rust', 'Cargo.toml'),
      '-p',
      'server-console',
      '--bin',
      'dainty-host',
      '--locked',
      '--',
      'db',
      'replay',
      ...rawArgs,
    ],
    {
      cwd: repoRoot,
      stdio: 'inherit',
    },
  );
}

if (result.error) {
  console.error(`replay-capture failed: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status ?? 1);
