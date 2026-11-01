/// <reference types="bun" />
// Page tests for the DaintyLass Manager (`server-gui`) screens: ServerTab, ServerConfigSection, MongoDB
// migration card (unmigrated, verified/migrated, and fresh-start/skipped states + confirmation + read-only
// guarantee), DoctorPanel, Backup, AdminTab (and api.adminAction calls), HistoryTab, and localized stub/refusal
// error messages in both English (en) and Brazilian Portuguese (pt-BR) (DU3, T4).
import { describe, expect, test } from "bun:test"
import { createElement, type ReactNode } from "react"
import { renderToStaticMarkup } from "react-dom/server"

import {
  api,
  type AdminActionKind,
  type AdminRequest,
  type AdminResultView,
  type DoctorReportView,
  type GuardedActionInfo,
  type HistoryEntryView,
  type HostKit,
  type HostServerView,
  type LauncherError,
  type RevertDoneView,
  type RevertPlanView,
  type ServerConfigView,
  type Snapshot,
} from "@workspace/bridge"
import { translate, type Language } from "@workspace/i18n"

import { I18nProvider } from "../src/app/i18n"
import { LauncherProvider } from "../src/app/launcher"
import { errorMessage, type Message } from "../src/app/model"
import {
  AdminTab,
  DoctorPanel,
  HistoryTab,
  ManagedInstallsPanel,
  ServerConfigSection,
  ServerTab,
  StatusTab,
  submitAdminAction,
} from "../src/screens/host"

const langs: Language[] = ["en", "pt-BR"]

const htmlEscape = (text: string) =>
  text
    .replaceAll("&", "&amp;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#x27;")

function renderInLanguage(lang: Language, node: ReactNode): string {
  return renderToStaticMarkup(
    createElement(LauncherProvider, {
      children: createElement(I18nProvider, {
        language: lang,
        children: node,
      }),
    })
  )
}

function sampleManagerSnap(over: Partial<Snapshot> = {}): Snapshot {
  return {
    phase: "join",
    busy: false,
    task: null,
    app: {
      version: "2.0.0-alpha.0",
      packaged: true,
      updateReady: false,
      platform: "win32",
      hostToolsAvailable: true,
      serverGui: true,
      legacyAvailable: true,
      gameVersions: ["1.14.7", "1.12.0", "1.4.4"],
    },
    server: null,
    savedServers: [],
    status: null,
    statusUnsupported: false,
    connect: {
      checking: false,
      problem: null,
      tailscaleInstalled: true,
      lastCheckedAt: null,
    },
    account: {
      username: null,
      hasKey: false,
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
      version: null,
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
      hostTools: true,
      gameEdition: "reborn",
    },
    running: { game: false, relayPort: null },
    lastError: null,
    notice: null,
    ...over,
  }
}

function sampleKit(over: Partial<HostKit> = {}): HostKit {
  return {
    workspace: "C:\\Users\\Host\\AppData\\Roaming\\DaintyLass\\Server",
    workspaceOk: true,
    upstream: { present: false, patched: false },
    sdk: { present: false, functionFiles: 0 },
    dumps: [],
    dumperDll: null,
    builds: { runtime: false, loader: false, exporter: false, published: false },
    toolchain: { visualStudio: null, msbuild: false, cargo: false },
    gameData: { backend: "synthetic", director: "synthetic" },
    exports: { present: false, inGame: false },
    serverConfig: {
      configured: true,
      name: "Friday Hunts",
      mode: "private",
      host: "100.64.0.1",
      multipliers: null,
    },
    requirements: [],
    job: null,
    update: null,
    projectTarget: null,
    ...over,
  }
}

function sampleServer(over: Partial<HostServerView> = {}): HostServerView {
  return {
    state: "stopped",
    message: null,
    lines: [],
    invite: null,
    embedded: true,
    components: [
      { name: "gateway", phase: "ready", port: 61000, detail: "listening" },
      { name: "content", phase: "ready", port: 61002, detail: "listening" },
      { name: "admin", phase: "ready", port: 61003, detail: "listening" },
      {
        name: "isles-1-14-7",
        phase: "ready",
        port: 61001,
        detail: "1.14.7 backend & director",
      },
    ],
    ...over,
  }
}

