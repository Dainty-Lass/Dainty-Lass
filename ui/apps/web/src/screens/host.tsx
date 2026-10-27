// The Host page: the server this PC runs, ported from the Electron launcher's, and, in the server GUI, the
// per-component status and the module panels (DU3). Every action goes through the bridge's `host_*` and
// `module_*` commands; the page holds only what the player is typing.
import { useEffect, useRef, useState } from "react"
import type { ReactNode } from "react"
import {
  CheckIcon,
  CircleAlertIcon,
  CopyIcon,
  FolderIcon,
  PlayIcon,
  RefreshCwIcon,
  SquareIcon,
  XIcon,
} from "lucide-react"

import {
  api,
  type HostKit,
  type HostServerMode,
  type HostServerView,
  type ModuleView,
  type Snapshot,
} from "@workspace/bridge"
import { Alert, AlertDescription } from "@workspace/ui/components/alert"
import { Badge } from "@workspace/ui/components/badge"
import { Button } from "@workspace/ui/components/button"
import { Input } from "@workspace/ui/components/input"
import { Label } from "@workspace/ui/components/label"
import {
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from "@workspace/ui/components/tabs"

import {
  canStart,
  checks,
  HOST_TAB_KEYS,
  hostTabs,
  JOB_KEYS,
  multiplier,
  PHASE_KEYS,
  phaseTone,
  readyCount,
  SERVER_STATE_KEYS,
  statusRows,
  type HostTab,
} from "@/app/host-model"
import { startHostStore, useAct, useHost } from "@/app/host-store"
import { useI18n } from "@/app/i18n"
import { useLauncher } from "@/app/launcher"
import { Choice } from "@/components/choice"
import { Heading, Panel, Row, Spinner } from "@/components/parts"
import { Secret, useHidden } from "@/components/secret"
import { toneClass } from "@/components/tone"
import { BuildTab, GameDataTab, RequirementsTab } from "@/screens/host-tools"
import { ModulesTab } from "@/screens/modules"

function JobBar({ kit }: { kit: HostKit }) {
  const { t } = useI18n()
  const job = kit.job
  if (!job) return null
  return (
    <Alert role="status" data-job={job.kind}>
      {job.state === "running" ? <Spinner /> : <CircleAlertIcon />}
      <AlertDescription>
        <span>
          {t(JOB_KEYS[job.kind])}: {t(`host_job_state_${job.state}`)}
        </span>
        {job.message && (
          <span className="block font-mono text-xs break-all">
            {job.message}
          </span>
        )}
      </AlertDescription>
      {job.logFile && (
        <Button
          variant="ghost"
          size="xs"
          onClick={() => void api.hostOpen("host_logs")}
        >
          {t("host_open_logs")}
        </Button>
      )}
    </Alert>
  )
}

function Workspace({ kit, snap }: { kit: HostKit; snap: Snapshot }) {
  const { t } = useI18n()
  const act = useAct()
  const state = !kit.workspaceOk
    ? "host_ws_missing"
    : kit.upstream.patched
      ? "host_ws_ready"
      : "host_ws_unpatched"
  const working = kit.job?.state === "running"
  return (
    <Panel title={t("host_ws_title")} text={t("host_ws_text")}>
      <Row
        title={t(state)}
        sub={kit.workspace ? <Secret text={kit.workspace} /> : undefined}
      >
        <Button
          variant="outline"
          size="sm"
          disabled={working}
          onClick={() => act(() => api.hostChooseWorkspace())}
        >
          <FolderIcon />
          {t("host_ws_choose")}
        </Button>
        {kit.workspace && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void api.hostOpen("workspace")}
          >
            {t("host_open")}
          </Button>
        )}
      </Row>
      {!kit.workspaceOk && (
        <Row
          title={t("host_ws_download")}
          sub={
            kit.projectTarget ? (
              <Secret
                text={t("host_ws_download_to", { folder: kit.projectTarget })}
              />
            ) : (
              t("host_ws_download_no_game")
            )
          }
        >
          <Button
            size="sm"
            disabled={working || !kit.projectTarget || !snap.game.root}
            onClick={() => act(() => api.hostDownloadProject())}
          >
            {t("host_ws_download")}
          </Button>
        </Row>
      )}
    </Panel>
  )
}

