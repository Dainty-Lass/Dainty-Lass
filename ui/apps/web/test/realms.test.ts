/// <reference types="bun" />
import { describe, expect, test } from "bun:test"
import { createElement } from "react"
import { renderToStaticMarkup } from "react-dom/server"

import type {
  ModuleView,
  RealmOutcome,
  RealmSessionView,
  RealmView,
  Snapshot,
} from "@workspace/bridge"
import {
  CATALOGS,
  isKey,
  translate,
  type Key,
  type Language,
} from "@workspace/i18n"

import { I18nProvider } from "../src/app/i18n"
import { LauncherProvider } from "../src/app/launcher"
import { errorMessage } from "../src/app/model"
import {
  checkRealmId,
  HUB_TARGET_KEYS,
  REALM_ID_MAX,
  REALM_KIND_KEYS,
  REALM_STATE_KEYS,
  realmsInBuild,
  realmsPageAvailable,
  realmsSupported,
  realmStateTone,
  relayStatusMessage,
  sessionResultMessage,
  sessionRoleKey,
} from "../src/app/realms"
import type { RealmsState } from "../src/app/use-realms"
import { RealmsContent } from "../src/screens/realms"

function baseSnap(over: Partial<Snapshot> = {}): Snapshot {
  return {
    phase: "ready",
    busy: false,
    task: null,
    app: {
      version: "0.2.0",
      packaged: false,
      updateReady: false,
      platform: "win32",
      hostToolsAvailable: true,
      serverGui: false,
      legacyAvailable: false,
      gameVersions: ["1.14.7"],
    },
    server: {
      host: "hunt.example.com",
      port: 8443,
      name: "Shattered Isles",
      mode: "public",
      game: "reforged",
      fingerprint: null,
      hasShare: false,
      hasPendingInvite: false,
      loopback: false,
    },
    savedServers: [],
    status: {
      name: "Shattered Isles",
      online: true,
      version: "0.2.0",
      commit: "abc1234",
      sourceUrl: null,
      registration: "OPEN",
      playersOnline: 3,
      players: [],
      instances: [
        {
          id: "01JREALMHUB000000000000001",
          kind: "city",
          title: "Ramsgate",
          map: "Ramsgate",
          behemoth: null,
          players: 2,
          maxPlayers: 6,
          startedAt: "2026-04-20T00:00:00Z",
        },
        {
          id: "01JREALMHUNT00000000000002",
          kind: "hunt",
          title: "Shrike",
          map: "Conundrum Rocks",
          behemoth: "Shrike",
          players: 1,
          maxPlayers: 4,
          startedAt: "2026-04-20T00:05:00Z",
        },
        {
          id: "01JREALMFULL00000000000003",
          kind: "dojo",
          title: "",
          map: "Training Grounds",
          behemoth: null,
          players: 4,
          maxPlayers: 4,
          startedAt: "2026-04-20T00:10:00Z",
        },
      ],
      uptimeSeconds: 120,
      game: { build: "1.14.7", changelist: 641645 },
      gameData: "real",
      chat: true,
      contentPort: null,
      limited: false,
    },
    statusUnsupported: false,
    connect: {
      checking: false,
      problem: null,
      tailscaleInstalled: true,
      lastCheckedAt: null,
    },
    account: {
      username: "Slayer",
      hasKey: true,
      offerBackup: false,
    },
    game: {
      root: "C:\\Games\\Dauntless",
      problem: null,
      versionText: "1.14.7",
      versionOk: true,
      exeVerified: true,
      detected: [],
    },
    revived: null,
    runtime: {
      state: "ok",
      version: "0.2.0",
      files: [],
      localDir: null,
    },
    settings: {
      graphics: 2,
      windowed: false,
      language: "en",
      gameLanguage: "auto",
      runtimeProfile: "production",
      streamerMode: false,
      hostTools: false,
      gameEdition: "reborn",
    },
    running: { game: true, relayPort: 9443 },
    lastError: null,
    notice: null,
    ...over,
  }
}

