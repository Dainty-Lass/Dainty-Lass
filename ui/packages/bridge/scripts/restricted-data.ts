// Copies the bridge types launcher-kit generates from its command table (crates/launcher-kit/generated, MIT since
// RL13; RR_UPDATE_GENERATED=1 cargo test -p launcher-kit generated). They are generated output, so only the copy
// in src/generated is kept out of git. Run after any bridge change:
//   bun run restricted-data
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

const generated = resolve(join(import.meta.dir, "../../../../crates/launcher-kit/generated"));
const sources = {
  "generated/types.ts": join(generated, "types.ts"),
};
const out = join(import.meta.dir, "../src");

// The page imports with .ts extensions for Vite's raw mode; the apps resolve without them.
const rewrite = (text: string) =>
  text.replaceAll(/from "\.\/(generated\/types|types)\.ts"/g, 'from "./$1"');

for (const [to, from] of Object.entries(sources)) {
  if (!existsSync(from)) {
    console.error(
      `missing ${from}: run RR_UPDATE_GENERATED=1 cargo test -p launcher-kit generated`,
    );
    process.exit(1);
  }
  mkdirSync(dirname(join(out, to)), { recursive: true });
  writeFileSync(join(out, to), rewrite(readFileSync(from, "utf8")));
  console.log(`copied ${to}`);
}
