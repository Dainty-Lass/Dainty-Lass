#!/usr/bin/env node
// End-to-end desktop test for DaintyLass Manager's "Managed installs" screen (R3, RM1-RM5)
// against the live Docker remote-management controller (`docker/remote/test.sh`).
// Copies the live Docker controller workspace (`host-controller:/opt/reforged/.workspace`) into an
// isolated DaintyLass Manager profile's `Server` workspace, launches `dainty-lass-manager.exe` with
// WebView2 CDP, verifies `remote_get_status` returns the enrolled installs (`alpha` online at rev 2
// with the completed ban outcome and `bravo` revoked), drives the "Managed installs" tab, and saves
// screenshots in both English (`server-gui-managed-en.png`) and Brazilian Portuguese
// (`server-gui-managed-pt-BR.png`) under `docs/port/screenshots/`.

import assert from "node:assert/strict"
import { execFileSync, spawn, spawnSync } from "node:child_process"
import fs from "node:fs"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const REPO_ROOT = path.resolve(__dirname, "../../..")
const SCREENSHOT_DIR = path.join(REPO_ROOT, "docs/port/screenshots")

const RELEASE_MANAGER_EXE = path.join(REPO_ROOT, "rust/target/release/dainty-lass-manager.exe")
const NSIS_MANAGER_INSTALLER = path.join(
  REPO_ROOT,
  "rust/target/release/bundle/nsis/DaintyLass Manager_2.0.0-alpha.0_x64-setup.exe"
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
    } catch {}
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
    await sleep(400)
    const { data } = await send("Page.captureScreenshot", { format: "png" })
    const file = path.join(SCREENSHOT_DIR, filename)
    fs.writeFileSync(file, Buffer.from(data, "base64"))
    console.log(`  [screenshot] saved ${filename}`)
  }

  async function setLanguage(lang) {
    await invoke("set_settings", { patch: { language: lang } })
    await sleep(300)
  }

  async function clickHostTabIndex(index) {
    await evaluate(`(() => {
      const tabs = Array.from(document.querySelectorAll('[role="tab"]'));
      const tab = tabs[${index}];
      if (!tab) throw new Error('missing host tab index ${index}');
      tab.click();
    })()`)
    await sleep(400)
  }

  return {
    send,
    evaluate,
    invoke,
    screenshot,
    setLanguage,
    clickHostTabIndex,
    close: () => ws.close(),
  }
}

function syncDockerControllerWorkspace(serverRoot) {
  const wsConfig = path.join(serverRoot, ".workspace/config")
  const wsTls = path.join(serverRoot, ".workspace/tls")
  fs.mkdirSync(wsConfig, { recursive: true })
  fs.mkdirSync(wsTls, { recursive: true })

  const composeArgs = ["-p", "reforged-remote", "--profile", "remote", "-f", "compose.yaml"]
  // Trigger a fresh sync on alpha right before snapshotting the controller workspace so alpha's
  // last_seen_ms is within the 60s online window.
  execFileSync(
    "docker",
    ["compose", ...composeArgs, "exec", "-T", "host-alpha", "reforged-host", "remote", "sync"],
    { cwd: REPO_ROOT, encoding: "utf8" }
  )
  const readFromController = (remotePath) =>
    execFileSync(
      "docker",
      ["compose", ...composeArgs, "exec", "-T", "host-controller", "cat", remotePath],
      { cwd: REPO_ROOT, encoding: "utf8" }
    )

  for (const name of ["host.toml", "secrets.env", "remote-controller.json"]) {
    const content = readFromController(`/opt/reforged/.workspace/config/${name}`)
    fs.writeFileSync(path.join(wsConfig, name), content, "utf8")
  }
  for (const name of [
    "gateway-cert.pem",
    "gateway-key.pem",
    "node-cert.pem",
    "node-key.pem",
    "cluster-ca-cert.pem",
    "remote-cluster-ca.pem",
  ]) {
    const content = readFromController(`/opt/reforged/.workspace/tls/${name}`)
    fs.writeFileSync(path.join(wsTls, name), content, "utf8")
  }
}

