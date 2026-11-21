// SPDX-License-Identifier: MIT OR Apache-2.0
import { afterAll, describe, expect, test } from "bun:test"
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import os from "node:os"
import path from "node:path"

import { mergePatch } from "../scripts/release-config"
import { compareSdkVersions } from "../scripts/sign-windows"

const scripts = path.resolve(import.meta.dir, "../scripts")
const temp = mkdtempSync(path.join(os.tmpdir(), "dl-launcher-scripts-"))

afterAll(() => rmSync(temp, { recursive: true, force: true }))

// The secrets the scripts read are cleared, so a developer's own shell can't change the outcome
function runScript(script: string, args: string[], env: Record<string, string> = {}) {
  const child = Bun.spawnSync([process.execPath, path.join(scripts, script), ...args], {
    env: {
      ...process.env,
      TAURI_SIGNING_PRIVATE_KEY: "",
      WINDOWS_CERTIFICATE_PATH: "",
      WINDOWS_CERTIFICATE_PASSWORD: "",
      ...env,
    },
    stdout: "pipe",
    stderr: "pipe",
  })
  return { code: child.exitCode, stdout: child.stdout.toString(), stderr: child.stderr.toString() }
}

describe("release-config's merge", () => {
  test("nested objects merge", () => {
    expect(mergePatch({ bundle: { active: true, windows: { a: 1 } } }, { bundle: { windows: { b: 2 } } })).toEqual({
      bundle: { active: true, windows: { a: 1, b: 2 } },
    })
  })

  test("arrays and scalars replace", () => {
    expect(mergePatch({ targets: ["nsis", "msi"], name: "a" }, { targets: ["nsis"], name: "b" })).toEqual({
      targets: ["nsis"],
      name: "b",
    })
    expect(mergePatch({ bundle: { icon: ["x"] } }, { bundle: { icon: "y" } })).toEqual({ bundle: { icon: "y" } })
    expect(mergePatch({ a: 1 }, ["patch"])).toEqual(["patch"])
  })

  test("a key only in the patch is added", () => {
    expect(mergePatch({ a: 1 }, { b: { c: 2 } })).toEqual({ a: 1, b: { c: 2 } })
    expect(mergePatch(undefined, { b: 1 })).toEqual({ b: 1 })
  })

  test("null deletes, and the target is not changed", () => {
    const target = { a: 1, b: { c: 2 } }
    expect(mergePatch(target, { a: null, b: { d: 3 } })).toEqual({ b: { c: 2, d: 3 } })
    expect(target).toEqual({ a: 1, b: { c: 2 } })
  })
})

describe("release-config as a process", () => {
  const serverGui = { productName: "Dainty Lass Manager", bundle: { active: true, targets: ["nsis"] } }

  function tauriDir(name: string, pubkey?: string): string {
    const dir = path.join(temp, name)
    mkdirSync(dir, { recursive: true })
    const plugins = pubkey === undefined ? {} : { updater: { pubkey } }
    writeFileSync(path.join(dir, "tauri.conf.json"), JSON.stringify({ productName: "Dainty Lass Launcher", plugins }))
    writeFileSync(path.join(dir, "tauri.server-gui.conf.json"), JSON.stringify(serverGui))
    return dir
  }

  function read(file: string): unknown {
    return JSON.parse(readFileSync(file, "utf8"))
  }

  test("no secrets: an empty bundle overlay", () => {
    const out = path.join(temp, "out-plain", "nested")
    const result = runScript("release-config.ts", [out, "--tauri-dir", tauriDir("plain")])
    expect(result.code).toBe(0)
    expect(result.stdout.trim()).toBe("updater signatures: off; Windows code signing: off")
    expect(read(path.join(out, "client.json"))).toEqual({ bundle: {} })
    expect(read(path.join(out, "manager.json"))).toEqual(serverGui)
    expect(readFileSync(path.join(out, "client.json"), "utf8")).toBe('{\n  "bundle": {}\n}\n')
  })

  test("the updater key without a pubkey fails", () => {
    for (const pubkey of [undefined, ""]) {
      const out = path.join(temp, "out-nokey")
      const result = runScript("release-config.ts", [out, "--tauri-dir", tauriDir("nokey", pubkey)], {
        TAURI_SIGNING_PRIVATE_KEY: "secret",
      })
      expect(result.code).toBe(1)
      expect(result.stderr).toContain(
        "::error::TAURI_SIGNING_PRIVATE_KEY is set but tauri.conf.json has no updater pubkey"
      )
    }
  })

  test("a pubkey and a certificate set both fields", () => {
    const out = path.join(temp, "out-signed")
    const result = runScript("release-config.ts", [out, "--tauri-dir", tauriDir("signed", "PUBKEY")], {
      TAURI_SIGNING_PRIVATE_KEY: "secret",
      WINDOWS_CERTIFICATE_PATH: path.join(temp, "cert.pfx"),
    })
    expect(result.code).toBe(0)
    expect(result.stdout.trim()).toBe("updater signatures: on; Windows code signing: on")
    const overlay = {
      bundle: {
        createUpdaterArtifacts: true,
        windows: { signCommand: { cmd: "bun", args: [path.join(scripts, "sign-windows.ts"), "%1"] } },
      },
    }
    expect(read(path.join(out, "client.json"))).toEqual(overlay)
    expect(read(path.join(out, "manager.json"))).toEqual({
      productName: "Dainty Lass Manager",
      bundle: { active: true, targets: ["nsis"], ...overlay.bundle },
    })
    expect(result.stdout).not.toContain("secret")
  })

  test("an unknown option prints the usage", () => {
    expect(runScript("release-config.ts", ["--nope"]).code).toBe(2)
  })
})

