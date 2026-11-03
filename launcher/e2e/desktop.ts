// SPDX-License-Identifier: MIT
// Installs both desktop bundles, hosts a server from the manager, joins it from the client, then bans the player
// and reads the history. Windows with WebView2 and the built bundles only; run by hand, never in CI.
import { mkdirSync, mkdtempSync, rmSync } from "node:fs"
import os from "node:os"
import path from "node:path"

import { restrictedDir } from "../scripts/restricted-data"
import { type Answer, type App, launch } from "./lib/app"
import { install, installer, uninstall } from "./lib/nsis"

const STEPS = 8
const SERVER_NAME = "Desktop E2E Hunts"
const START_MS = 15_000
const SNAPSHOT_MS = 20_000

function step(n: number, text: string): void {
  console.log(`[${n}/${STEPS}] ${text}`)
}

function check(condition: unknown, message: string, answer?: Answer): asserts condition {
  if (!condition) throw new Error(answer === undefined ? message : `${message}: ${JSON.stringify(answer)}`)
}

async function ok(app: App, cmd: string, args: Record<string, unknown> = {}): Promise<Answer> {
  const answer = await app.invoke(cmd, args)
  check(answer?.ok === true, `${app.label}: ${cmd} failed`, answer)
  return answer
}

async function ignore(work: () => unknown): Promise<void> {
  try {
    await work()
  } catch (error) {
    console.error(`cleanup: ${String(error)}`)
  }
}

async function clickButton(app: App, name: string): Promise<void> {
  await app.page.getByRole("button", { name }).first().click()
}

