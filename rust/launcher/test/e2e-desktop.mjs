#!/usr/bin/env node
// Opt-in end-to-end desktop test for dainty-lass-client and dainty-lass-server-gui on Windows (B4).
// Installs both NSIS bundles into temporary directories, launches both apps with WebView2 CDP debugging
// enabled and isolated REFORGED_DEV_PROFILE directories, drives the full server setup -> start -> invite ->
// client join -> register -> status -> ban -> history -> backup -> stop flow over WebView2 CDP, and saves
// screenshots of every screen in English and Brazilian Portuguese under docs/port/screenshots/.

import assert from "node:assert/strict"
import { spawn, spawnSync } from "node:child_process"
import fs from "node:fs"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const REPO_ROOT = path.resolve(__dirname, "../../..")
const BUNDLE_DIR = path.join(REPO_ROOT, "rust/target/release/bundle/nsis")
const SCREENSHOT_DIR = path.join(REPO_ROOT, "docs/port/screenshots")

const CLIENT_INSTALLER = path.join(
  BUNDLE_DIR,
  "DaintyLass Launcher_2.0.0-alpha.0_x64-setup.exe"
)
const SERVER_INSTALLER = path.join(
  BUNDLE_DIR,
  "DaintyLass Server_2.0.0-alpha.0_x64-setup.exe"
)

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

