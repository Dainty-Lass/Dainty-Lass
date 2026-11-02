/// <reference types="bun" />
// The Game screen's decisions (DE7): what each version offers for each state of the account, the library and the
// install, and that every error the Epic commands answer has its text in both languages
import { expect, test } from "bun:test"

import type {
  EpicLibraryCheck,
  ErrorCode,
  GameVersionEntry,
  InstallInfo,
  InstallState,
  TaskProgress,
} from "@workspace/bridge"
import { CATALOGS, isKey, LANGUAGES } from "@workspace/i18n"

import {
  ACTION_KEYS,
  installProgress,
  isRunning,
  owned,
  sameFolder,
  versionRow,
  versionSizes,
  type RowContext,
} from "../src/app/epic"
import { formatDateTime } from "../src/app/format"
import { ERROR_KEYS, errorMessage, taskView } from "../src/app/model"

function epicEntry(
  over: Partial<GameVersionEntry["epic"] & object> = {}
): GameVersionEntry {
  return {
    version: "1.14.7",
    displayName: "Rustic Isles 1.14.7",
    origin: "launcher",
    install: "epic",
    epic: {
      buildVersion: "1.14.7.647472",
      manifest: "ready",
      downloadBytes: 15_000_000_000,
      installBytes: 32_000_000_000,
      ...over,
    },
  }
}

const locate: GameVersionEntry = {
  version: "1.4.4",
  displayName: "Dauntless Revived",
  origin: "launcher",
  install: "locate",
  epic: null,
}

function info(
  state: InstallState,
  over: Partial<InstallInfo> = {}
): InstallInfo {
  return {
    version: "1.14.7",
    state,
    folder: null,
    progress: null,
    error: null,
    ...over,
  }
}

const signedIn = { signedIn: true, displayName: "Slayer", signingIn: false }
const library = (ownedByAccount: boolean): EpicLibraryCheck => ({
  ok: true,
  owned: ownedByAccount,
  checkedAt: "2026-10-27T12:00:00Z",
})
const ctx: RowContext = {
  account: signedIn,
  library: library(true),
  otherRunning: false,
  inUse: false,
}

const actions = (row: ReturnType<typeof versionRow>) =>
  row.buttons.map((b) => `${b.action}${b.disabled ? "!" : ""}`)

test("a version with no Epic build is located, never installed", () => {
  const row = versionRow(locate, null, ctx)
  expect(row.status.key).toBe("gm_ver_locate")
  expect(row.buttons).toEqual([])
})

test("a signed-out account is told to sign in, and a library without Dauntless refuses the install", () => {
  const out = versionRow(epicEntry(), info("not_installed"), {
    ...ctx,
    account: { signedIn: false, displayName: null, signingIn: false },
  })
  expect(actions(out)).toEqual(["install!"])
  expect(out.hint).toBe("gm_sign_in_first")
  const without = versionRow(epicEntry(), info("not_installed"), {
    ...ctx,
    library: library(false),
  })
  expect(actions(without)).toEqual(["install!"])
  expect(without.hint).toBe("gm_not_owned_short")
  // Before the library is checked, the backend's own check decides
  const unknown = versionRow(epicEntry(), info("not_installed"), {
    ...ctx,
    library: null,
  })
  expect(actions(unknown)).toEqual(["install"])
  expect(unknown.hint).toBeNull()
})

test("one install runs at a time", () => {
  const row = versionRow(epicEntry(), info("not_installed"), {
    ...ctx,
    otherRunning: true,
  })
  expect(actions(row)).toEqual(["install!"])
  expect(row.hint).toBe("gm_other_running")
})

test("each install state offers what the backend can do with it", () => {
  const offers = (state: InstallState) =>
    actions(versionRow(epicEntry(), info(state, { folder: "/g/1.14.7" }), ctx))
  expect(offers("not_installed")).toEqual(["install"])
  expect(offers("resumable")).toEqual(["continue"])
  expect(offers("failed")).toEqual(["retry"])
  expect(offers("preparing")).toEqual(["pause", "cancel"])
  expect(offers("downloading")).toEqual(["pause", "cancel"])
  expect(offers("paused")).toEqual(["resume", "cancel"])
  expect(offers("finishing")).toEqual([])
  expect(offers("installed")).toEqual(["use"])
  const states: InstallState[] = [
    "not_installed",
    "resumable",
    "preparing",
    "downloading",
    "paused",
    "finishing",
    "installed",
    "failed",
  ]
  for (const state of states) {
    const row = versionRow(epicEntry(), info(state), ctx)
    expect(isKey(row.status.key), state).toBe(true)
    expect(row.running).toBe(isRunning(state))
  }
})

test("an installed version in use offers nothing, and shows its folder", () => {
  const done = info("installed", {
    folder: "C:\\Users\\Ana\\Games\\Dauntless\\1.14.7",
  })
  const row = versionRow(epicEntry(), done, { ...ctx, inUse: true })
  expect(row.buttons).toEqual([])
  expect(row.status).toEqual({ key: "gm_ver_installed", detail: done.folder! })
  expect(sameFolder("c:/users/ana/games/dauntless/1.14.7/", done.folder)).toBe(
    true
  )
  expect(sameFolder("/a/b", "/A/b")).toBe(false)
  expect(sameFolder(null, "/a")).toBe(false)
})