async function main() {
  fs.mkdirSync(SCREENSHOT_DIR, { recursive: true })
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "dl-e2e-remote-"))
  console.log(`[1/4] Using temporary E2E root: ${tempRoot}`)

  const serverProfileDir = path.join(tempRoot, "profile-server")
  const serverWorkspaceDir = path.join(serverProfileDir, "User Data", "Server")
  fs.mkdirSync(serverWorkspaceDir, { recursive: true })

  let managerExe = RELEASE_MANAGER_EXE
  let installedDir = null
  if (!fs.existsSync(managerExe)) {
    assert.ok(
      fs.existsSync(NSIS_MANAGER_INSTALLER),
      `neither ${RELEASE_MANAGER_EXE} nor ${NSIS_MANAGER_INSTALLER} exists`
    )
    installedDir = path.join(tempRoot, "install-manager")
    fs.mkdirSync(installedDir, { recursive: true })
    const res = spawnSync(NSIS_MANAGER_INSTALLER, ["/S", `/D=${installedDir}`], {
      stdio: "inherit",
      windowsHide: true,
    })
    assert.equal(res.status, 0, "NSIS installer failed")
    managerExe = path.join(installedDir, "dainty-lass-manager.exe")
  }

  console.log("[2/4] Copying live Docker controller workspace into DaintyLass Manager profile...")
  syncDockerControllerWorkspace(serverWorkspaceDir)

  let serverProc = null
  let serverCdp = null
  try {
    console.log("[3/4] Launching dainty-lass-manager with WebView2 CDP...")
    serverProc = spawn(managerExe, [], {
      env: {
        ...process.env,
        REFORGED_DEV_PROFILE: serverProfileDir,
        WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-port=9325",
      },
      stdio: "ignore",
      windowsHide: false,
    })

    serverCdp = await connectCdp(9325, "server-gui-remote")
    await serverCdp.send("Page.enable")
    await serverCdp.send("Emulation.setDeviceMetricsOverride", {
      width: 1280,
      height: 920,
      deviceScaleFactor: 1,
      mobile: false,
    })

    for (let i = 0; i < 40; i++) {
      const snap = await serverCdp.invoke("get_snapshot")
      if (snap?.app?.serverGui === true) break
      await sleep(200)
    }

    const remoteRes = await serverCdp.invoke("remote_status")
    assert.equal(remoteRes.ok, true, `remote_status failed: ${JSON.stringify(remoteRes)}`)
    assert.equal(remoteRes.status.role, "controller")
    assert.equal(remoteRes.status.revision, 3)
    assert.equal(remoteRes.status.installs.length, 2)
    const alpha = remoteRes.status.installs.find((i) => i.node === "alpha")
    const bravo = remoteRes.status.installs.find((i) => i.node === "bravo")
    assert.ok(alpha, "expected enrolled install alpha")
    assert.equal(alpha.state, "online")
    assert.equal(alpha.appliedRevision, 3)
    assert.ok(bravo, "expected enrolled install bravo")
    assert.equal(bravo.revoked, true)
    assert.equal(bravo.state, "revoked")

    // Mint an invite in the UI so the single-use invite box is also visible in the screenshot
    const invRes = await serverCdp.invoke("remote_invite", {
      request: {
        node: "charlie",
        region: "ap-east",
        label: "Install Charlie",
        url: "https://172.28.77.20",
      },
    })
    assert.equal(invRes.ok, true, `remote_invite failed: ${JSON.stringify(invRes)}`)
    assert.ok(invRes.invite.uri.startsWith("dainty-lass-remote://enroll?"))

    console.log("[4/4] Navigating to Managed installs panel on Server tab and capturing EN + pt-BR screenshots...")
    await serverCdp.setLanguage("en")
    await serverCdp.clickHostTabIndex(0)
    await serverCdp.evaluate(`(() => {
      const nodeInput = document.querySelector('#rem-inv-node');
      const regionInput = document.querySelector('#rem-inv-region');
      const labelInput = document.querySelector('#rem-inv-label');
      const setVal = (el, val) => {
        if (!el) return;
        const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
        setter.call(el, val);
        el.dispatchEvent(new Event('input', { bubbles: true }));
      };
      setVal(nodeInput, 'delta');
      setVal(regionInput, 'ap-east');
      setVal(labelInput, 'Install Delta');
    })()`)
    await sleep(200)
    await serverCdp.evaluate(`(() => {
      const btn = document.querySelector('[data-action="remote-invite"]');
      if (btn) btn.click();
    })()`)
    await sleep(500)
    await serverCdp.evaluate(`(() => {
      const panel = document.querySelector('[data-remote-role]')?.closest('section') ?? document.querySelector('[data-remote-role]');
      if (panel) panel.scrollIntoView({ behavior: 'instant', block: 'start' });
    })()`)
    await sleep(400)
    await serverCdp.screenshot("server-gui-managed-en.png")

    await serverCdp.setLanguage("pt-BR")
    await sleep(400)
    await serverCdp.evaluate(`(() => {
      const panel = document.querySelector('[data-remote-role]')?.closest('section') ?? document.querySelector('[data-remote-role]');
      if (panel) panel.scrollIntoView({ behavior: 'instant', block: 'start' });
    })()`)
    await sleep(400)
    await serverCdp.screenshot("server-gui-managed-pt-BR.png")
    await serverCdp.setLanguage("en")
  } finally {
    try {
      serverCdp?.close()
    } catch {}
    if (serverProc && serverProc.exitCode === null) {
      serverProc.kill()
    }
    await sleep(400)
    if (installedDir) {
      const uninstaller = path.join(installedDir, "uninstall.exe")
      if (fs.existsSync(uninstaller)) {
        spawnSync(uninstaller, ["/S", `_?=${installedDir}`], {
          stdio: "ignore",
          windowsHide: true,
        })
      }
    }
    try {
      fs.rmSync(tempRoot, { recursive: true, force: true })
    } catch {}
  }

  console.log("Remote Management Desktop E2E completed successfully.")
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