function ServerPanel({
  kit,
  server,
  setTab,
}: {
  kit: HostKit
  server: HostServerView
  setTab(tab: HostTab): void
}) {
  const { t } = useI18n()
  const act = useAct()
  const hide = useHidden()
  const keys = SERVER_STATE_KEYS[server.state]
  const working = kit.job?.state === "running"
  const running = server.state === "running"
  const changing = server.state === "starting" || server.state === "stopping"
  const fix = (id: string): (() => void) | null => {
    switch (id) {
      case "workspace":
        return () => act(() => api.hostChooseWorkspace())
      case "gamedata":
        return () => setTab("gamedata")
      case "runtime":
        return () => setTab("build")
      default:
        return null
    }
  }
  return (
    <Panel
      title={
        <span className="flex flex-wrap items-center gap-3">
          <span>{t("hsrv_title")}</span>
          <Badge
            variant="outline"
            data-state={server.state}
            className={toneClass(
              running
                ? "good"
                : changing
                  ? "busy"
                  : server.state === "failed"
                    ? "bad"
                    : "idle"
            )}
          >
            {changing && <Spinner />}
            {t(keys.state)}
          </Badge>
        </span>
      }
      text={
        server.state === "failed" && server.message
          ? t("hsrv_text_failed", { reason: server.message })
          : t(keys.text)
      }
    >
      {kit.serverConfig.host && (
        <p className="text-sm">
          {t("hsrv_address", { address: hide(kit.serverConfig.host) })}
        </p>
      )}
      <div className="flex flex-wrap items-center gap-2">
        {running || server.state === "stopping" ? (
          <Button
            variant="outline"
            disabled={changing}
            onClick={() => act(() => api.hostServerStop())}
          >
            <SquareIcon />
            {t("hsrv_stop")}
          </Button>
        ) : (
          <Button
            disabled={
              working ||
              changing ||
              server.state === "external" ||
              !canStart(kit)
            }
            onClick={() => act(() => api.hostServerStart())}
          >
            <PlayIcon />
            {t("hsrv_start")}
          </Button>
        )}
      </div>
      <div className="flex flex-col gap-1">
        <h3 className="text-xs font-semibold tracking-widest text-muted-foreground uppercase">
          {t("hsrv_checks")}
        </h3>
        <ul className="flex flex-col">
          {checks(kit).map((check) => {
            const fixer = !check.ok ? fix(check.id) : null
            return (
              <li
                key={check.id}
                data-check={check.id}
                data-ok={check.ok}
                className="flex items-center gap-2 border-b py-2 text-sm last:border-b-0"
              >
                {check.ok ? (
                  <CheckIcon
                    className={
                      check.warn ? "text-amber-500" : "text-emerald-500"
                    }
                  />
                ) : (
                  <XIcon className="text-destructive" />
                )}
                <span className="flex-1">{t(check.key)}</span>
                {fixer && (
                  <Button variant="outline" size="xs" onClick={fixer}>
                    {t("hsrv_fix")}
                  </Button>
                )}
              </li>
            )
          })}
        </ul>
      </div>
    </Panel>
  )
}

function SetupPanel({
  kit,
  server,
  snap,
}: {
  kit: HostKit
  server: HostServerView
  snap: Snapshot
}) {
  const { t } = useI18n()
  const act = useAct()
  const config = kit.serverConfig
  const [name, setName] = useState(config.name ?? "")
  const [mode, setMode] = useState<HostServerMode>(config.mode ?? "private")
  const [host, setHost] = useState(
    config.mode === "public" ? (config.host ?? "") : ""
  )
  const stopped = server.state === "stopped" || server.state === "failed"
  const valid = name.trim() !== "" && (mode === "private" || host.trim() !== "")
  const working = kit.job?.state === "running"
  return (
    <Panel title={t("hsrv_setup_title")} text={t("hsrv_setup_text")}>
      {config.configured && (
        <p className="text-sm">
          {t("hsrv_setup_done", {
            name: config.name ?? "",
            mode: t(
              config.mode === "public"
                ? "hsrv_mode_short_public"
                : "hsrv_mode_short_private"
            ),
          })}
        </p>
      )}
      <div className="flex flex-col gap-1">
        <Label htmlFor="host-name">{t("hsrv_setup_name")}</Label>
        <Input
          id="host-name"
          value={name}
          maxLength={60}
          autoComplete="off"
          onChange={(event) => setName(event.target.value)}
        />
      </div>
      <div className="flex flex-col gap-1">
        <Label htmlFor="host-mode">{t("hsrv_setup_mode")}</Label>
        <Choice<HostServerMode>
          id="host-mode"
          value={mode}
          disabled={!stopped}
          options={[
            { value: "private", label: t("hsrv_mode_private") },
            { value: "public", label: t("hsrv_mode_public") },
          ]}
          onChange={setMode}
        />
      </div>
      {mode === "public" && (
        <div className="flex flex-col gap-1">
          <Label htmlFor="host-public">{t("hsrv_setup_host")}</Label>
          <Input
            id="host-public"
            value={host}
            maxLength={253}
            autoComplete="off"
            onChange={(event) => setHost(event.target.value)}
          />
        </div>
      )}
      {mode === "private" && (
        <p className="text-xs text-muted-foreground">
          {t("hsrv_setup_tailscale")}
        </p>
      )}
      <p className="text-xs text-muted-foreground">
        {snap.game.root ? (
          <Secret text={t("hsrv_setup_game", { folder: snap.game.root })} />
        ) : (
          t("hsrv_setup_no_game")
        )}
      </p>
      <div>
        <Button
          disabled={!valid || !stopped || working || !kit.workspaceOk}
          onClick={() =>
            act(() =>
              api.hostSetup({
                name: name.trim(),
                mode,
                publicHost: mode === "public" ? host.trim() : null,
              })
            )
          }
        >
          {t(config.configured ? "hsrv_setup_update" : "hsrv_setup_run")}
        </Button>
      </div>
    </Panel>
  )
}

