// Copies the bridge types the launcher generates from spec.rs (RR_UPDATE_GENERATED=1) from a dainty-restricted
// checkout. They are AGPL, so they stay out of this repository (RD27).
// The checkout is DAINTY_RESTRICTED_DIR, or dainty-restricted beside dainty-lass. Run after any bridge change:
//   bun run restricted-data
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

const restricted = resolve(
  process.env.DAINTY_RESTRICTED_DIR ?? join(import.meta.dir, "../../../../../dainty-restricted"),
);
const sources = {
  "generated/types.ts": join(restricted, "ui/packages/bridge/src/generated/types.ts"),
};
const out = join(import.meta.dir, "../src");

// The page imports with .ts extensions for Vite's raw mode; the apps resolve without them.
const rewrite = (text: string) =>
  text.replaceAll(/from "\.\/(generated\/types|types)\.ts"/g, 'from "./$1"');

for (const [to, from] of Object.entries(sources)) {
  if (!existsSync(from)) {
    console.error(`missing ${from}: set DAINTY_RESTRICTED_DIR to a dainty-restricted checkout`);
    process.exit(1);
  }
  mkdirSync(dirname(join(out, to)), { recursive: true });
  writeFileSync(join(out, to), rewrite(readFileSync(from, "utf8")));
  console.log(`copied ${to}`);
}
