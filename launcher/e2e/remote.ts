// SPDX-License-Identifier: MIT
// The manager's "Managed installs" screen against the remote-management stack that restricted's
// docker/remote/test.sh leaves running. Windows with WebView2 and Docker only; run by hand, never in CI.
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs"
import os from "node:os"
import path from "node:path"

import { restrictedDir } from "../scripts/restricted-data"
import { type Answer, type App, launch } from "./lib/app"
import { install, installer, uninstall } from "./lib/nsis"

const STEPS = 5
const SNAPSHOT_MS = 20_000
const CONTAINER_WORKSPACE = "/opt/reforged/.workspace"

// The controller's files the manager needs to act as it: config, secrets, the controller state and its TLS
const WORKSPACE_FILES = [
  "config/host.toml",
  "secrets.env",
  "remote-controller.json",
  "tls/gateway-cert.pem",
  "tls/gateway-key.pem",
  "tls/node-cert.pem",
  "tls/node-key.pem",
  "tls/cluster-ca-cert.pem",
  "tls/remote-cluster-ca.pem",
]

function step(n: number, text: string): void {
  console.log(`[${n}/${STEPS}] ${text}`)
}

function check(condition: unknown, message: string, answer?: Answer): asserts condition {
  if (!condition) throw new Error(answer === undefined ? message : `${message}: ${JSON.stringify(answer)}`)
}

async function ignore(work: () => unknown): Promise<void> {
  try {
    await work()
  } catch (error) {
    console.error(`cleanup: ${String(error)}`)
  }
}

function compose(restricted: string, args: string[]): Buffer {
  const argv = ["docker", "compose", "-p", "reforged-remote", "--profile", "remote"]
  argv.push("-f", path.join(restricted, "compose.yaml"), "exec", "-T", ...args)
  const child = Bun.spawnSync(argv, { stdout: "pipe", stderr: "inherit" })
  if (child.exitCode !== 0) throw new Error(`docker compose exec ${args.slice(0, 2).join(" ")} exited ${child.exitCode}`)
  return child.stdout
}

async function main(): Promise<void> {
  const restricted = restrictedDir()
  const root = mkdtempSync(path.join(os.tmpdir(), "dl-e2e-remote-"))
  const profile = path.join(root, "profile-server")
  const workspace = path.join(profile, "User Data", "Server")
  const installDir = path.join(root, "install-server")
  let manager: App | undefined
  let installed = false
  let failed = false

  try {
    step(1, `temporary root ${root}`)
    mkdirSync(workspace, { recursive: true })

    step(2, "finding the manager")
    let exe = path.join(restricted, "target", "release", "dainty-manager.exe")
    if (!existsSync(exe)) {
      const bundles =
        process.env.E2E_BUNDLE_DIR || path.join(restricted, "target", "release", "bundle", "nsis")
      mkdirSync(installDir, { recursive: true })
      installed = true
      exe = install(installer(bundles, "Dainty Lass Manager"), installDir, "dainty-manager.exe")
    }

    step(3, "copying the controller's workspace")
    // A sync first, so alpha's last contact falls inside the 60 s online window
    compose(restricted, ["host-alpha", "dainty-host", "remote", "sync"])
    // These hold keys: they stay in the temporary root and go with it
    for (const file of WORKSPACE_FILES) {
      const contents = compose(restricted, ["host-controller", "cat", `${CONTAINER_WORKSPACE}/${file}`])
      const target = path.join(workspace, ".workspace", ...file.split("/"))
      mkdirSync(path.dirname(target), { recursive: true })
      writeFileSync(target, contents)
    }

    step(4, "checking the managed installs")
    manager = await launch(exe, {
      profile,
      port: 9325,
      label: "manager",
      viewport: { width: 1280, height: 920 },
    })
    await manager.waitFor("get_snapshot", {}, (s) => s?.app?.serverGui === true, SNAPSHOT_MS)
    const remote = await manager.invoke("remote_status")
    check(remote?.ok === true, "remote_status failed", remote)
    const status = remote.status
    check(status?.role === "controller" && status.revision === 3, "controller at revision 3", status)
    check(status.installs?.length === 2, "two managed installs", status)
    const byNode = (node: string): Answer => status.installs.find((i: Answer) => i?.node === node)
    const alpha = byNode("alpha")
    check(alpha?.state === "online" && alpha.appliedRevision === 3, "alpha online at revision 3", alpha)
    const bravo = byNode("bravo")
    check(bravo?.revoked === true && bravo.state === "revoked", "bravo revoked", bravo)
    const invite = await manager.invoke("remote_invite", {
      request: { node: "charlie", region: "ap-east", label: "Install Charlie", url: "https://172.28.77.20" },
    })
    check(invite?.ok === true, "remote_invite failed", invite)
    check(String(invite.invite?.uri).startsWith("dainty-lass-remote://enroll?"), "enrollment URI", invite.invite)

    step(5, "screenshots of the managed-installs panel")
    await manager.tab(0)
    const page = manager.page
    await page.locator("#rem-inv-node").fill("delta")
    await page.locator("#rem-inv-region").fill("ap-east")
    await page.locator("#rem-inv-label").fill("Install Delta")
    await page.locator("#rem-inv-node").scrollIntoViewIfNeeded()
    await manager.shots("server-gui-managed")
    await manager.close()
  } catch (error) {
    failed = true
    console.error(error instanceof Error ? (error.stack ?? error.message) : String(error))
  } finally {
    await ignore(() => manager?.close())
    if (installed) await ignore(() => uninstall(installDir))
    await ignore(() => rmSync(root, { recursive: true, force: true }))
  }
  if (failed) process.exit(1)
}

await main()