function InvitePanel({
  kit,
  server,
}: {
  kit: HostKit
  server: HostServerView
}) {
  const { t } = useI18n()
  const act = useAct()
  const [who, setWho] = useState("")
  const [copied, setCopied] = useState(false)
  const running = server.state === "running"
  const invite = server.invite
  useEffect(() => {
    if (!copied) return
    const timer = setTimeout(() => setCopied(false), 2000)
    return () => clearTimeout(timer)
  }, [copied])
  return (
    <Panel title={t("hsrv_invite_title")} text={t("hsrv_invite_text")}>
      <div className="flex flex-col gap-1">
        <Label htmlFor="host-invite-for">{t("hsrv_invite_for")}</Label>
        <Input
          id="host-invite-for"
          value={who}
          maxLength={40}
          autoComplete="off"
          onChange={(event) => setWho(event.target.value)}
        />
      </div>
      <div className="flex flex-wrap items-center gap-3">
        <Button
          disabled={!running || kit.job?.state === "running"}
          onClick={() =>
            act(() =>
              api.hostCreateInvite(who.trim() === "" ? null : who.trim())
            )
          }
        >
          {t("hsrv_invite_create")}
        </Button>
        {!running && (
          <span className="text-xs text-muted-foreground">
            {t("hsrv_invite_need_running")}
          </span>
        )}
      </div>
      {invite && (
        <div className="flex flex-col gap-2" data-invite="">
          <p className="text-sm font-medium">
            {t("hsrv_invite_ready", {
              for: invite.for ? ` (${invite.for})` : "",
            })}
          </p>
          <code className="block bg-muted p-2 text-xs break-all">
            <Secret text={invite.link} />
          </code>
          <div>
            <Button
              variant="outline"
              size="sm"
              onClick={() =>
                void api.copyText(invite.link).then((ok) => setCopied(ok))
              }
            >
              <CopyIcon />
              {t(copied ? "hsrv_invite_copied" : "hsrv_invite_copy")}
            </Button>
          </div>
          {kit.serverConfig.mode === "private" && (
            <p className="text-xs text-muted-foreground">
              {t("hsrv_invite_tailscale")}
            </p>
          )}
        </div>
      )}
    </Panel>
  )
}