function sampleSession(
  overrides: Partial<RealmSessionView> = {}
): RealmSessionView {
  return {
    session: "01JREALMHUB000000000000001@local",
    kind: "hub",
    hosting: true,
    state: "running",
    generation: 2,
    localPort: 30777,
    result: null,
    ...overrides,
  }
}

function sampleRealm(overrides: Partial<RealmView> = {}): RealmView {
  const active = sampleSession()
  return {
    available: true,
    active,
    sessions: [active],
    ...overrides,
  }
}

function sampleModule(overrides: Partial<ModuleView> = {}): ModuleView {
  return {
    id: "dauntless-1.14.7",
    displayName: "Dauntless 1.14.7",
    role: "game",
    gameVersion: "1.14.7",
    origin: "compiled_in",
    autoload: true,
    state: "ready",
    detail: null,
    unverified: false,
    ports: [],
    capabilities: {
      realms: true,
      network: true,
      chat: true,
      patchSet: { provides: false, displayName: null },
      theme: null,
    },
    commands: [],
    settings: [],
    ...overrides,
  }
}

function sampleRealmsState(
  realm: RealmView | null = sampleRealm(),
  outcomeOverride?: RealmOutcome | null
): RealmsState {
  const outcome: RealmOutcome | null =
    outcomeOverride !== undefined
      ? outcomeOverride
      : realm
        ? { ok: true, realm }
        : null
  return {
    outcome,
    realm,
    working: null,
    refresh: async () => undefined,
    host: async () => true,
    join: async () => true,
    stop: async () => true,
  }
}

describe("realm availability & edition gating (DD11, DD14, RF7, DU5)", () => {
  test("hidden when realms are not in the build", () => {
    const s = baseSnap()
    expect(realmsInBuild(null)).toBe(false)
    expect(
      realmsInBuild({
        ok: false,
        error: { code: "not_in_this_build", detail: "realm_get" },
      })
    ).toBe(false)
    expect(
      realmsInBuild({
        ok: false,
        error: { code: "not_implemented" },
      })
    ).toBe(false)
    expect(
      realmsPageAvailable(s, {
        ok: false,
        error: { code: "not_in_this_build", detail: "realm_get" },
      })
    ).toBe(false)
  })

  test("available when realms are in the build on 1.14.7", () => {
    const s = baseSnap()
    const outcome: RealmOutcome = { ok: true, realm: sampleRealm() }
    expect(realmsInBuild(outcome)).toBe(true)
    expect(realmsSupported(s)).toBe(true)
    expect(realmsPageAvailable(s, outcome)).toBe(true)
    // A transient realm_unavailable error still means the build has the realms feature
    expect(
      realmsInBuild({
        ok: false,
        error: { code: "realm_unavailable", detail: "transport_missing" },
      })
    ).toBe(true)
  })

  test("hidden on 1.4.4 legacy edition and 1.12.0", () => {
    const outcome: RealmOutcome = { ok: true, realm: sampleRealm() }

    const legacy = baseSnap()
    legacy.settings.gameEdition = "legacy"
    expect(realmsSupported(legacy)).toBe(false)
    expect(realmsPageAvailable(legacy, outcome)).toBe(false)

    const revivedServer = baseSnap()
    revivedServer.server!.game = "revived"
    expect(realmsSupported(revivedServer)).toBe(false)

    const build144 = baseSnap()
    build144.status!.game = { build: "1.4.4", changelist: null }
    expect(realmsSupported(build144)).toBe(false)

    const build112 = baseSnap()
    build112.status!.game = { build: "1.12.0", changelist: null }
    expect(realmsSupported(build112)).toBe(false)
    expect(realmsPageAvailable(build112, outcome)).toBe(false)
  })

  test("respects loaded game module realms capability when modules are provided", () => {
    const s = baseSnap()
    const outcome: RealmOutcome = { ok: true, realm: sampleRealm() }
    const withRealms = [sampleModule()]
    const withoutRealms = [
      sampleModule({
        id: "dauntless-1.12.0",
        gameVersion: "1.12.0",
        capabilities: {
          ...sampleModule().capabilities,
          realms: false,
        },
      }),
    ]
    expect(realmsPageAvailable(s, outcome, withRealms)).toBe(true)
    expect(realmsPageAvailable(s, outcome, withoutRealms)).toBe(false)
  })
})

