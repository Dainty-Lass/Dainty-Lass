/// <reference types="bun" />
// What the screens show for a Snapshot: the PLAY button, the steps, the progress and the error text
import { expect, test } from "bun:test"

import type { ErrorCode, Snapshot } from "@workspace/bridge"
import { CATALOGS, isKey } from "@workspace/i18n"

import { formatBytes, formatPercentText } from "../src/app/format"
import {
  checkUsername,
  ERROR_KEYS,
  errorMessage,
  hintKey,
  inviteSummary,
  NOTICE_KEYS,
  primaryButton,
  shortHex,
  shownError,
  STEP_KEYS,
  stepIndex,
  taskView,
} from "../src/app/model"

function snap(over: Partial<Snapshot> = {}): Snapshot {
  return {
    phase: "join",
    busy: false,
    server: null,
    savedServers: [],
    connect: {
      checking: false,
      problem: null,
      tailscaleInstalled: true,
      lastCheckedAt: null,
    },
    account: { username: null, hasKey: false, offerBackup: false },
    game: {
      root: null,
      problem: "not_set",
      versionText: null,
      versionOk: false,
      exeVerified: false,
      detected: [],
    },
    revived: null,
    runtime: { state: "unavailable", version: null, files: [], localDir: null },
    task: null,
    running: { game: false, relayPort: null },
    settings: {
      graphics: -1,
      windowed: false,
      language: "en",
      gameLanguage: "auto",
      runtimeProfile: "production",
      streamerMode: false,
      hostTools: false,
      gameEdition: "reborn",
    },
    app: {
      version: "0.1.0",
      packaged: false,
      updateReady: false,
      platform: "win32",
      hostToolsAvailable: true,
      serverGui: false,
      legacyAvailable: false,
      gameVersions: ["1.14.7"],
    },
    status: null,
    statusUnsupported: false,
    lastError: null,
    notice: null,
    ...over,
  }
}

const form = { inviteValid: true, usernameValid: true }

test("the primary button follows the phase", () => {
  expect(primaryButton(snap(), form)).toEqual({
    key: "btn_join",
    action: "join",
    disabled: false,
  })
  expect(primaryButton(snap(), { ...form, inviteValid: false }).disabled).toBe(
    true
  )
  const connect = (problem: Snapshot["connect"]["problem"]) =>
    snap({
      phase: "connect",
      connect: {
        checking: false,
        problem,
        tailscaleInstalled: true,
        lastCheckedAt: null,
      },
    })
  expect(primaryButton(connect("cert_mismatch"), form).action).toBe(
    "new_invite"
  )
  expect(primaryButton(connect("unreachable"), form).action).toBe("retry")
  expect(primaryButton(snap({ phase: "register" }), form).action).toBe(
    "register"
  )
  expect(primaryButton(snap({ phase: "locate" }), form).action).toBe("locate")
  expect(
    primaryButton(
      snap({
        phase: "locate",
        game: { ...snap().game, problem: "unverified" },
      }),
      form
    ).action
  ).toBe("verify")
  const runtime = (state: Snapshot["runtime"]["state"]) =>
    primaryButton(
      snap({
        phase: "runtime",
        runtime: { state, version: "1", files: [], localDir: null },
      }),
      form
    )
  expect(runtime("untrusted").action).toBe("trust")
  expect(runtime("missing")).toEqual({
    key: "btn_install",
    action: "install_runtime",
    disabled: false,
  })
  expect(runtime("outdated")).toEqual({
    key: "btn_update",
    action: "install_runtime",
    disabled: false,
  })
  expect(runtime("unavailable")).toEqual({
    key: "btn_waiting_host",
    action: "none",
    disabled: true,
  })
  expect(primaryButton(snap({ phase: "installing" }), form).action).toBe(
    "cancel"
  )
  expect(primaryButton(snap({ phase: "ready" }), form)).toEqual({
    key: "btn_play",
    action: "play",
    disabled: false,
  })
  expect(
    primaryButton(snap({ phase: "ready", busy: true }), form).disabled
  ).toBe(true)
  expect(primaryButton(snap({ phase: "running" }), form).key).toBe(
    "btn_running"
  )
})

test("registration closed on the server keeps REGISTER off", () => {
  const closed = snap({
    phase: "register",
    status: {
      name: "x",
      online: true,
      version: "",
      commit: "",
      sourceUrl: null,
      registration: "NONE",
      playersOnline: 0,
      players: [],
      instances: [],
      uptimeSeconds: 0,
      game: null,
      gameData: "real",
      chat: false,
      contentPort: null,
      limited: false,
    },
  })
  expect(primaryButton(closed, form).disabled).toBe(true)
})

test("1.4.4 in the legacy edition: DOWNLOAD, then CHECK", () => {
  const revived = (problem: "not_set" | "unverified") =>
    snap({
      phase: "locate",
      revived: {
        root: "D",
        chosen: false,
        problem,
        missingFiles: 0,
        totalBytes: 1,
        dllsOk: false,
        detected: [],
      },
    })
  expect(primaryButton(revived("not_set"), form).key).toBe("btn_download")
  expect(primaryButton(revived("unverified"), form).key).toBe("btn_check")
  expect(hintKey(revived("not_set"))).toBe("hint_revived_download")
})

test("steps: Join, Register, Game, Runtime, Play", () => {
  expect(STEP_KEYS.length).toBe(5)
  expect(stepIndex("join")).toBe(0)
  expect(stepIndex("connect")).toBe(0)
  expect(stepIndex("register")).toBe(1)
  expect(stepIndex("locate")).toBe(2)
  expect(stepIndex("runtime")).toBe(3)
  expect(stepIndex("installing")).toBe(3)
  expect(stepIndex("ready")).toBe(4)
})