async function main(): Promise<void> {
  const root = mkdtempSync(path.join(os.tmpdir(), "dl-e2e-desktop-"))
  const installClient = path.join(root, "install-client")
  const installServer = path.join(root, "install-server")
  const profileClient = path.join(root, "profile-client")
  const profileServer = path.join(root, "profile-server")
  let manager: App | undefined
  let client: App | undefined
  let failed = false

  try {
    step(1, `temporary root ${root}`)
    for (const dir of [installClient, installServer]) mkdirSync(dir, { recursive: true })
    for (const dir of [profileClient, profileServer]) mkdirSync(path.join(dir, "User Data"), { recursive: true })

    step(2, "installing both bundles")
    const bundles =
      process.env.E2E_BUNDLE_DIR || path.join(restrictedDir(), "target", "release", "bundle", "nsis")
    const clientExe = install(installer(bundles, "DaintyLass Launcher"), installClient, "dainty-lass-client.exe")
    const managerExe = install(installer(bundles, "DaintyLass Manager"), installServer, "dainty-lass-manager.exe")

    step(3, "launching the manager and the client")
    manager = await launch(managerExe, { profile: profileServer, port: 9321, label: "manager" })
    client = await launch(clientExe, { profile: profileClient, port: 9322, label: "client" })
    await manager.waitFor("get_snapshot", {}, (s) => s?.app?.serverGui === true, SNAPSHOT_MS)
    const kit = await manager.invoke("host_get")
    check(typeof kit?.workspace === "string" && kit.workspace.endsWith("Server"), "manager workspace", kit)
    check(kit.workspaceOk === true, "manager workspace not ok", kit)
    const download = await manager.invoke("host_download_project")
    check(download?.ok === false && download.error?.code === "not_in_this_build", "host_download_project", download)
    check((await manager.page.title()) === "DaintyLass Manager", "manager title")
    const first = await client.invoke("get_snapshot")
    check(first?.app?.serverGui === false && first.phase === "join", "client snapshot", first)
    await client.shots("client-join")
    const setupPanel = manager.page.locator("main > div").first()
    await setupPanel.evaluate((el) => {
      el.scrollTop = 620
    })
    await manager.shots("server-gui-setup")
    await setupPanel.evaluate((el) => {
      el.scrollTop = 0
    })

    step(4, "hosting a server from the manager")
    await ok(manager, "host_action", { action: "synthetic_data" })
    await ok(manager, "host_setup", { request: { name: SERVER_NAME, mode: "private", publicHost: null } })
    const config = await ok(manager, "server_config_get")
    check(config.config?.configured === true && config.config.name === SERVER_NAME, "server config", config)
    const doctor = await ok(manager, "server_doctor")
    check((doctor.report?.checks?.length ?? 0) > 0, "server_doctor returned no checks", doctor)
    await ok(manager, "host_server_start")
    const started = await manager.waitFor("host_server_get", {}, (v) => v?.state !== "starting", START_MS)
    check(started.state === "running", "server not running", started)
    await ok(manager, "host_create_invite", { forName: "Slayer42" })
    const view = await manager.invoke("host_server_get")
    const link: unknown = view?.invite?.link
    check(typeof link === "string" && link !== "" && view.invite.for === "Slayer42", "invite in the server view")
    const invites = await ok(manager, "server_invite_list")
    check(invites.invites?.length === 1 && typeof invites.invites[0]?.code === "string", "invite list", invites)
    await manager.tab(0)
    await clickButton(manager, "Run doctor")
    await clickButton(manager, "Refresh invites")
    for (const [index, name] of [
      [0, "server"],
      [1, "status"],
      [2, "modules"],
      [5, "gamedata"],
      [6, "build"],
      [7, "requirements"],
    ] as const) {
      await manager.tab(index)
      await manager.shots(`server-gui-${name}`)
    }

    step(5, "joining from the client")
    await ok(client, "check_invite", { text: link })
    await ok(client, "submit_invite", { text: link, acceptNewCertificate: false, carryAccount: false })
    await client.shots("client-register")
    const registered = await ok(client, "register", { username: "Slayer_42" })
    check(registered.username === "Slayer_42", "registered username", registered)
    await client.invoke("dismiss_backup_offer")
    const joined = await client.waitFor(
      "get_snapshot",
      {},
      (s) => s?.account?.username === "Slayer_42" && s.account.hasKey === true && s.status?.online === true,
      SNAPSHOT_MS
    )
    check(joined.status.name === SERVER_NAME, "client sees the server's name", joined.status)
    for (const name of ["play", "realms", "server", "game", "settings"]) {
      await client.view(name)
      await client.shots(`client-${name}`)
    }
    await client.view("play")

    step(6, "banning the player")
    const ban = await ok(manager, "admin_action", {
      request: { action: "ban", player: "Slayer_42", reason: "Desktop E2E ban check", until: null, module: null },
    })
    check(ban.result?.player?.banned === true, "player not banned", ban)
    await manager.tab(3)
    await manager.page.locator("#admin-player").fill("Slayer_42")
    await clickButton(manager, "Look up")
    await manager.shots("server-gui-admin")
    await client.invoke("refresh_status")
    await client.waitFor("get_snapshot", {}, (s) => s?.lastError?.code === "account_banned", SNAPSHOT_MS)
    await client.shots("client-banned")

    step(7, "history, backup and stop")
    const history = await ok(manager, "history_query", {
      filter: { account: null, actor: null, since: null, until: null, cursor: null },
    })
    check((history.page?.entries?.length ?? 0) > 0, "history has no entries", history)
    await manager.tab(4)
    await manager.shots("server-gui-history")
    await ok(manager, "server_backup")
    await ok(manager, "host_server_stop")
    const stopped = await manager.invoke("host_server_get")
    check(stopped?.state === "stopped", "server not stopped", stopped)

    step(8, "closing both apps")
    await client.close()
    await manager.close()
  } catch (error) {
    failed = true
    console.error(error instanceof Error ? (error.stack ?? error.message) : String(error))
  } finally {
    await ignore(() => client?.close())
    await ignore(() => manager?.close())
    await ignore(() => uninstall(installClient))
    await ignore(() => uninstall(installServer))
    await ignore(() => rmSync(root, { recursive: true, force: true }))
  }
  if (failed) process.exit(1)
}

await main()