describe("session state, roles, relay status, and realm ID checks", () => {
  test("maps session roles and state tones", () => {
    expect(sessionRoleKey({ kind: "hub", hosting: true })).toBe(
      "rlm_role_host_hub"
    )
    expect(sessionRoleKey({ kind: "hunt", hosting: true })).toBe(
      "rlm_role_host_hunt"
    )
    expect(sessionRoleKey({ kind: "hub", hosting: false })).toBe(
      "rlm_role_join_hub"
    )
    expect(sessionRoleKey({ kind: "hunt", hosting: false })).toBe(
      "rlm_role_join_hunt"
    )

    expect(realmStateTone("starting", null)).toBe("busy")
    expect(realmStateTone("ready", null)).toBe("good")
    expect(realmStateTone("running", null)).toBe("good")
    expect(realmStateTone("ended", "clean")).toBe("idle")
    expect(realmStateTone("ended", "setup")).toBe("bad")
  })

  test("formats relay status for starting, local UDP port, gateway relay, and ended results", () => {
    const starting = sampleSession({ state: "starting", localPort: null })
    expect(relayStatusMessage(starting, null)).toEqual({
      key: "rlm_relay_starting",
    })

    const localOnly = sampleSession({ state: "ready", localPort: 30777 })
    expect(relayStatusMessage(localOnly, null)).toEqual({
      key: "rlm_relay_ready",
      vars: { port: 30777 },
    })

    expect(relayStatusMessage(localOnly, 9443)).toEqual({
      key: "rlm_relay_both",
      vars: { port: 30777, gateway: 9443 },
    })

    for (const code of [
      "clean",
      "bad_input",
      "setup",
      "refused_admission",
      "lease_failed",
      "unknown_handle",
    ]) {
      const ended = sampleSession({ state: "ended", result: code })
      expect(sessionResultMessage(code)).toEqual({
        key: `rlm_result_${code}` as never,
      })
      expect(relayStatusMessage(ended, 9443)).toEqual({
        key: `rlm_result_${code}` as never,
      })
    }

    expect(sessionResultMessage("unknown_exit")).toEqual({
      key: "rlm_result_ended",
    })
  })

  test("validates realm session IDs", () => {
    expect(checkRealmId("")).toBe("empty")
    expect(checkRealmId("   ")).toBe("empty")
    expect(checkRealmId(null)).toBe("empty")
    expect(checkRealmId("01JREALMHUB000000000000001")).toBe("ok")
    expect(checkRealmId("01JREALMHUB000000000000001@local")).toBe("ok")
    expect(checkRealmId("has space")).toBe("bad_chars")
    expect(checkRealmId("has\tcontrol")).toBe("bad_chars")
    expect(checkRealmId("a".repeat(REALM_ID_MAX))).toBe("ok")
    expect(checkRealmId("a".repeat(REALM_ID_MAX + 1))).toBe("too_long")
  })

  test("maps realm_unavailable details and realm errors to catalog keys", () => {
    for (const detail of [
      "transport_missing",
      "admission_key_missing",
      "transport_untrusted",
      "transport_hash_mismatch",
      "coop_disabled",
    ]) {
      expect(
        errorMessage({ code: "realm_unavailable", detail }, "public").key
      ).toBe(`realm_err_${detail}` as never)
    }
    expect(errorMessage({ code: "realm_unavailable" }, "public").key).toBe(
      "err_realm_unavailable"
    )
    expect(
      errorMessage({ code: "realm_not_found", detail: "01J..." }, "public").key
    ).toBe("err_realm_not_found")
    expect(
      errorMessage({ code: "realm_failed", detail: "spawn" }, "public").key
    ).toBe("err_realm_failed")
  })
})