function sampleConfig(over: Partial<ServerConfigView> = {}): ServerConfigView {
  return {
    configured: true,
    name: "Friday Hunts",
    mode: "private",
    publicHost: null,
    registration: "INVITECODE",
    database: "sqlite",
    maxHubs: 1,
    maxHunts: 3,
    freshStart: false,
    mongoMigration: {
      state: "none",
      source: null,
      detail: null,
    },
    configFile:
      "C:\\Users\\Host\\AppData\\Roaming\\DaintyLass\\Server\\.workspace\\config\\host.toml",
    ...over,
  }
}

describe("ServerTab & DaintyLass Manager setup flow in en and pt-BR (T2, T3, T4)", () => {
  for (const lang of langs) {
    test(`renders DaintyLass Manager data folder, setup panel, game folder picker, and backup button without Download-it in ${lang}`, () => {
      const snap = sampleManagerSnap({
        game: {
          root: null,
          problem: "not_set",
          versionText: null,
          versionOk: false,
          exeVerified: false,
          detected: [],
        },
      })
      const kit = sampleKit({
        workspaceOk: false,
        serverConfig: {
          configured: false,
          name: null,
          mode: null,
          host: null,
          multipliers: null,
        },
      })
      const html = renderInLanguage(
        lang,
        createElement(ServerTab, {
          kit,
          server: sampleServer({ state: "stopped" }),
          snap,
          setTab: () => undefined,
        })
      )

      expect(html).toContain(htmlEscape(translate(lang, "host_ws_title_mgr")))
      expect(html).toContain(htmlEscape(translate(lang, "host_ws_text_mgr")))
      expect(html).toContain(htmlEscape(translate(lang, "host_ws_choose_mgr")))
      expect(html).toContain(
        htmlEscape(translate(lang, "hsrv_text_stopped_mgr"))
      )
      expect(html).toContain(htmlEscape(translate(lang, "hsrv_setup_text_mgr")))
      expect(html).toContain(
        htmlEscape(translate(lang, "hsrv_setup_no_game_mgr"))
      )
      expect(html).toContain(
        htmlEscape(translate(lang, "hsrv_setup_choose_game"))
      )
      // DaintyLass Manager never offers the repository Download-it button or UpdatePanel
      expect(html).not.toContain(htmlEscape(translate(lang, "host_ws_download")))
      expect(html).not.toContain(
        htmlEscape(translate(lang, "hsrv_update_title"))
      )
    })

    test(`renders running DaintyLass Manager card and database backup button in ${lang}`, () => {
      const snap = sampleManagerSnap()
      const kit = sampleKit()
      const html = renderInLanguage(
        lang,
        createElement(ServerTab, {
          kit,
          server: sampleServer({ state: "running" }),
          snap,
          setTab: () => undefined,
        })
      )

      expect(html).toContain(
        htmlEscape(translate(lang, "hsrv_text_running_mgr"))
      )
      expect(html).toContain(htmlEscape(translate(lang, "hsrv_backup")))
      expect(html).toContain(
        htmlEscape(translate(lang, "hsrv_check_workspace_mgr"))
      )
    })
  }
})

