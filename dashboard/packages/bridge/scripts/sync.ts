// Copies the launcher bridge from a dainty-lass checkout: rust/launcher/page is its source of truth and its
// generated types come from spec.rs (RR_UPDATE_GENERATED=1). Run after any bridge change there:
//   bun run bridge:sync [path/to/dainty-lass]
import { readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(process.argv[2] ?? join(import.meta.dir, "../../../../dainty-lass"));
const page = join(root, "rust/launcher/page");
const out = join(import.meta.dir, "../src");

// The page imports with .ts extensions for Vite's raw mode; the apps resolve without them.
const rewrite = (text: string) =>
  text.replaceAll(/from "\.\/(generated\/types|types)\.ts"/g, 'from "./$1"');

for (const [from, to] of [
  ["generated/types.ts", "generated/types.ts"],
  ["types.ts", "types.ts"],
  ["api.ts", "api.ts"],
] as const) {
  writeFileSync(join(out, to), rewrite(readFileSync(join(page, from), "utf8")));
  console.log(`synced ${from}`);
}