function Multipliers({ kit }: { kit: HostKit }) {
  const { t } = useI18n()
  const act = useAct()
  const current = kit.serverConfig.multipliers
  const [xp, setXp] = useState(String(current?.xp ?? 1))
  const [rewards, setRewards] = useState(String(current?.rewards ?? 1))
  const parsed = { xp: multiplier(xp), rewards: multiplier(rewards) }
  const values =
    parsed.xp !== null && parsed.rewards !== null
      ? { xp: parsed.xp, rewards: parsed.rewards }
      : null
  if (kit.serverConfig.mode !== "private") return null
  return (
    <Panel title={t("hsrv_mult_title")} text={t("hsrv_mult_text")}>
      {current && (
        <p className="text-sm">
          {t("hsrv_mult_state", { xp: current.xp, rewards: current.rewards })}
        </p>
      )}
      <div className="flex flex-wrap gap-4">
        <div className="flex flex-col gap-1">
          <Label htmlFor="mult-xp">{t("hsrv_mult_xp")}</Label>
          <Input
            id="mult-xp"
            className="w-28"
            inputMode="decimal"
            value={xp}
            aria-invalid={parsed.xp === null}
            onChange={(event) => setXp(event.target.value)}
          />
        </div>
        <div className="flex flex-col gap-1">
          <Label htmlFor="mult-rewards">{t("hsrv_mult_rewards")}</Label>
          <Input
            id="mult-rewards"
            className="w-28"
            inputMode="decimal"
            value={rewards}
            aria-invalid={parsed.rewards === null}
            onChange={(event) => setRewards(event.target.value)}
          />
        </div>
      </div>
      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          disabled={values === null}
          onClick={() => values && act(() => api.hostSetMultipliers(values))}
        >
          {t("hsrv_mult_apply")}
        </Button>
        {current && (
          <Button
            size="sm"
            variant="outline"
            onClick={() => act(() => api.hostSetMultipliers(null))}
          >
            {t("hsrv_mult_off")}
          </Button>
        )}
      </div>
    </Panel>
  )
}

function UpdatePanel({
  kit,
  server,
}: {
  kit: HostKit
  server: HostServerView
}) {
  const { t } = useI18n()
  const act = useAct()
  const update = kit.update
  const stopped = server.state === "stopped" || server.state === "failed"
  return (
    <Panel title={t("hsrv_update_title")} text={t("hsrv_update_text")}>
      <div>
        <Button
          variant="outline"
          size="sm"
          disabled={!stopped || kit.job?.state === "running"}
          onClick={() => act(() => api.hostUpdate())}
        >
          <RefreshCwIcon />
          {t("hsrv_update_button")}
        </Button>
      </div>
      {update && (
        <p className="text-sm" role="status">
          {update.state === "up_to_date"
            ? t("hsrv_update_uptodate", { head: update.head })
            : t("hsrv_update_updated", {
                head: update.head,
                files: update.files,
              })}
        </p>
      )}
      {update?.runtimeChanged && (
        <p className="text-sm text-amber-600 dark:text-amber-400">
          {t("hsrv_update_runtime")}
        </p>
      )}
    </Panel>
  )
}

function Console({ server }: { server: HostServerView }) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  return (
    <Panel title={t("hsrv_console")}>
      <div>
        <Button variant="outline" size="sm" onClick={() => setOpen(!open)}>
          {t(open ? "hsrv_console_hide" : "hsrv_console_show")}
        </Button>
      </div>
      {open &&
        (server.lines.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t("hsrv_console_empty")}
          </p>
        ) : (
          <pre className="max-h-80 overflow-auto bg-muted p-2 font-mono text-xs whitespace-pre-wrap">
            {server.lines.join("\n")}
          </pre>
        ))}
    </Panel>
  )
}

function ServerTab({
  kit,
  server,
  snap,
  setTab,
}: {
  kit: HostKit
  server: HostServerView
  snap: Snapshot
  setTab(tab: HostTab): void
}) {
  return (
    <>
      <ServerPanel kit={kit} server={server} setTab={setTab} />
      <Workspace kit={kit} snap={snap} />
      <SetupPanel kit={kit} server={server} snap={snap} />
      <InvitePanel kit={kit} server={server} />
      <Multipliers kit={kit} />
      <UpdatePanel kit={kit} server={server} />
      <Console server={server} />
    </>
  )
}