describe("MongoDB migration card states, confirmation, and non-modification guarantee in en and pt-BR (M1, M4, T4)", () => {
  for (const lang of langs) {
    test(`renders unmigrated state with migrate action, fresh-start button, and read-only MongoDB note in ${lang}`, () => {
      const cfg = sampleConfig({
        mongoMigration: {
          state: "unmigrated",
          source: "managed MongoDB data folder (.workspace/data/mongo)",
          detail: null,
        },
      })
      const html = renderInLanguage(
        lang,
        createElement(ServerConfigSection, {
          kit: sampleKit(),
          server: sampleServer({ state: "stopped" }),
          initialConfig: cfg,
          initialConfirmFresh: false,
        })
      )

      expect(html).toContain('data-mongo-migration="unmigrated"')
      expect(html).toContain('data-mongo-readonly="true"')
      expect(html).toContain(htmlEscape(translate(lang, "hsrv_mongo_title")))
      expect(html).toContain(
        htmlEscape(
          translate(lang, "hsrv_mongo_unmigrated", {
            source: "managed MongoDB data folder (.workspace/data/mongo)",
          })
        )
      )
      expect(html).toContain(
        htmlEscape(translate(lang, "hsrv_mongo_readonly_note"))
      )
      expect(html).toContain('data-action="mongo-migrate"')
      expect(html).toContain(htmlEscape(translate(lang, "hsrv_mongo_migrate")))
      // Fresh start requires confirmation: before clicking, only the initial button is shown
      expect(html).toContain('data-action="mongo-fresh-start"')
      expect(html).not.toContain('data-action="mongo-fresh-confirm"')
    })

    test(`fresh-start asks for explicit confirmation before ignoring MongoDB data in ${lang}`, () => {
      const cfg = sampleConfig({
        mongoMigration: {
          state: "unmigrated",
          source: "mongodb://127.0.0.1:27017/dauntless",
          detail: null,
        },
      })
      const html = renderInLanguage(
        lang,
        createElement(ServerConfigSection, {
          kit: sampleKit(),
          server: sampleServer({ state: "stopped" }),
          initialConfig: cfg,
          initialConfirmFresh: true,
        })
      )

      expect(html).toContain('data-action="mongo-fresh-confirm"')
      expect(html).toContain('data-action="mongo-fresh-cancel"')
      expect(html).toContain(
        htmlEscape(translate(lang, "hsrv_mongo_fresh_confirm"))
      )
      expect(html).toContain(
        htmlEscape(translate(lang, "hsrv_mongo_fresh_cancel"))
      )
      expect(html).toContain(
        htmlEscape(translate(lang, "hsrv_mongo_readonly_note"))
      )
    })

    test(`renders verified (migrated) state and states that MongoDB was never modified in ${lang}`, () => {
      const cfg = sampleConfig({
        mongoMigration: {
          state: "migrated",
          source: null,
          detail: null,
        },
      })
      const html = renderInLanguage(
        lang,
        createElement(ServerConfigSection, {
          kit: sampleKit(),
          server: sampleServer({ state: "stopped" }),
          initialConfig: cfg,
        })
      )

      expect(html).toContain('data-mongo-migration="migrated"')
      expect(html).toContain('data-mongo-readonly="true"')
      expect(html).toContain(
        htmlEscape(translate(lang, "hsrv_mongo_migrated_title"))
      )
      expect(html).toContain(htmlEscape(translate(lang, "hsrv_mongo_migrated")))
      expect(html).not.toContain('data-action="mongo-migrate"')
    })

    test(`renders fresh-start (skipped) state and states that MongoDB was never modified in ${lang}`, () => {
      const cfg = sampleConfig({
        freshStart: true,
        mongoMigration: {
          state: "skipped",
          source: null,
          detail: null,
        },
      })
      const html = renderInLanguage(
        lang,
        createElement(ServerConfigSection, {
          kit: sampleKit(),
          server: sampleServer({ state: "stopped" }),
          initialConfig: cfg,
        })
      )

      expect(html).toContain('data-mongo-migration="skipped"')
      expect(html).toContain('data-mongo-readonly="true"')
      expect(html).toContain(
        htmlEscape(translate(lang, "hsrv_mongo_skipped_title"))
      )
      expect(html).toContain(htmlEscape(translate(lang, "hsrv_mongo_skipped")))
      expect(html).not.toContain('data-action="mongo-migrate"')
    })
  }

  test("no bridge method or UI action modifies or deletes MongoDB collections", () => {
    const bridgeMethods = Object.keys(api)
    // Only read-only migration to SQLite/Postgres and fresh-start marker confirmation exist on the bridge
    expect(bridgeMethods).toContain("serverMigrateMongo")
    expect(bridgeMethods).toContain("serverConfirmFreshStart")
    expect(
      bridgeMethods.some((m) =>
        /deleteMongo|dropMongo|writeMongo|modifyMongo|clearMongo/i.test(m)
      )
    ).toBe(false)
    for (const lang of langs) {
      expect(translate(lang, "hsrv_mongo_migrated")).toMatch(
        /never modified|nunca foram modificados/i
      )
      expect(translate(lang, "hsrv_mongo_skipped")).toMatch(
        /never modified|nunca foram modificados/i
      )
    }
  })
})