describe("dashboard-page", () => {
  for (const args of [[], ["build"], ["serve", "launcher"], ["build", "dashboard"], ["dev", "launcher", "extra"]]) {
    test(`exits 2 for ${JSON.stringify(args)}`, () => {
      const result = runScript("dashboard-page.ts", args)
      expect(result.code).toBe(2)
      expect(result.stderr).toContain("usage: bun scripts/dashboard-page.ts <build|dev> <launcher|server-manager>")
    })
  }

  test("a missing dashboard says where it looked", () => {
    const missing = path.join(temp, "no-dashboard")
    const result = runScript("dashboard-page.ts", ["build", "launcher"], { DAINTY_DASH_DIR: missing })
    expect(result.code).toBe(1)
    expect(result.stderr).toContain(path.join(missing, "apps", "web", "package.json"))
    expect(result.stderr).toContain("DAINTY_DASH_DIR")
  })
})

describe("sign-windows", () => {
  test("no file prints the usage", () => {
    expect(runScript("sign-windows.ts", []).code).toBe(2)
  })

  test("a missing certificate fails before signtool runs", () => {
    const result = runScript("sign-windows.ts", ["app.exe"], {
      WINDOWS_CERTIFICATE_PATH: path.join(temp, "missing.pfx"),
      WINDOWS_CERTIFICATE_PASSWORD: "hunter2",
    })
    expect(result.code).toBe(1)
    expect(result.stderr).toContain("WINDOWS_CERTIFICATE_PATH is not set to the certificate file")
    expect(result.stderr + result.stdout).not.toContain("hunter2")
  })

  test("no signtool: a clear error, and the password never printed", () => {
    const cert = path.join(temp, "cert.pfx")
    writeFileSync(cert, "not a certificate")
    const result = runScript("sign-windows.ts", ["app.exe"], {
      WINDOWS_CERTIFICATE_PATH: cert,
      WINDOWS_CERTIFICATE_PASSWORD: "hunter2",
      SIGNTOOL: path.join(temp, "missing-signtool.exe"),
    })
    expect(result.code).toBe(1)
    expect(result.stderr).toContain("signtool.exe not found: install a Windows 10/11 SDK")
    expect(result.stderr + result.stdout).not.toContain("hunter2")
  })

  test("SDK folders sort newest first, non-versions last", () => {
    const names = ["10.0.19041.0", "arm64", "10.0.22621.0", "10.0.9200.0", "10.0.22621.755", "x64"]
    expect(names.sort(compareSdkVersions)).toEqual([
      "10.0.22621.755",
      "10.0.22621.0",
      "10.0.19041.0",
      "10.0.9200.0",
      "arm64",
      "x64",
    ])
  })
})