describe("RealmsContent SSR in en and pt-BR", () => {
  const langs: Language[] = ["en", "pt-BR"]
  const htmlEscape = (text: string) =>
    text
      .replaceAll("&", "&amp;")
      .replaceAll('"', "&quot;")
      .replaceAll("'", "&#x27;")

  function renderRealms(
    lang: Language,
    snap: Snapshot | null,
    state: RealmsState
  ): string {
    return renderToStaticMarkup(
      createElement(LauncherProvider, {
        children: createElement(I18nProvider, {
          language: lang,
          children: createElement(RealmsContent, { snap, state }),
        }),
      })
    )
  }

  test("renders nothing when realms are not in the build or on unsupported editions", () => {
    const snap = baseSnap()
    const notInBuild = sampleRealmsState(null, {
      ok: false,
      error: { code: "not_in_this_build", detail: "realm_get" },
    })
    expect(renderRealms("en", snap, notInBuild)).toBe("")
    expect(renderRealms("pt-BR", snap, notInBuild)).toBe("")

    const legacySnap = baseSnap()
    legacySnap.settings.gameEdition = "legacy"
    expect(renderRealms("en", legacySnap, sampleRealmsState())).toBe("")
  })

  test("never lists dedicated server instances as joinable realms", () => {
    const snap = baseSnap()
    const html = renderRealms("en", snap, sampleRealmsState())
    expect(html).not.toContain('data-action="join-browser"')
    for (const inst of snap.status!.instances) {
      expect(html).not.toContain(`data-target="${inst.id}"`)
    }
  })

  test("disables host and join-by-ID controls when realm transport is unavailable", () => {
    const snap = baseSnap()
    const unavailable = sampleRealm({ available: false, active: null, sessions: [] })
    const html = renderRealms("en", snap, sampleRealmsState(unavailable))
    expect(html).toContain('data-transport="unavailable"')
    expect(html).toContain(htmlEscape(translate("en", "rlm_unavailable_hint")))
  })

  for (const lang of langs) {
    test(`renders active session, relay status, host controls, and join-by-ID form without server-instance browser in ${lang}`, () => {
      const snap = baseSnap()
      const secondSession = sampleSession({
        session: "01JREALMHUNT00000000000009",
        kind: "hunt",
        hosting: false,
        state: "ended",
        generation: 1,
        localPort: null,
        result: "clean",
      })
      const realm = sampleRealm({
        available: true,
        active: sampleSession(),
        sessions: [sampleSession(), secondSession],
      })
      const html = renderRealms(lang, snap, sampleRealmsState(realm))

      expect(html).toContain('data-screen="realms"')
      expect(html).toContain('data-transport="ready"')
      expect(html).toContain('data-action="host-hub"')
      expect(html).toContain('data-action="host-hunt"')
      expect(html).toContain('data-action="join-id"')
      expect(html).not.toContain('data-action="join-browser"')
      expect(html).not.toContain("01JREALMHUNT00000000000002")
      expect(html).toContain('data-action="stop-realm"')
      expect(html).toContain(htmlEscape(translate(lang, "rlm_title")))
      expect(html).toContain(htmlEscape(translate(lang, "rlm_status_title")))
      expect(html).toContain(htmlEscape(translate(lang, "rlm_host_title")))
      expect(html).toContain(htmlEscape(translate(lang, "rlm_join_title")))
      expect(html).not.toContain(
        htmlEscape(translate(lang, "rlm_browser_title"))
      )
      expect(html).toContain(
        htmlEscape(
          translate(lang, "rlm_relay_both", { port: 30777, gateway: 9443 })
        )
      )
      expect(html).toContain(htmlEscape(translate(lang, "rlm_result_clean")))
    })
  }

  test("every realm catalog key is registered in both catalogs", () => {
    const keys: Key[] = [
      "nav_realms",
      ...Object.values(HUB_TARGET_KEYS),
      ...Object.values(REALM_KIND_KEYS),
      ...Object.values(REALM_STATE_KEYS),
    ]
    for (const k of keys) {
      expect(isKey(k)).toBe(true)
      expect(CATALOGS.en[k].length).toBeGreaterThan(0)
      expect(CATALOGS["pt-BR"][k].length).toBeGreaterThan(0)
    }
  })
})