describe("DoctorPanel, StatusTab, AdminTab, and HistoryTab in en and pt-BR (T4)", () => {
  for (const lang of langs) {
    test(`renders DoctorPanel report with ok, warning, and problem checks in ${lang}`, () => {
      const report: DoctorReportView = {
        warnings: 1,
        problems: 1,
        checks: [
          {
            level: "ok",
            label: "host.toml",
            detail: "Private server Friday Hunts configured",
          },
          {
            level: "warning",
            label: "gamedata",
            detail: "Using synthetic example game data",
          },
          {
            level: "problem",
            label: "runtime",
            detail: "MysticParadox.dll is not published yet",
          },
        ],
      }
      const html = renderInLanguage(
        lang,
        createElement(DoctorPanel, {
          kit: sampleKit(),
          initialReport: report,
        })
      )

      expect(html).toContain('data-doctor-checks=""')
      expect(html).toContain(htmlEscape(translate(lang, "hsrv_doc_title")))
      expect(html).toContain(htmlEscape(translate(lang, "hsrv_doc_run")))
      expect(html).toContain(
        htmlEscape(
          translate(lang, "hsrv_doc_summary", { problems: 1, warnings: 1 })
        )
      )
      expect(html).toContain("Private server Friday Hunts configured")
      expect(html).toContain("Using synthetic example game data")
      expect(html).toContain("MysticParadox.dll is not published yet")
    })

    test(`renders StatusTab with embedded server components and restart controls in ${lang}`, () => {
      const html = renderInLanguage(
        lang,
        createElement(StatusTab, {
          server: sampleServer({ state: "running" }),
        })
      )

      expect(html).toContain(htmlEscape(translate(lang, "hstat_title")))
      expect(html).toContain(
        htmlEscape(translate(lang, "hstat_summary", { ready: 4, total: 4 }))
      )
      expect(html).toContain('data-component="gateway"')
      expect(html).toContain('data-component="isles-1-14-7"')
    })

    test(`renders AdminTab player controls and ban/lookup result card in ${lang}`, () => {
      const result: AdminResultView = {
        action: "ban",
        ok: true,
        message: "Banned Slayer_42.",
        player: {
          globalId: "059b4bd8-8e4e-4598-a2f6-a9b06ca0cd54",
          moduleUserId: "acc_42",
          displayName: "Slayer_42",
          banned: true,
          banReason: "Griefing in Ramsgate",
          timeoutUntil: null,
          online: false,
          createdAt: "2026-10-29T20:00:00Z",
          lastLoginAt: "2026-10-29T21:00:00Z",
        },
      }
      const html = renderInLanguage(
        lang,
        createElement(AdminTab, {
          server: sampleServer({ state: "running" }),
          initialPlayer: "Slayer_42",
          initialResult: result,
        })
      )

      expect(html).toContain(htmlEscape(translate(lang, "hadm_title")))
      expect(html).toContain(htmlEscape(translate(lang, "hadm_lookup")))
      expect(html).toContain(htmlEscape(translate(lang, "hadm_kick")))
      expect(html).toContain(htmlEscape(translate(lang, "hadm_timeout")))
      expect(html).toContain(htmlEscape(translate(lang, "hadm_ban")))
      expect(html).toContain(htmlEscape(translate(lang, "hadm_unban")))
      expect(html).toContain('data-admin-result="ban"')
      expect(html).toContain('data-admin-player="Slayer_42"')
      expect(html).toContain(htmlEscape(translate(lang, "hadm_banned")))
      expect(html).toContain(htmlEscape(translate(lang, "hadm_offline")))
      expect(html).toContain("Griefing in Ramsgate")
    })

    test(`renders HistoryTab entries, revert dry-run plan, applied summary, and guarded actions in ${lang}`, () => {
      const rows: HistoryEntryView[] = [
        {
          id: "42",
          at: "2026-10-29T22:00:00Z",
          actor: "host:console",
          database: "isles",
          table: "accounts",
          op: "update",
          key: "Slayer_42",
          before: '{"banned":false}',
          after: '{"banned":true}',
        },
      ]
      const plan: RevertPlanView = {
        id: "plan-99",
        entries: 3,
        actionCount: 2,
        conflictCount: 0,
        unchangedCount: 1,
        actions: [
          {
            database: "isles",
            table: "accounts",
            key: "Slayer_42",
            kind: "restore",
            current: '{"banned":true}',
            target: '{"banned":false}',
          },
        ],
        conflicts: [],
        truncated: false,
      }
      const applied: RevertDoneView = {
        at: "2026-10-29T22:01:00Z",
        databases: [
          {
            database: "isles",
            rows: 2,
          },
        ],
        rowsChanged: 2,
        conflicts: [],
        conflictCount: 0,
        truncated: false,
      }
      const guarded: GuardedActionInfo[] = [
        {
          id: "grd-1",
          kind: "bulk_ban",
          summary: "Ban 12 accounts",
          initiatedBy: "admin:58ef5419",
          needsRole: "owner",
          createdAt: "2026-10-29T21:50:00Z",
          releaseAt: "2026-10-29T22:20:00Z",
          expiresAt: "2026-10-30T21:50:00Z",
          approvedBy: null,
          canceledBy: null,
          endedAt: null,
          result: null,
          state: "pending",
          mode: "approval",
        },
      ]

      const html = renderInLanguage(
        lang,
        createElement(HistoryTab, {
          initialRows: rows,
          initialSearched: true,
          initialPlan: plan,
          initialApplied: applied,
          initialGuarded: guarded,
        })
      )

      expect(html).toContain(htmlEscape(translate(lang, "hhis_title")))
      expect(html).toContain(htmlEscape(translate(lang, "hhis_search")))
      expect(html).toContain(htmlEscape(translate(lang, "hhis_dry_run")))
      expect(html).toContain(
        htmlEscape(translate(lang, "hhis_apply", { count: 2 }))
      )
      expect(html).toContain(
        htmlEscape(
          translate(lang, "hhis_applied", { reverted: 2, accounts: 1 })
        )
      )
      expect(html).toContain('data-revert-plan="plan-99"')
      expect(html).toContain('data-history-rows=""')
      expect(html).toContain("Slayer_42")
      expect(html).toContain(htmlEscape(translate(lang, "hhis_guarded_title")))
      expect(html).toContain("Ban 12 accounts")
      expect(html).toContain(
        htmlEscape(translate(lang, "hhis_guarded_approve"))
      )
      expect(html).toContain(htmlEscape(translate(lang, "hhis_guarded_cancel")))
    })
  }

  test("submitAdminAction calls api.adminAction with normalized arguments for every admin action", async () => {
    const calls: AdminRequest[] = []
    const orig = api.adminAction
    api.adminAction = async (request: AdminRequest) => {
      calls.push(request)
      return {
        ok: true,
        result: {
          action: request.action,
          ok: true,
          message: `ran ${request.action}`,
          player: null,
        },
      }
    }
    try {
      const actions: AdminActionKind[] = [
        "lookup",
        "kick",
        "timeout",
        "ban",
        "unban",
      ]
      for (const action of actions) {
        const res = await submitAdminAction(
          action,
          "  Slayer_42  ",
          action === "ban" ? "  Rule 1  " : "   ",
          action === "timeout" ? " 30m " : ""
        )
        expect(res.ok).toBe(true)
      }
      expect(calls).toEqual([
        {
          action: "lookup",
          player: "Slayer_42",
          reason: null,
          until: null,
          module: null,
        },
        {
          action: "kick",
          player: "Slayer_42",
          reason: null,
          until: null,
          module: null,
        },
        {
          action: "timeout",
          player: "Slayer_42",
          reason: null,
          until: "30m",
          module: null,
        },
        {
          action: "ban",
          player: "Slayer_42",
          reason: "Rule 1",
          until: null,
          module: null,
        },
        {
          action: "unban",
          player: "Slayer_42",
          reason: null,
          until: null,
          module: null,
        },
      ])
    } finally {
      api.adminAction = orig
    }
  })

  test("stub and refusal errors map to localized catalog messages in en and pt-BR, never raw error codes", () => {
    const formatMsg = (lang: Language, m: Message): string => {
      if (!m.vars) return translate(lang, m.key)
      const plain: Record<string, string | number> = {}
      for (const [k, v] of Object.entries(m.vars)) {
        plain[k] = typeof v === "object" ? translate(lang, v.key) : v
      }
      return translate(lang, m.key, plain)
    }
    const refusalErrors: LauncherError[] = [
      { code: "not_in_this_build", detail: "host_download_project" },
      { code: "not_implemented", detail: "custom_stub" },
      { code: "server_stopped" },
      { code: "host_failed", detail: "workspace_mgr" },
      { code: "host_failed", detail: "not_configured" },
      { code: "host_failed", detail: "server_running" },
    ]
    for (const err of refusalErrors) {
      const mapped = errorMessage(err, "private")
      for (const lang of langs) {
        const text = formatMsg(lang, mapped)
        expect(text.length).toBeGreaterThan(10)
        expect(text).not.toBe(err.code)
        expect(text).not.toContain("not_in_this_build")
        expect(text).not.toContain("not_implemented")
        expect(text).not.toContain("workspace_mgr")
      }
    }
  })

  for (const lang of langs) {
    test(`renders ManagedInstallsPanel in standalone and controller modes in ${lang}, and omits it in client launcher`, () => {
      const kit = sampleKit()
      const standaloneHtml = renderInLanguage(
        lang,
        createElement(ManagedInstallsPanel, {
          kit,
          initialStatus: {
            role: "standalone",
            node: null,
            region: null,
            controllerUrl: null,
            controllerCertFp: null,
            controllerNode: null,
            revision: 0,
            pendingEnrollments: 0,
            installs: [],
            detached: false,
          },
        })
      )
      expect(standaloneHtml).toContain(htmlEscape(translate(lang, "hrem_title")))
      expect(standaloneHtml).toContain('data-remote-role="standalone"')
      expect(standaloneHtml).toContain('data-action="remote-init-controller"')
      expect(standaloneHtml).toContain('data-action="remote-enroll"')

      const controllerHtml = renderInLanguage(
        lang,
        createElement(ManagedInstallsPanel, {
          kit,
          initialStatus: {
            role: "controller",
            node: "ctrl-main",
            region: "us-east",
            controllerUrl: "https://ctrl.example.com:8443",
            controllerCertFp:
              "SHA256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            controllerNode: null,
            revision: 3,
            pendingEnrollments: 1,
            installs: [
              {
                node: "eu-alpha",
                region: "eu-west",
                label: "EU Hunt Node Alpha",
                state: "online",
                enrolledAtMs: 1700000000000,
                lastSeenMs: 1700000030000,
                revoked: false,
                appliedRevision: 3,
                playersOnline: 12,
                activeHunts: 3,
                version: "2.0.0-alpha.0",
                queuedCommands: 0,
              },
            ],
            detached: false,
          },
          initialInvite: {
            uri: "dainty-lass-remote://enroll?v=1&node=eu-beta",
            node: "eu-beta",
            region: "eu-west",
            label: "EU Beta",
            controllerUrl: "https://ctrl.example.com:8443",
            controllerFp:
              "SHA256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            expiresAtMs: 1700000900000,
          },
        })
      )
      expect(controllerHtml).toContain('data-remote-role="controller"')
      expect(controllerHtml).toContain('data-managed-node="eu-alpha"')
      expect(controllerHtml).toContain("EU Hunt Node Alpha")
      expect(controllerHtml).toContain("12 / 3")
      expect(controllerHtml).toContain('data-remote-invite="eu-beta"')
      expect(controllerHtml).toContain('data-action="remote-revoke-eu-alpha"')

      const clientSnap = sampleManagerSnap({
        app: {
          version: "2.0.0-alpha.0",
          packaged: true,
          updateReady: false,
          platform: "win32",
          hostToolsAvailable: true,
          serverGui: false,
          legacyAvailable: true,
          gameVersions: ["1.14.7", "1.12.0", "1.4.4"],
        },
      })
      const clientHtml = renderInLanguage(
        lang,
        createElement(ServerTab, {
          kit,
          server: sampleServer({ state: "stopped" }),
          snap: clientSnap,
          setTab: () => undefined,
        })
      )
      expect(clientHtml).not.toContain(htmlEscape(translate(lang, "hrem_title")))
    })
  }
})