// The server's components as the controller reports them (the server GUI runs it inside this window)
function StatusTab({ server }: { server: HostServerView }) {
  const { t } = useI18n()
  const act = useAct()
  const [modules, setModules] = useState<ModuleView[]>([])
  const seen = server.components.map((c) => `${c.name}:${c.phase}`).join(",")
  useEffect(() => {
    let current = true
    void api.moduleList().then((out) => {
      if (current) setModules(out.ok ? out.modules : [])
    })
    return () => {
      current = false
    }
  }, [seen])
  const rows = statusRows(server.components, modules)
  const { ready, total } = readyCount(server.components)
  return (
    <Panel title={t("hstat_title")} text={t("hstat_text")}>
      {rows.length > 0 && (
        <p className="text-sm font-medium" role="status">
          {t("hstat_summary", { ready, total })}
        </p>
      )}
      {rows.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {t(
            server.state === "external" ? "hstat_not_embedded" : "hstat_empty"
          )}
        </p>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full text-left text-sm">
            <thead>
              <tr className="border-b text-xs tracking-widest text-muted-foreground uppercase">
                <th className="py-2 pr-4 font-semibold">{t("hstat_name")}</th>
                <th className="py-2 pr-4 font-semibold">{t("hstat_phase")}</th>
                <th className="py-2 pr-4 font-semibold">{t("hstat_port")}</th>
                <th className="py-2 pr-4 font-semibold">{t("hstat_detail")}</th>
                <th className="py-2" />
              </tr>
            </thead>
            <tbody>
              {rows.map(({ component, module }) => (
                <tr
                  key={component.name}
                  data-component={component.name}
                  className="border-b last:border-b-0"
                >
                  <th scope="row" className="py-2 pr-4 font-mono font-normal">
                    {component.name}
                    {module && (
                      <Badge variant="secondary" className="ml-2">
                        {t("hstat_module")}
                      </Badge>
                    )}
                  </th>
                  <td className="py-2 pr-4">
                    <Badge
                      variant="outline"
                      className={toneClass(phaseTone(component.phase))}
                    >
                      {t(PHASE_KEYS[component.phase])}
                    </Badge>
                  </td>
                  <td className="py-2 pr-4 font-mono">
                    {component.port ?? ""}
                  </td>
                  <td className="max-w-80 py-2 pr-4 font-mono text-xs break-all text-muted-foreground">
                    {component.detail}
                  </td>
                  <td className="py-2 text-right">
                    {!module && component.phase !== "disabled" && (
                      <Button
                        variant="outline"
                        size="xs"
                        aria-label={t("hstat_restart_label", {
                          name: component.name,
                        })}
                        onClick={() =>
                          act(() => api.hostServerRestart(component.name))
                        }
                      >
                        <RefreshCwIcon />
                        {t("hstat_restart")}
                      </Button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </Panel>
  )
}

export function HostScreen() {
  const { t } = useI18n()
  const { snap } = useLauncher()
  const { kit, server } = useHost()
  const [tab, setTab] = useState<HostTab>("server")
  // A plugin's look can be previewed on this page (the theme slot sets its colours here)
  const scope = useRef<HTMLDivElement>(null)
  useEffect(() => startHostStore(), [])
  if (!snap) return null
  const tabs = hostTabs(snap)
  let body: ReactNode
  if (kit === null || server === null) {
    body = (
      <div className="flex items-center gap-3 text-sm text-muted-foreground">
        <Spinner />
      </div>
    )
  } else {
    body = (
      <>
        {snap.app.platform !== "win32" && (
          <p className="text-sm text-muted-foreground">
            {t("host_windows_only")}
          </p>
        )}
        <JobBar kit={kit} />
        <Tabs value={tab} onValueChange={(next) => setTab(next as HostTab)}>
          <TabsList aria-label={t("host_title")} className="flex-wrap">
            {tabs.map((id) => (
              <TabsTrigger key={id} value={id}>
                {t(HOST_TAB_KEYS[id])}
              </TabsTrigger>
            ))}
          </TabsList>
          <TabsContent value="server" className="flex flex-col gap-6 pt-4">
            <ServerTab kit={kit} server={server} snap={snap} setTab={setTab} />
          </TabsContent>
          <TabsContent value="status" className="flex flex-col gap-6 pt-4">
            <StatusTab server={server} />
          </TabsContent>
          <TabsContent value="modules" className="flex flex-col gap-6 pt-4">
            <ModulesTab scope={scope} />
          </TabsContent>
          <TabsContent value="gamedata" className="flex flex-col gap-6 pt-4">
            <GameDataTab kit={kit} snap={snap} />
          </TabsContent>
          <TabsContent value="build" className="flex flex-col gap-6 pt-4">
            <BuildTab kit={kit} snap={snap} />
          </TabsContent>
          <TabsContent
            value="requirements"
            className="flex flex-col gap-6 pt-4"
          >
            <RequirementsTab kit={kit} />
          </TabsContent>
        </Tabs>
      </>
    )
  }
  return (
    <div
      ref={scope}
      data-host-root=""
      className="flex max-w-4xl flex-col gap-6 bg-background text-foreground"
    >
      <Heading title={t("host_title")} text={t("host_intro")} />
      {body}
    </div>
  )
}
