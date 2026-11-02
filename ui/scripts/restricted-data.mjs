// SPDX-License-Identifier: MIT
// Copies the page's two AGPL inputs from a dainty-restricted checkout (RD27): the language catalogs, which keep
// the Revived and Reforged launcher text word for word, and the bridge types its launcher generates. They stay
// out of this repository; the checkout is DAINTY_RESTRICTED_DIR, or dainty-restricted beside dainty-lass.
import { cpSync, existsSync, mkdirSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"

const ui = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const restricted = process.env.DAINTY_RESTRICTED_DIR ?? path.resolve(ui, "../../dainty-restricted")
const files = [
  "packages/i18n/catalogs/en.json",
  "packages/i18n/catalogs/pt-BR.json",
  "packages/bridge/src/generated/types.ts",
]

for (const file of files) {
  const from = path.join(restricted, "ui", file)
  if (!existsSync(from)) {
    console.error(`missing ${from}: set DAINTY_RESTRICTED_DIR to a dainty-restricted checkout`)
    process.exit(1)
  }
  mkdirSync(path.dirname(path.join(ui, file)), { recursive: true })
  cpSync(from, path.join(ui, file))
}
