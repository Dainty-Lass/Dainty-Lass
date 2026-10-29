// The Host page's tools: what this PC needs (requirements), the game's SDK and the runtime built from it, and the
// game data the server runs on. Each step is detected from files on this PC by the backend; the page only names
// the action to run (`host_action`), never a path or a command.
import { useState } from "react"
import type { ReactNode } from "react"
import { CheckIcon, FolderIcon, RefreshCwIcon, XIcon } from "lucide-react"

import {
  api,
  type GameDataState,
  type HostAction,
  type HostFolder,
  type HostKit,
  type Requirement,
  type Snapshot,
} from "@workspace/bridge"
import { isKey, type Key } from "@workspace/i18n"
import { Badge } from "@workspace/ui/components/badge"
import { Button } from "@workspace/ui/components/button"

import {
  REQUIREMENT_GROUPS,
  REQUIREMENT_KEYS,
  REQUIREMENT_LINKS,
  requirementSteps,
} from "@/app/host-model"
import { refreshKit, useAct } from "@/app/host-store"
import { useI18n } from "@/app/i18n"
import { ExternalLink, Panel, Row, Spinner } from "@/components/parts"
import { Secret } from "@/components/secret"
import { toneClass } from "@/components/tone"

function Yes({ ok, yes, no }: { ok: boolean; yes: Key; no: Key }) {
  const { t } = useI18n()
  return (
    <Badge variant="outline" className={toneClass(ok ? "good" : "idle")}>
      {ok ? <CheckIcon /> : <XIcon />}
      {t(ok ? yes : no)}
    </Badge>
  )
}

// A step: its words, and the buttons that do it
function Step({
  text,
  hint,
  children,
}: {
  text: ReactNode
  hint?: ReactNode
  children?: ReactNode
}) {
  return (
    <Row title={<span className="font-normal">{text}</span>} sub={hint}>
      {children}
    </Row>
  )
}

function useActions(kit: HostKit) {
  const act = useAct()
  const working = kit.job?.state === "running"
  return {
    working,
    run: (action: HostAction) => act(() => api.hostAction(action)),
    act,
  }
}

function OpenFolder({ folder }: { folder: HostFolder }) {
  const { t } = useI18n()
  return (
    <Button variant="ghost" size="sm" onClick={() => void api.hostOpen(folder)}>
      <FolderIcon />
      {t("host_open")}
    </Button>
  )
}

// ------------------------------------------------------------------ requirements

function problemKey(requirement: Requirement): Key | null {
  const key = `req_${requirement.id}_${requirement.problem ?? ""}`
  return requirement.problem && isKey(key) ? key : null
}

function RequirementRow({ requirement }: { requirement: Requirement }) {
  const { t } = useI18n()
  const [steps, setSteps] = useState(false)
  const names = REQUIREMENT_KEYS[requirement.id]
  const lines = requirementSteps(requirement.id)
  const problem = problemKey(requirement)
  const [main] = REQUIREMENT_LINKS[requirement.id]
  return (
    <div
      className="flex flex-col gap-2 border-b py-3 last:border-b-0"
      data-requirement={requirement.id}
      data-state={requirement.state}
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-1">
          <span className="flex flex-wrap items-center gap-3 text-sm font-medium">
            {t(names.name)}
            <Badge
              variant="outline"
              className={toneClass(
                requirement.state === "ok"
                  ? "good"
                  : requirement.state === "problem"
                    ? "busy"
                    : "bad"
              )}
            >
              {t(`hreq_state_${requirement.state}`)}
            </Badge>
          </span>
          <span className="text-xs text-muted-foreground">{t(names.why)}</span>
          {requirement.detail && (
            <span className="text-xs break-all text-muted-foreground">
              <Secret text={requirement.detail} />
            </span>
          )}
        </div>
        {requirement.state !== "ok" && (
          <div className="flex flex-wrap items-center gap-2">
            {main && (
              <Button
                size="sm"
                variant="outline"
                onClick={() => void api.openExternal(main)}
              >
                {t("hreq_download")}
              </Button>
            )}
            {lines.length > 0 && (
              <Button
                size="sm"
                variant="ghost"
                onClick={() => setSteps(!steps)}
              >
                {t(steps ? "hreq_hide" : "hreq_how")}
              </Button>
            )}
          </div>
        )}
      </div>
      {requirement.state === "problem" && problem && (
        <p className="text-sm text-amber-600 dark:text-amber-400">
          {t(problem, { version: requirement.detail ?? "" })}
        </p>
      )}
      {steps && requirement.state !== "ok" && (
        <ol className="flex list-decimal flex-col gap-1 pl-5 text-sm">
          {lines.map((key) => (
            <li key={key}>{t(key)}</li>
          ))}
        </ol>
      )}
    </div>
  )
}