async function connectCdp(port, label) {
  let wsUrl = null
  for (let i = 0; i < 120; i++) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/json/list`)
      if (res.ok) {
        const targets = await res.json()
        const main = targets.find(
          (t) =>
            t.type === "page" &&
            typeof t.url === "string" &&
            t.url.startsWith("http://tauri.localhost") &&
            !t.url.includes("__tauri_isolation__")
        )
        if (main?.webSocketDebuggerUrl) {
          wsUrl = main.webSocketDebuggerUrl
          break
        }
      }
    } catch {
      // Waiting for WebView2 CDP server
    }
    await sleep(250)
  }
  assert.ok(wsUrl, `timed out waiting for ${label} WebView2 CDP on port ${port}`)

  const ws = new WebSocket(wsUrl)
  await new Promise((resolve, reject) => {
    ws.addEventListener("open", resolve, { once: true })
    ws.addEventListener("error", reject, { once: true })
  })

  let nextId = 1
  const pending = new Map()
  ws.addEventListener("message", (event) => {
    const msg = JSON.parse(String(event.data))
    if (msg.id && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id)
      pending.delete(msg.id)
      if (msg.error) {
        reject(new Error(`${label} CDP error: ${JSON.stringify(msg.error)}`))
      } else {
        resolve(msg.result)
      }
    }
  })

  async function send(method, params = {}) {
    const id = nextId++
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject })
      ws.send(JSON.stringify({ id, method, params }))
    })
  }

  async function evaluate(expression) {
    const res = await send("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
    })
    if (res.exceptionDetails) {
      throw new Error(
        `${label} evaluate failed: ${res.exceptionDetails.text} ${res.exceptionDetails.exception?.description ?? ""}`
      )
    }
    return res.result?.value
  }

  // Wait until Tauri's bridge and isolation iframe are ready in http://tauri.localhost/
  let ready = false
  for (let i = 0; i < 80; i++) {
    try {
      const ok = await evaluate(
        "typeof window.__TAURI_INTERNALS__?.invoke === 'function' && location.href.startsWith('http://tauri.localhost')"
      )
      if (ok === true) {
        ready = true
        break
      }
    } catch {
      // Context may clear while navigating from about:blank
    }
    await sleep(200)
  }
  assert.ok(ready, `${label} __TAURI_INTERNALS__.invoke did not become ready`)

  async function invoke(cmd, args = {}) {
    const expr = `new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("invoke timeout for ${cmd}")), 15000);
      window.__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args)}).then(
        (v) => { clearTimeout(timer); resolve(v); },
        (e) => { clearTimeout(timer); reject(e); }
      );
    })`
    return evaluate(expr)
  }

  async function screenshot(filename) {
    await sleep(350)
    const { data } = await send("Page.captureScreenshot", { format: "png" })
    const file = path.join(SCREENSHOT_DIR, filename)
    fs.writeFileSync(file, Buffer.from(data, "base64"))
    console.log(`  [screenshot] saved ${filename}`)
  }

  async function setLanguage(lang) {
    await invoke("set_settings", { patch: { language: lang } })
    await sleep(250)
  }

  async function clickView(view) {
    await evaluate(`(() => {
      const btn = document.querySelector('[data-view="${view}"]');
      if (!btn) throw new Error('missing nav button for ${view}');
      btn.click();
    })()`)
    await sleep(250)
  }

  async function clickHostTabIndex(index) {
    await evaluate(`(() => {
      const tabs = Array.from(document.querySelectorAll('[role="tab"]'));
      const tab = tabs[${index}];
      if (!tab) throw new Error('missing host tab index ${index}');
      tab.click();
    })()`)
    await sleep(300)
  }

  return {
    send,
    evaluate,
    invoke,
    screenshot,
    setLanguage,
    clickView,
    clickHostTabIndex,
    close: () => ws.close(),
  }
}

function installNsisBundle(installerPath, targetDir, expectedExe) {
  assert.ok(fs.existsSync(installerPath), `missing installer: ${installerPath}`)
  fs.mkdirSync(targetDir, { recursive: true })
  // NSIS requires /D= to be the last parameter without quotes
  const res = spawnSync(installerPath, ["/S", `/D=${targetDir}`], {
    stdio: "inherit",
    windowsHide: true,
  })
  assert.equal(res.status, 0, `NSIS installer failed for ${installerPath}`)
  const exePath = path.join(targetDir, expectedExe)
  assert.ok(fs.existsSync(exePath), `expected installed binary at ${exePath}`)
  return exePath
}

async function main() {
  fs.mkdirSync(SCREENSHOT_DIR, { recursive: true })
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "dl-e2e-desktop-"))
  console.log(`[1/8] Using temporary E2E root: ${tempRoot}`)

  const clientInstallDir = path.join(tempRoot, "install-client")
  const serverInstallDir = path.join(tempRoot, "install-server")
  const clientProfileDir = path.join(tempRoot, "profile-client")
  const serverProfileDir = path.join(tempRoot, "profile-server")
  const serverWorkspaceDir = path.join(tempRoot, "server-workspace")

  fs.mkdirSync(path.join(clientProfileDir, "User Data"), { recursive: true })
  fs.mkdirSync(path.join(serverProfileDir, "User Data"), { recursive: true })
  fs.mkdirSync(serverWorkspaceDir, { recursive: true })

  // Pre-seed the server workspace path in the server GUI's isolated settings.json so no OS folder dialog blocks
  fs.writeFileSync(
    path.join(serverProfileDir, "User Data", "settings.json"),
    JSON.stringify({ hostWorkspace: serverWorkspaceDir, hostTools: true }, null, 2)
  )

  console.log("[2/8] Installing both NSIS bundles into isolated directories...")
  const clientExe = installNsisBundle(
    CLIENT_INSTALLER,
    clientInstallDir,
    "dainty-lass-client.exe"
  )
  const serverExe = installNsisBundle(
    SERVER_INSTALLER,
    serverInstallDir,
    "dainty-lass-server-gui.exe"
  )
  console.log(`  Installed client: ${clientExe}`)
  console.log(`  Installed server: ${serverExe}`)

  let serverProc = null
  let clientProc = null
  let serverCdp = null
  let clientCdp = null

  try {
    console.log("[3/8] Launching dainty-lass-server-gui and dainty-lass-client with WebView2 CDP...")
    serverProc = spawn(serverExe, [], {
      env: {
        ...process.env,
        REFORGED_DEV_PROFILE: serverProfileDir,
        WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-port=9321",
      },
      stdio: "ignore",
      windowsHide: false,
    })
    clientProc = spawn(clientExe, [], {
      env: {
        ...process.env,
        REFORGED_DEV_PROFILE: clientProfileDir,
        WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-port=9322",
      },
      stdio: "ignore",
      windowsHide: false,
    })

    serverCdp = await connectCdp(9321, "server-gui")
    clientCdp = await connectCdp(9322, "client")
    await serverCdp.send("Page.enable")
    await clientCdp.send("Page.enable")

    // Wait for initial snapshots to settle
    for (let i = 0; i < 40; i++) {
      const snap = await serverCdp.invoke("get_snapshot")
      if (snap?.app?.serverGui === true) break
      await sleep(200)
    }

    const serverSnap = await serverCdp.invoke("get_snapshot")
    assert.equal(serverSnap.app.serverGui, true, "server-gui must set app.serverGui = true")
    const clientSnap = await clientCdp.invoke("get_snapshot")
    assert.equal(clientSnap.app.serverGui, false, "client must set app.serverGui = false")
    assert.equal(clientSnap.phase, "join", "fresh client starts on join phase")

    // Capture initial Join screen in EN and pt-BR
    await clientCdp.setLanguage("en")
    await clientCdp.screenshot("client-join-en.png")
    await clientCdp.setLanguage("pt-BR")
    await clientCdp.screenshot("client-join-pt-BR.png")
    await clientCdp.setLanguage("en")

    // Capture initial Server GUI unconfigured setup state in EN and pt-BR
    await serverCdp.setLanguage("en")
    await serverCdp.screenshot("server-gui-setup-en.png")
    await serverCdp.setLanguage("pt-BR")
    await serverCdp.screenshot("server-gui-setup-pt-BR.png")
    await serverCdp.setLanguage("en")

    console.log("[4/8] Setting up server workspace with embedded example game data and starting server...")
    const synRes = await serverCdp.invoke("host_action", { action: "synthetic_data" })
    assert.equal(synRes.ok, true, `synthetic_data failed: ${JSON.stringify(synRes)}`)

    const setupRes = await serverCdp.invoke("host_setup", {
      request: {
        name: "Desktop E2E Hunts",
        mode: "private",
        publicHost: null,
      },
    })
    assert.equal(setupRes.ok, true, `host_setup failed: ${JSON.stringify(setupRes)}`)

    const cfgRes = await serverCdp.invoke("server_config_get")
    assert.equal(cfgRes.ok, true, `server_config_get failed: ${JSON.stringify(cfgRes)}`)
    assert.equal(cfgRes.config.configured, true)
    assert.equal(cfgRes.config.name, "Desktop E2E Hunts")

    const docRes = await serverCdp.invoke("server_doctor")
    assert.equal(docRes.ok, true, `server_doctor failed: ${JSON.stringify(docRes)}`)
    assert.ok(docRes.report.checks.length > 0, "expected doctor checks")

    const startRes = await serverCdp.invoke("host_server_start")
    assert.equal(startRes.ok, true, `host_server_start failed: ${JSON.stringify(startRes)}`)

    let srvView = null
    for (let i = 0; i < 60; i++) {
      srvView = await serverCdp.invoke("host_server_get")
      if (srvView.state !== "starting") break
      await sleep(250)
    }
    assert.equal(srvView?.state, "running", `server did not reach running: ${JSON.stringify(srvView)}`)

    const inviteRes = await serverCdp.invoke("host_create_invite", { forName: "Slayer42" })
    assert.equal(inviteRes.ok, true, `host_create_invite failed: ${JSON.stringify(inviteRes)}`)
    srvView = await serverCdp.invoke("host_server_get")
    const inviteLink = srvView.invite?.link
    assert.ok(inviteLink, "expected invite link after host_create_invite")
    assert.equal(srvView.invite?.for, "Slayer42")

    const inviteList = await serverCdp.invoke("server_invite_list")
    assert.equal(inviteList.ok, true)
    assert.equal(inviteList.invites.length, 1)
    assert.ok(inviteList.invites[0].code, "expected invite code in server_invite_list")

    // Trigger doctor & invite list in the UI so the Server tab displays them
    await serverCdp.clickHostTabIndex(0)
    await serverCdp.evaluate(`(() => {
      const buttons = Array.from(document.querySelectorAll('button'));
      buttons.find(b => b.textContent?.includes('Run doctor'))?.click();
      buttons.find(b => b.textContent?.includes('Refresh invites'))?.click();
    })()`)
    await sleep(600)

    // Capture Server GUI tabs (0=server, 1=status, 2=modules, 5=gamedata, 6=build, 7=requirements) in EN and pt-BR
    const serverTabs = [
      [0, "server"],
      [1, "status"],
      [2, "modules"],
      [5, "gamedata"],
      [6, "build"],
      [7, "requirements"],
    ]
    for (const [idx, name] of serverTabs) {
      await serverCdp.setLanguage("en")
      await serverCdp.clickHostTabIndex(idx)
      await serverCdp.screenshot(`server-gui-${name}-en.png`)
      await serverCdp.setLanguage("pt-BR")
      await serverCdp.screenshot(`server-gui-${name}-pt-BR.png`)
    }
    await serverCdp.setLanguage("en")

    console.log("[5/8] Joining server from client via invite, registering Slayer_42, and verifying status...")
    const checkInv = await clientCdp.invoke("check_invite", { text: inviteLink })
    assert.equal(checkInv.ok, true, `check_invite failed: ${JSON.stringify(checkInv)}`)

    const subInv = await clientCdp.invoke("submit_invite", {
      text: inviteLink,
      acceptNewCertificate: false,
      carryAccount: false,
    })
    assert.equal(subInv.ok, true, `submit_invite failed: ${JSON.stringify(subInv)}`)

    // Capture Register screen in EN and pt-BR
    await clientCdp.setLanguage("en")
    await clientCdp.screenshot("client-register-en.png")
    await clientCdp.setLanguage("pt-BR")
    await clientCdp.screenshot("client-register-pt-BR.png")
    await clientCdp.setLanguage("en")

    const regRes = await clientCdp.invoke("register", { username: "Slayer_42" })
    assert.equal(regRes.ok, true, `register failed: ${JSON.stringify(regRes)}`)
    assert.equal(regRes.username, "Slayer_42")

    // Dismiss the key backup offer so we can capture all client screens cleanly
    await clientCdp.invoke("dismiss_backup_offer")
    await sleep(300)

    const afterReg = await clientCdp.invoke("get_snapshot")
    assert.equal(afterReg.account.username, "Slayer_42")
    assert.equal(afterReg.account.hasKey, true)
    assert.equal(afterReg.status?.online, true)
    assert.equal(afterReg.status?.name, "Desktop E2E Hunts")

    // Capture Client screens (Play, Realms, Server, Game, Settings) in EN and pt-BR
    for (const view of ["play", "realms", "server", "game", "settings"]) {
      await clientCdp.setLanguage("en")
      await clientCdp.clickView(view)
      await clientCdp.screenshot(`client-${view}-en.png`)
      await clientCdp.setLanguage("pt-BR")
      await clientCdp.screenshot(`client-${view}-pt-BR.png`)
    }
    await clientCdp.setLanguage("en")
    await clientCdp.clickView("play")

    console.log("[6/8] Banning Slayer_42 from Server GUI and verifying Client shows AccountBanned...")
    const banRes = await serverCdp.invoke("admin_action", {
      request: {
        action: "ban",
        player: "Slayer_42",
        reason: "Desktop E2E ban check",
        until: null,
        module: null,
      },
    })
    assert.equal(banRes.ok, true, `admin_action ban failed: ${JSON.stringify(banRes)}`)
    assert.equal(banRes.result.player?.banned, true)

    // Switch Server GUI to Admin tab (index 3), look up Slayer_42, and capture in EN and pt-BR
    await serverCdp.setLanguage("en")
    await serverCdp.clickHostTabIndex(3)
    await serverCdp.evaluate(`(() => {
      const input = document.querySelector('#admin-player');
      if (input) {
        const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
        setter.call(input, 'Slayer_42');
        input.dispatchEvent(new Event('input', { bubbles: true }));
      }
      const btn = Array.from(document.querySelectorAll('button')).find(b => b.textContent?.includes('Look up'));
      btn?.click();
    })()`)
    await sleep(500)
    await serverCdp.screenshot("server-gui-admin-en.png")
    await serverCdp.setLanguage("pt-BR")
    await serverCdp.screenshot("server-gui-admin-pt-BR.png")
    await serverCdp.setLanguage("en")

    // Refresh status on the client and verify it shows the ban
    await clientCdp.invoke("refresh_status")
    await sleep(400)
    const bannedSnap = await clientCdp.invoke("get_snapshot")
    assert.equal(
      bannedSnap.lastError?.code,
      "account_banned",
      `expected account_banned on client, got: ${JSON.stringify(bannedSnap.lastError)}`
    )
    await clientCdp.setLanguage("en")
    await clientCdp.screenshot("client-banned-en.png")
    await clientCdp.setLanguage("pt-BR")
    await clientCdp.screenshot("client-banned-pt-BR.png")
    await clientCdp.setLanguage("en")

    console.log("[7/8] Querying change-log history, backing up database, and stopping server cleanly...")
    const histRes = await serverCdp.invoke("history_query", {
      filter: {
        account: null,
        actor: null,
        since: null,
        until: null,
        cursor: null,
      },
    })
    assert.equal(histRes.ok, true, `history_query failed: ${JSON.stringify(histRes)}`)
    assert.ok(histRes.page.entries.length > 0, "expected history entries after registration and ban")

    // Switch Server GUI to History tab (index 4) and capture in EN and pt-BR
    await serverCdp.setLanguage("en")
    await serverCdp.clickHostTabIndex(4)
    await sleep(500)
    await serverCdp.screenshot("server-gui-history-en.png")
    await serverCdp.setLanguage("pt-BR")
    await serverCdp.screenshot("server-gui-history-pt-BR.png")
    await serverCdp.setLanguage("en")

    const backupRes = await serverCdp.invoke("server_backup")
    assert.equal(backupRes.ok, true, `server_backup failed: ${JSON.stringify(backupRes)}`)

    const stopRes = await serverCdp.invoke("host_server_stop")
    assert.equal(stopRes.ok, true, `host_server_stop failed: ${JSON.stringify(stopRes)}`)
    const stoppedView = await serverCdp.invoke("host_server_get")
    assert.equal(stoppedView.state, "stopped", "server must be stopped after host_server_stop")

    console.log("[8/8] Closing both apps cleanly...")
    await sleep(300)
  } finally {
    try {
      clientCdp?.close()
    } catch {}
    try {
      serverCdp?.close()
    } catch {}
    if (clientProc && clientProc.exitCode === null) {
      clientProc.kill()
    }
    if (serverProc && serverProc.exitCode === null) {
      serverProc.kill()
    }
    await sleep(500)
    try {
      fs.rmSync(tempRoot, { recursive: true, force: true })
    } catch {}
  }

  console.log("E2E desktop test completed successfully.")
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