test("every error and notice code has its text in both languages", () => {
  for (const key of [
    ...Object.values(ERROR_KEYS),
    ...Object.values(NOTICE_KEYS),
  ]) {
    expect(isKey(key), key).toBe(true)
    expect(CATALOGS["pt-BR"][key].length, key).toBeGreaterThan(0)
  }
})

test("an error's text: the invite's reason, the port, the host step, a detail kept apart", () => {
  expect(
    errorMessage({ code: "invite_invalid_format", detail: "revived" }, null)
  ).toEqual({ key: "invite_err_revived" })
  expect(
    errorMessage({ code: "invite_invalid_format", detail: "nonsense" }, null)
      .key
  ).toBe("err_invite_invalid_format")
  expect(
    errorMessage({ code: "relay_port_busy", detail: "61101" }, null).vars
  ).toEqual({ port: "61101" })
  expect(
    errorMessage({ code: "relay_port_busy", detail: "x" }, null).vars
  ).toEqual({ port: "61100" })
  expect(errorMessage({ code: "server_unreachable" }, "public").key).toBe(
    "err_server_unreachable_public"
  )
  expect(
    errorMessage({ code: "host_failed", detail: "workspace" }, null).key
  ).toBe("host_err_workspace_rust")
  expect(
    errorMessage({ code: "host_failed", detail: "workspace_mgr" }, null).key
  ).toBe("host_err_workspace_mgr")
  expect(
    errorMessage({ code: "host_failed", detail: "project_missing" }, null).key
  ).toBe("host_err_project_missing")
  expect(
    errorMessage({ code: "host_failed", detail: "pack_invalid" }, null).key
  ).toBe("host_err_pack_invalid_rust")
  expect(
    errorMessage({ code: "host_failed", detail: "node_missing" }, null)
  ).toEqual({
    key: "err_host_failed",
    vars: { detail: "node_missing" },
  })
  expect(
    errorMessage({ code: "host_failed", detail: "npm_missing" }, null)
  ).toEqual({
    key: "err_host_failed",
    vars: { detail: "npm_missing" },
  })
  expect(
    errorMessage({ code: "launch_failed", detail: "os error 5" }, null)
  ).toEqual({
    key: "err_launch_failed",
    detail: "os error 5",
  })
  expect(errorMessage({ code: "account_banned" }, null).vars).toEqual({
    reason: { key: "err_account_banned_noreason" },
  })
  for (const code of ["not_implemented", "not_in_this_build"] as ErrorCode[]) {
    expect(errorMessage({ code, detail: "epic_get" }, null)).toEqual({
      key: ERROR_KEYS[code],
    })
  }
})

test("the banner skips a cancel and the key_rejected the register screen shows itself", () => {
  expect(
    shownError(snap({ lastError: { code: "cancelled" } }), null)
  ).toBeNull()
  expect(
    shownError(
      snap({ phase: "register", lastError: { code: "key_rejected" } }),
      null
    )
  ).toBeNull()
  expect(
    shownError(snap({ lastError: { code: "busy" } }), { code: "unknown" })
  ).toEqual({ code: "busy" })
  expect(shownError(snap(), { code: "unknown" })).toEqual({ code: "unknown" })
})

test("progress: percent, sizes and speed in the player's language", () => {
  const v = taskView(
    {
      kind: "runtime",
      totalBytes: 2_000_000,
      doneBytes: 1_000_000,
      filesTotal: 2,
      filesDone: 1,
      bytesPerSecond: 500_000,
      etaSeconds: 2,
      currentFile: "winmm.dll",
    },
    "pt-BR"
  )
  expect(v.key).toBe("task_runtime")
  expect(v.percent).toBe(50)
  expect(v.bytes).toBe("1,00 MB / 2,00 MB")
  expect(v.eta).toBe("2 s")
  expect(formatBytes(1_500, "en")).toBe("1.50 KB")
  expect(formatPercentText(50, "en")).toBe("50%")
  const verify = taskView(
    {
      kind: "verify",
      totalBytes: 10,
      doneBytes: 5,
      filesTotal: 1,
      filesDone: 0,
      bytesPerSecond: 0,
      etaSeconds: null,
      currentFile: null,
    },
    "en"
  )
  expect(verify.speed).toBeNull()
})

test("an invite's preview: name, address, mode and fingerprint, never more", () => {
  const fp = "ab".repeat(32)
  expect(
    inviteSummary(
      `reforged-revival://join?v=2&mode=public&host=play.example.org&port=443&fp=${fp}&code=x&name=Hunts`
    )
  ).toEqual({
    name: "Hunts",
    host: "play.example.org",
    port: 443,
    mode: "public",
    fp,
  })
  expect(
    inviteSummary(
      "reforged-revival://join?v=1&host=100.64.0.9&port=61000&code=c&name=Friends%20Club"
    )
  ).toEqual({
    name: "Friends Club",
    host: "100.64.0.9",
    port: 61000,
    mode: "private",
    fp: null,
  })
  expect(inviteSummary("not an invite")).toBeNull()
  expect(shortHex(fp)).toBe("abab abab abab abab…")
})

test("usernames: Reforged's rule", () => {
  expect(checkUsername("Slayer_01")).toBe("ok")
  expect(checkUsername("")).toBe("empty")
  expect(checkUsername("ab")).toBe("too_short")
  expect(checkUsername("a".repeat(17))).toBe("too_long")
  expect(checkUsername("bad name")).toBe("bad_chars")
  expect(checkUsername(123)).toBe("empty")
})