export function RequirementsTab({ kit }: { kit: HostKit }) {
  const { t } = useI18n()
  const [checking, setChecking] = useState(false)
  const recheck = async () => {
    setChecking(true)
    try {
      await refreshKit()
    } finally {
      setChecking(false)
    }
  }
  return (
    <>
      <p className="max-w-prose text-sm text-muted-foreground">
        {t("hreq_intro")}
      </p>
      {REQUIREMENT_GROUPS.map((group) => {
        const rows = kit.requirements.filter((r) => group.ids.includes(r.id))
        return (
          rows.length > 0 && (
            <Panel key={group.key} title={t(group.key)}>
              <div className="flex flex-col">
                {rows.map((requirement) => (
                  <RequirementRow
                    key={requirement.id}
                    requirement={requirement}
                  />
                ))}
              </div>
            </Panel>
          )
        )
      })}
      <div>
        <Button
          variant="outline"
          size="sm"
          disabled={checking}
          onClick={() => void recheck()}
        >
          {checking ? <Spinner /> : <RefreshCwIcon />}
          {t("hreq_recheck")}
        </Button>
      </div>
    </>
  )
}

// ------------------------------------------------------------------ SDK, runtime, publish

export function BuildTab({ kit, snap }: { kit: HostKit; snap: Snapshot }) {
  const { t } = useI18n()
  const { working, run, act } = useActions(kit)
  const dump = kit.dumps[0]
  const hasGame = snap.game.root !== null
  const builds: {
    action: HostAction
    file: string
    done: boolean
    key: Key
  }[] = [
    {
      action: "build_runtime",
      file: "MysticParadox.dll",
      done: kit.builds.runtime,
      key: "host_build_runtime",
    },
    {
      action: "build_loader",
      file: "winmm.dll",
      done: kit.builds.loader,
      key: "host_build_loader",
    },
    {
      action: "build_exporter",
      file: "CatalogExporter.dll",
      done: kit.builds.exporter,
      key: "host_build_exporter",
    },
  ]
  return (
    <>
      <Panel
        title={
          <span className="flex flex-wrap items-center gap-3">
            {t("host_sdk_title")}
            <Badge
              variant="outline"
              className={toneClass(kit.sdk.present ? "good" : "idle")}
            >
              {kit.sdk.present
                ? t("host_sdk_ready", { n: kit.sdk.functionFiles })
                : t("host_sdk_missing")}
            </Badge>
          </span>
        }
        text={t("host_sdk_text")}
      >
        <Step
          text={t("host_sdk_step_dumper")}
          hint={kit.dumperDll ? <Secret text={kit.dumperDll} /> : undefined}
        >
          <Button
            variant="outline"
            size="sm"
            disabled={working}
            onClick={() => act(() => api.hostChooseDumper())}
          >
            {t("host_sdk_choose")}
          </Button>
          <ExternalLink target="dumper7">{t("host_sdk_get")}</ExternalLink>
        </Step>
        <Step
          text={t("host_sdk_step_run")}
          hint={hasGame ? undefined : t("host_sdk_need_game")}
        >
          <Button
            size="sm"
            disabled={working || !hasGame || !kit.dumperDll}
            onClick={() => run("inject_dumper")}
          >
            {t("host_sdk_run")}
          </Button>
        </Step>
        <Step
          text={t("host_sdk_step_import")}
          hint={
            <>
              <span className="block">
                {dump
                  ? t("host_sdk_found", {
                      name: dump.name,
                      date: dump.modified,
                    })
                  : t("host_sdk_none")}
              </span>
              {!kit.upstream.present && (
                <span className="block">{t("host_sdk_need_setup")}</span>
              )}
            </>
          }
        >
          <Button
            size="sm"
            disabled={working || !dump}
            onClick={() => run("import_sdk")}
          >
            {t("host_sdk_import")}
          </Button>
          <Button
            variant="outline"
            size="sm"
            disabled={working}
            onClick={() => act(() => api.hostImportSdkFolder())}
          >
            {t("host_sdk_choose_folder")}
          </Button>
          <OpenFolder folder="dumps" />
        </Step>
      </Panel>
      <Panel title={t("host_build_title")} text={t("host_build_text")}>
        {builds.map((build) => (
          <Row
            key={build.action}
            title={<span className="font-mono">{build.file}</span>}
          >
            <Yes ok={build.done} yes="host_built" no="host_not_built" />
            <Button
              variant="outline"
              size="sm"
              disabled={working}
              onClick={() => run(build.action)}
            >
              {t(build.key)}
            </Button>
          </Row>
        ))}
        <div className="flex flex-wrap items-center gap-4">
          {kit.toolchain.visualStudio === null && (
            <span className="flex items-center gap-2 text-sm text-amber-600 dark:text-amber-400">
              {t("host_build_no_vs")}
              <ExternalLink target="visual_studio">
                {t("host_get_vs")}
              </ExternalLink>
            </span>
          )}
          {!kit.toolchain.cargo && (
            <span className="flex items-center gap-2 text-sm text-amber-600 dark:text-amber-400">
              {t("host_build_no_cargo")}
              <ExternalLink target="rust">{t("host_get_rust")}</ExternalLink>
            </span>
          )}
          <OpenFolder folder="runtime_source" />
        </div>
      </Panel>
      <Panel title={t("host_publish_title")} text={t("host_publish_text")}>
        <Row
          title={
            <Yes
              ok={kit.builds.published}
              yes="host_publish_ready"
              no="host_publish_missing"
            />
          }
        >
          <Button
            size="sm"
            disabled={working || !kit.builds.runtime || !kit.builds.loader}
            onClick={() => run("publish_runtime")}
          >
            {t("host_publish")}
          </Button>
          <OpenFolder folder="runtime_publish" />
        </Row>
      </Panel>
    </>
  )
}

