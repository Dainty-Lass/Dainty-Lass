// Checks that both Tauri desktop app bundle configurations (`dainty-lass-client` and
// `dainty-lass-manager`), their icons, Cargo features, and `package.json` build scripts are wired
// and distinct (Part B2, T3).

import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const launcherRoot = path.resolve(here, "..");
const srcTauri = path.join(launcherRoot, "src-tauri");

interface PackageJson {
  version: string;
  scripts?: Record<string, string>;
}

interface TauriConfig {
  productName?: string;
  mainBinaryName?: string;
  identifier?: string;
  build?: {
    frontendDist?: string;
    beforeBuildCommand?: string;
  };
  bundle?: {
    icon?: string[];
    windows?: {
      nsis?: {
        installerIcon?: string;
      };
    };
  };
}

function readJson<T>(filePath: string): T {
  return JSON.parse(fs.readFileSync(filePath, "utf8")) as T;
}

test("client and server-gui Tauri configurations have distinct names, identifiers, binaries, and icons", () => {
  const clientConf = readJson<TauriConfig>(path.join(srcTauri, "tauri.conf.json"));
  const serverConf = readJson<TauriConfig>(path.join(srcTauri, "tauri.server-gui.conf.json"));

  assert.equal(clientConf.productName, "DaintyLass Launcher");
  assert.equal(clientConf.mainBinaryName, "dainty-lass-client");
  assert.equal(clientConf.identifier, "io.github.freezamotoqueiro.dauntlessreforgedrevival");

  assert.equal(serverConf.productName, "DaintyLass Manager");
  assert.equal(serverConf.mainBinaryName, "dainty-lass-manager");
  assert.equal(serverConf.identifier, "io.github.freezamotoqueiro.dauntlessreforgedrevival.server");

  assert.notEqual(clientConf.productName, serverConf.productName);
  assert.notEqual(clientConf.mainBinaryName, serverConf.mainBinaryName);
  assert.notEqual(clientConf.identifier, serverConf.identifier);

  // Each app serves its own page from dainty-lass's dashboard/, which its build hook exports and copies into place
  assert.equal(clientConf.build?.frontendDist, "../dist/launcher");
  assert.equal(clientConf.build?.beforeBuildCommand, "node scripts/dashboard-page.mjs build launcher");
  assert.equal(serverConf.build?.frontendDist, "../dist/server-manager");
  assert.equal(serverConf.build?.beforeBuildCommand, "node scripts/dashboard-page.mjs build server-manager");

  // Every icon referenced by both configs exists on disk and the sets are distinct
  const clientIcons = clientConf.bundle?.icon ?? [];
  const serverIcons = serverConf.bundle?.icon ?? [];
  assert.ok(clientIcons.length >= 4, "client config must list bundle icons");
  assert.ok(serverIcons.length >= 4, "server-gui config must list bundle icons");
  for (const rel of clientIcons) {
    assert.ok(fs.existsSync(path.join(srcTauri, rel)), `missing client icon: ${rel}`);
  }
  for (const rel of serverIcons) {
    assert.ok(rel.startsWith("icons/server/"), `server-gui icon must be under icons/server/: ${rel}`);
    assert.ok(fs.existsSync(path.join(srcTauri, rel)), `missing server-gui icon: ${rel}`);
  }
  const clientIco = clientConf.bundle?.windows?.nsis?.installerIcon;
  const serverIco = serverConf.bundle?.windows?.nsis?.installerIcon;
  assert.ok(clientIco && fs.existsSync(path.join(srcTauri, clientIco)));
  assert.ok(serverIco && fs.existsSync(path.join(srcTauri, serverIco)));
  assert.notEqual(clientIco, serverIco);
});

test("package.json exposes build scripts for both desktop apps and Cargo.toml links game modules into server-gui", () => {
  const pkg = readJson<PackageJson>(path.join(launcherRoot, "package.json"));
  const scripts = pkg.scripts ?? {};

  assert.ok(scripts["build:client"]?.includes("tauri build"), "missing build:client script");
  assert.ok(
    scripts["build:manager"]?.includes("tauri.server-gui.conf.json") &&
      scripts["build:manager"]?.includes("--features server-gui"),
    "build:manager must pass tauri.server-gui.conf.json and --features server-gui",
  );
  assert.ok(scripts["build:server-gui"], "missing build:server-gui alias script");

  const cargoToml = fs.readFileSync(path.join(srcTauri, "Cargo.toml"), "utf8");
  assert.ok(
    cargoToml.includes(`version = "${pkg.version}"`),
    "Cargo.toml version must match package.json version",
  );
  const serverGuiLine = cargoToml
    .split(/\r?\n/)
    .find((line) => line.trimStart().startsWith("server-gui ="));
  assert.ok(serverGuiLine, "missing server-gui feature in Cargo.toml");
  for (const modFeature of ["isles-1-14-7"]) {
    assert.ok(
      serverGuiLine.includes(`"${modFeature}"`),
      `server-gui feature must include ${modFeature}`,
    );
  }
});
