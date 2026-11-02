// Writes the release builds' `tauri build --config` files into the folder given (RA14, docs/launcher-release.md):
// client.json, and manager.json with tauri.server-gui.conf.json under the same signing settings. Updater
// artifacts and their ed25519 signatures only with TAURI_SIGNING_PRIVATE_KEY, Authenticode only with
// WINDOWS_CERTIFICATE_PATH, so a run without the secrets still builds, unsigned.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scripts = path.dirname(fileURLToPath(import.meta.url));
const srcTauri = path.resolve(scripts, "../src-tauri");
const out = path.resolve(process.argv[2] ?? ".");
const read = (name) => JSON.parse(fs.readFileSync(path.join(srcTauri, name), "utf8"));

// As tauri merges --config (JSON merge patch): objects merge, anything else replaces
const merge = (base, patch) => {
  const isObject = (value) => value !== null && typeof value === "object" && !Array.isArray(value);
  if (!isObject(base) || !isObject(patch)) return patch;
  const result = { ...base };
  for (const [key, value] of Object.entries(patch)) result[key] = merge(base[key], value);
  return result;
};

const updaterSigning = Boolean(process.env.TAURI_SIGNING_PRIVATE_KEY);
const codeSigning = Boolean(process.env.WINDOWS_CERTIFICATE_PATH);
const overlay = { bundle: {} };
if (updaterSigning) {
  // A signed feed the launcher can't check would strand every install on this version (RD7)
  if (!read("tauri.conf.json").plugins?.updater?.pubkey) {
    console.error("::error::TAURI_SIGNING_PRIVATE_KEY is set but tauri.conf.json has no updater pubkey");
    process.exit(1);
  }
  overlay.bundle.createUpdaterArtifacts = true;
}
if (codeSigning) {
  const sign = path.join(scripts, "sign-windows.ps1");
  overlay.bundle.windows = {
    signCommand: { cmd: "pwsh", args: ["-NoProfile", "-NonInteractive", "-File", sign, "%1"] },
  };
}

fs.mkdirSync(out, { recursive: true });
fs.writeFileSync(path.join(out, "client.json"), JSON.stringify(overlay, null, 2) + "\n");
fs.writeFileSync(
  path.join(out, "manager.json"),
  JSON.stringify(merge(read("tauri.server-gui.conf.json"), overlay), null, 2) + "\n",
);
console.log(`updater signatures: ${updaterSigning ? "on" : "off"}; Windows code signing: ${codeSigning ? "on" : "off"}`);
