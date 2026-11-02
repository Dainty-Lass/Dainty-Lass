// Copies the types the launcher generates from spec.rs (RR_UPDATE_GENERATED=1) from this repository's ui/ (RD27). The
// page's api.ts and types.ts here are the dashboard's own. Run after any bridge change:
//   bun run bridge:sync
import { readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const restricted = resolve(import.meta.dir, "../../../..");
const sources = {
  "generated/types.ts": join(restricted, "ui/packages/bridge/src/generated/types.ts"),
};
const out = join(import.meta.dir, "../src");

// The page imports with .ts extensions for Vite's raw mode; the apps resolve without them.
const rewrite = (text: string) =>
  text.replaceAll(/from "\.\/(generated\/types|types)\.ts"/g, 'from "./$1"');

for (const [to, from] of Object.entries(sources)) {
  writeFileSync(join(out, to), rewrite(readFileSync(from, "utf8")));
  console.log(`synced ${to}`);
}