// ------------------------------------------------------------------ game data

const DATA_STATE_KEYS: Record<GameDataState, Key> = {
  real: "host_data_state_real",
  synthetic: "host_data_state_synthetic",
  missing: "host_data_state_missing",
}

export function GameDataTab({ kit, snap }: { kit: HostKit; snap: Snapshot }) {
  const { t } = useI18n()
  const { working, run, act } = useActions(kit)
  const { backend, director } = kit.gameData
  return (
    <Panel title={t("host_data_title")} text={t("host_data_text")}>
      <div className="flex flex-wrap gap-6 text-sm">
        <span>
          {t("host_data_backend", { state: t(DATA_STATE_KEYS[backend]) })}
        </span>
        <span>
          {t("host_data_director", { state: t(DATA_STATE_KEYS[director]) })}
        </span>
      </div>
      <Step text={t("host_data_step_synthetic")}>
        <Button
          variant="outline"
          size="sm"
          disabled={working}
          onClick={() => run("synthetic_data")}
        >
          {t("host_data_synthetic")}
        </Button>
      </Step>
      <Step
        text={t("host_data_step_export")}
        hint={
          !kit.builds.exporter
            ? t("host_data_need_exporter")
            : snap.running.game
              ? undefined
              : t("host_data_need_game")
        }
      >
        <Button
          size="sm"
          disabled={working || !kit.builds.exporter || !snap.running.game}
          onClick={() => run("inject_exporter")}
        >
          {t("host_data_inject")}
        </Button>
      </Step>
      <Step text={t("host_data_step_copy")}>
        <Button
          size="sm"
          disabled={working || !kit.exports.inGame}
          onClick={() => run("copy_exports")}
        >
          {t("host_data_copy")}
        </Button>
        <OpenFolder folder="game_data" />
      </Step>
      <Step text={t("host_data_step_generate")}>
        <Button
          size="sm"
          disabled={working || !kit.exports.present}
          onClick={() => run("generate_gamedata")}
        >
          {t("host_data_generate")}
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={working || backend !== "real"}
          onClick={() => run("export_gamedata")}
        >
          {t("host_data_export_pack")}
        </Button>
        <OpenFolder folder="gamedata_packs" />
      </Step>
      <Step text={t("host_data_step_pack_rust")}>
        <Button
          variant="outline"
          size="sm"
          disabled={working}
          onClick={() => act(() => api.hostImportGameData())}
        >
          {t("host_data_import_pack")}
        </Button>
      </Step>
    </Panel>
  )
}