test("a failed install says why, with a message the page has", () => {
  const row = versionRow(
    epicEntry(),
    info("failed", { error: { code: "disk_space", detail: "15000000000" } }),
    ctx
  )
  expect(actions(row)).toEqual(["retry"])
  expect(row.error?.code).toBe("disk_space")
})

test("a manifest that can't be read hides the install, but not a stopped one's continue", () => {
  const unreachable = versionRow(
    epicEntry({
      manifest: "unreachable",
      downloadBytes: null,
      installBytes: null,
    }),
    info("not_installed"),
    ctx
  )
  expect(unreachable.status.key).toBe("gm_ver_manifest_unreachable")
  expect(unreachable.buttons).toEqual([])
  const invalid = versionRow(
    epicEntry({ manifest: "invalid" }),
    info("not_installed"),
    ctx
  )
  expect(invalid.status.key).toBe("gm_ver_manifest_invalid")
  const resumable = versionRow(
    epicEntry({ manifest: "unreachable" }),
    info("resumable"),
    ctx
  )
  expect(actions(resumable)).toEqual(["continue"])
})

test("the channel's progress counts only for the download that is polled", () => {
  const progress = (total: number, done: number): TaskProgress => ({
    kind: "install",
    totalBytes: total,
    doneBytes: done,
    filesTotal: 611,
    filesDone: 1,
    bytesPerSecond: 1,
    etaSeconds: null,
    currentFile: null,
  })
  const polled = info("downloading", { progress: progress(100, 40) })
  expect(installProgress(polled, progress(100, 55))?.doneBytes).toBe(55)
  // An earlier install's last numbers (another total) are not this one's
  expect(installProgress(polled, progress(7, 7))?.doneBytes).toBe(40)
  expect(installProgress(info("preparing"), progress(7, 7))?.doneBytes).toBe(7)
  expect(installProgress(info("preparing"), null)).toBeNull()
  expect(installProgress(info("installed"), progress(100, 100))).toBeNull()
  expect(installProgress(null, progress(100, 100))).toBeNull()
})

test("an install task is drawn as a moving download", () => {
  const view = taskView(
    {
      kind: "install",
      totalBytes: 15_000_000_000,
      doneBytes: 3_000_000_000,
      filesTotal: 611,
      filesDone: 100,
      bytesPerSecond: 50_000_000,
      etaSeconds: 240,
      currentFile: null,
    },
    "en"
  )
  expect(view.key).toBe("task_install")
  expect(view.percent).toBe(20)
  expect(view.bytes).toBe("3.00 GB / 15.0 GB")
  expect(view.speed).toBe("50.0 MB/s")
  expect(view.eta).toBe("4 min 0 s")
})

test("the sizes come from the manifest once it is ready", () => {
  expect(versionSizes(epicEntry())).toEqual({
    download: 15_000_000_000,
    install: 32_000_000_000,
  })
  expect(
    versionSizes(epicEntry({ downloadBytes: null, installBytes: null }))
  ).toBeNull()
  expect(versionSizes(locate)).toBeNull()
})

test("the library check reads as owned, not owned or unknown", () => {
  expect(owned(null)).toBeNull()
  expect(owned(library(true))).toBe(true)
  expect(owned(library(false))).toBe(false)
  expect(
    owned({ ok: false, error: { code: "epic_library_failed" } })
  ).toBeNull()
})

// DE2 to DE5's codes: each has a text in the model and in both catalogs
const EPIC_CODES: ErrorCode[] = [
  "epic_signed_out",
  "epic_library_failed",
  "epic_not_owned",
  "epic_code_invalid",
  "epic_rejected",
  "epic_unreachable",
  "epic_failed",
  "epic_manifest_unavailable",
  "game_version_unknown",
  "game_version_unavailable",
  // What an install stops with besides
  "disk_space",
  "download_failed",
  "folder_invalid",
  "game_wrong_version",
  "busy",
  "cancelled",
]

test("every error an Epic command answers has a message in both languages", () => {
  for (const code of EPIC_CODES) {
    const key = ERROR_KEYS[code]
    expect(key, code).toBeDefined()
    for (const lang of LANGUAGES) {
      expect(
        CATALOGS[lang][key].trim().length,
        `${lang}.${code}`
      ).toBeGreaterThan(0)
    }
    expect(errorMessage({ code }, null).key).toBe(key)
  }
})

test("every key the Game screen uses exists in both languages", () => {
  const keys = [
    ...Object.values(ACTION_KEYS),
    "nav_game",
    "locate_epic_title",
    "locate_epic_text",
    "locate_epic_button",
    "task_install",
  ]
  for (const key of keys) {
    expect(isKey(key), key).toBe(true)
  }
  for (const key of Object.keys(CATALOGS.en)) {
    if (!key.startsWith("gm_")) continue
    expect(CATALOGS["pt-BR"][key as keyof typeof CATALOGS.en]).toBeTruthy()
  }
})

test("a library check time is written in the player's locale", () => {
  const en = formatDateTime("2026-10-27T12:00:00Z", "en")
  const pt = formatDateTime("2026-10-27T12:00:00Z", "pt-BR")
  expect(en).toContain("2026")
  expect(pt).toContain("2026")
  expect(en).not.toBe(pt)
  expect(formatDateTime("not a time", "en")).toBe("")
})
