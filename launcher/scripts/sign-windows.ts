// SPDX-License-Identifier: MIT
// Authenticode-signs one file. Tauri runs this as bundle.windows.signCommand once per file and prints its command
// line, so the certificate and its password come from the environment, and neither is ever printed here.
import { existsSync, readdirSync, statSync } from "node:fs"
import path from "node:path"

const USAGE = "usage: bun scripts/sign-windows.ts <file>"
const DEFAULT_TIMESTAMP = "http://timestamp.digicert.com"

function isFile(where: string | undefined): where is string {
  return where !== undefined && where !== "" && existsSync(where) && statSync(where).isFile()
}

// Newest first by dotted numbers; a folder whose name isn't a version sorts last
export function compareSdkVersions(a: string, b: string): number {
  const parse = (name: string) => (/^\d+(\.\d+)*$/.test(name) ? name.split(".").map(Number) : null)
  const left = parse(a)
  const right = parse(b)
  if (left === null || right === null) {
    if (left === right) return a.localeCompare(b)
    return left === null ? 1 : -1
  }
  for (let i = 0; i < Math.max(left.length, right.length); i++) {
    const diff = (right[i] ?? 0) - (left[i] ?? 0)
    if (diff !== 0) return diff
  }
  return 0
}

export function findSigntool(env: Record<string, string | undefined> = process.env): string | null {
  if (env.SIGNTOOL) return isFile(env.SIGNTOOL) ? env.SIGNTOOL : null
  const programFiles = env["ProgramFiles(x86)"]
  if (!programFiles) return null
  const bin = path.join(programFiles, "Windows Kits", "10", "bin")
  if (!existsSync(bin)) return null
  for (const version of readdirSync(bin).sort(compareSdkVersions)) {
    const candidate = path.join(bin, version, "x64", "signtool.exe")
    if (isFile(candidate)) return candidate
  }
  return null
}

function fail(message: string): never {
  console.error(message)
  process.exit(1)
}

function main(): void {
  const file = process.argv[2]
  if (file === undefined || file === "") {
    console.error(USAGE)
    process.exit(2)
  }
  const certificate = process.env.WINDOWS_CERTIFICATE_PATH
  if (!isFile(certificate)) fail("WINDOWS_CERTIFICATE_PATH is not set to the certificate file")
  const signtool = findSigntool()
  if (signtool === null) fail("signtool.exe not found: install a Windows 10/11 SDK")
  const timestamp = process.env.WINDOWS_TIMESTAMP_URL || DEFAULT_TIMESTAMP
  const password = process.env.WINDOWS_CERTIFICATE_PASSWORD ?? ""

  const argv = [signtool, "sign", "/q", "/fd", "sha256", "/td", "sha256", "/tr", timestamp, "/f", certificate]
  // A certificate file without a password takes no /p
  if (password !== "") argv.push("/p", password)
  argv.push(file)
  const child = Bun.spawnSync(argv, { stdio: ["ignore", "inherit", "inherit"] })
  if (child.exitCode !== 0) fail(`signtool failed on ${file} (exit ${child.exitCode ?? "signal"})`)
}

if (import.meta.main) main()
