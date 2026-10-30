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
  type AdminActionKind,
  type AdminResultView,
  type DoctorReportView,
  type GuardedActionInfo,
  type HistoryEntryView,
  type HostKit,
  type HostServerMode,
  type HostServerView,
  type ModuleView,
  type RevertDoneView,
  type RevertPlanView,
  type ServerConfigView,
  type ServerInviteEntry,
  type Snapshot,
} from "@workspace/bridge"
import {
  Alert,
  AlertDescription,
  AlertTitle,
} from "@workspace/ui/components/alert"
import { Badge } from "@workspace/ui/components/badge"
import { Button } from "@workspace/ui/components/button"
import { Input } from "@workspace/ui/components/input"
import { Label } from "@workspace/ui/components/label"
import { Switch } from "@workspace/ui/components/switch"
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
  snap,
  setTab,
}: {
  kit: HostKit
  server: HostServerView
  snap: Snapshot
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
        {snap.app.serverGui && kit.serverConfig.configured && (
          <Button
            variant="outline"
            disabled={working || changing}
            onClick={() => act(() => api.serverBackup())}
          >
            {t("hsrv_backup")}
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
  snap,
}: {
  kit: HostKit
  server: HostServerView
  snap: Snapshot
}) {
  const { t } = useI18n()
  const act = useAct()
  const [who, setWho] = useState("")
  const [copied, setCopied] = useState(false)
  const [copiedCode, setCopiedCode] = useState<string | null>(null)
  const [invites, setInvites] = useState<ServerInviteEntry[]>([])
  const [loadingInvites, setLoadingInvites] = useState(false)
  const running = server.state === "running"
  const invite = server.invite
  useEffect(() => {
    if (!copied) return
    const timer = setTimeout(() => setCopied(false), 2000)
    return () => clearTimeout(timer)
  }, [copied])
  useEffect(() => {
    if (!copiedCode) return
    const timer = setTimeout(() => setCopiedCode(null), 2000)
    return () => clearTimeout(timer)
  }, [copiedCode])
  const loadInvites = async () => {
    if (!snap.app.serverGui || !running) return
    setLoadingInvites(true)
    try {
      const out = await api.serverInviteList()
      if (out.ok) setInvites(out.invites)
    } finally {
      setLoadingInvites(false)
    }
  }
  useEffect(() => {
    if (!snap.app.serverGui || !running) return
    let current = true
    void api.serverInviteList().then((out) => {
      if (current && out.ok) setInvites(out.invites)
    })
    return () => {
      current = false
    }
  }, [snap.app.serverGui, running, invite?.link])
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
        {snap.app.serverGui && running && (
          <Button
            variant="outline"
            size="sm"
            disabled={loadingInvites}
            onClick={() => void loadInvites()}
          >
            {loadingInvites ? <Spinner /> : <RefreshCwIcon />}
            {t("hsrv_invite_list_refresh")}
          </Button>
        )}
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
      {snap.app.serverGui && running && (
        <div className="flex flex-col gap-2 border-t pt-3">
          <h4 className="text-xs font-semibold tracking-widest text-muted-foreground uppercase">
            {t("hsrv_invite_list_title")}
          </h4>
          {invites.length === 0 ? (
            <p className="text-xs text-muted-foreground">
              {t("hsrv_invite_list_empty")}
            </p>
          ) : (
            <ul className="flex flex-col gap-2">
              {invites.map((entry) => (
                <li
                  key={entry.code}
                  data-invite-code={entry.code}
                  className="flex flex-wrap items-center justify-between gap-2 border-b pb-2 text-xs last:border-b-0"
                >
                  <div className="flex min-w-0 flex-col gap-0.5">
                    <span className="font-mono font-medium">
                      <Secret text={entry.code} />
                      {entry.for && ` (${entry.for})`}
                    </span>
                    <span className="text-muted-foreground">
                      {t("hsrv_invite_uses", {
                        used: entry.infiniteUses ? "∞" : entry.usesRemaining,
                        max: entry.infiniteUses ? "∞" : entry.usesRemaining,
                      })}
                    </span>
                  </div>
                  <Button
                    variant="outline"
                    size="xs"
                    onClick={() =>
                      void api
                        .copyText(entry.link)
                        .then((ok) => ok && setCopiedCode(entry.code))
                    }
                  >
                    <CopyIcon />
                    {t(
                      copiedCode === entry.code
                        ? "hsrv_invite_copied"
                        : "hsrv_invite_copy"
                    )}
                  </Button>
                </li>
              ))}
            </ul>
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

function ServerConfigSection({
  kit,
  server,
}: {
  kit: HostKit
  server: HostServerView
}) {
  const { t } = useI18n()
  const act = useAct()
  const { report } = useLauncher()
  const [cfg, setCfg] = useState<ServerConfigView | null>(null)
  const [registration, setRegistration] = useState<
    "OPEN" | "INVITECODE" | "NONE"
  >("INVITECODE")
  const [database, setDatabase] = useState<"sqlite" | "postgres">("sqlite")
  const [maxHubs, setMaxHubs] = useState("1")
  const [maxHunts, setMaxHunts] = useState("3")
  const [freshStart, setFreshStart] = useState(false)
  const [confirmFresh, setConfirmFresh] = useState(false)
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(false)
  const working = kit.job?.state === "running"
  const stopped = server.state === "stopped" || server.state === "failed"

  const applyView = (view: ServerConfigView) => {
    setCfg(view)
    const reg =
      view.registration === "OPEN" || view.registration === "NONE"
        ? view.registration
        : "INVITECODE"
    setRegistration(reg)
    setDatabase(view.database === "postgres" ? "postgres" : "sqlite")
    setMaxHubs(String(view.maxHubs))
    setMaxHunts(String(view.maxHunts))
    setFreshStart(view.freshStart)
  }

  useEffect(() => {
    if (!kit.workspaceOk) return
    let current = true
    void api.serverConfigGet().then((out) => {
      if (current && out.ok) applyView(out.config)
    })
    return () => {
      current = false
    }
  }, [
    kit.workspace,
    kit.workspaceOk,
    kit.serverConfig.configured,
    kit.job?.state,
  ])

  if (!kit.workspaceOk || !cfg) return null
  const mig = cfg.mongoMigration
  const showMongoBanner =
    mig.state === "unmigrated" || mig.state === "unreachable"
  const hubsNum = Number(maxHubs)
  const huntsNum = Number(maxHunts)
  const countsValid =
    Number.isInteger(hubsNum) &&
    hubsNum >= 1 &&
    hubsNum <= 64 &&
    Number.isInteger(huntsNum) &&
    huntsNum >= 1 &&
    huntsNum <= 128

  const onSave = async () => {
    if (!countsValid) return
    setSaving(true)
    setSaved(false)
    try {
      const out = await api.serverConfigSave({
        registration,
        database,
        maxHubs: hubsNum,
        maxHunts: huntsNum,
        freshStart,
      })
      if (out.ok) {
        applyView(out.config)
        setSaved(true)
      } else {
        await report(out)
      }
    } finally {
      setSaving(false)
    }
  }

  const onConfirmFresh = async () => {
    setConfirmFresh(false)
    const out = await api.serverConfirmFreshStart()
    if (out.ok) applyView(out.config)
    else await report(out)
  }

  return (
    <>
      {showMongoBanner && (
        <Alert
          variant="destructive"
          data-mongo-migration={mig.state}
          className="flex flex-col gap-3"
        >
          <CircleAlertIcon />
          <AlertTitle>{t("hsrv_mongo_title")}</AlertTitle>
          <AlertDescription className="flex flex-col gap-3">
            <p>
              {mig.state === "unmigrated"
                ? t("hsrv_mongo_unmigrated", { source: mig.source ?? "" })
                : t("hsrv_mongo_unreachable", {
                    uri: mig.source ?? "",
                    detail: mig.detail ?? "",
                  })}
            </p>
            <div className="flex flex-wrap items-center gap-2">
              <Button
                size="sm"
                disabled={working || !stopped}
                onClick={() => act(() => api.serverMigrateMongo())}
              >
                {t("hsrv_mongo_migrate")}
              </Button>
              {!confirmFresh ? (
                <Button
                  size="sm"
                  variant="outline"
                  disabled={working || !stopped}
                  onClick={() => setConfirmFresh(true)}
                >
                  {t("hsrv_mongo_fresh")}
                </Button>
              ) : (
                <>
                  <Button
                    size="sm"
                    variant="destructive"
                    disabled={working || !stopped}
                    onClick={() => void onConfirmFresh()}
                  >
                    {t("hsrv_mongo_fresh_confirm")}
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => setConfirmFresh(false)}
                  >
                    {t("hsrv_mongo_fresh_cancel")}
                  </Button>
                </>
              )}
            </div>
          </AlertDescription>
        </Alert>
      )}
      {cfg.configured && (
        <Panel title={t("hsrv_cfg_title")} text={t("hsrv_cfg_text")}>
          <div className="flex flex-col gap-1">
            <Label htmlFor="cfg-registration">
              {t("hsrv_cfg_registration")}
            </Label>
            <Choice<"OPEN" | "INVITECODE" | "NONE">
              id="cfg-registration"
              value={registration}
              options={[
                { value: "INVITECODE", label: t("hsrv_cfg_reg_invite") },
                { value: "OPEN", label: t("hsrv_cfg_reg_open") },
                { value: "NONE", label: t("hsrv_cfg_reg_none") },
              ]}
              onChange={setRegistration}
            />
          </div>
          <div className="flex flex-col gap-1">
            <Label htmlFor="cfg-database">{t("hsrv_cfg_database")}</Label>
            <Choice<"sqlite" | "postgres">
              id="cfg-database"
              value={database}
              options={[
                { value: "sqlite", label: t("hsrv_cfg_db_sqlite") },
                { value: "postgres", label: t("hsrv_cfg_db_postgres") },
              ]}
              onChange={setDatabase}
            />
          </div>
          <div className="flex flex-wrap gap-4">
            <div className="flex flex-col gap-1">
              <Label htmlFor="cfg-max-hubs">{t("hsrv_cfg_max_hubs")}</Label>
              <Input
                id="cfg-max-hubs"
                className="w-28"
                inputMode="numeric"
                value={maxHubs}
                onChange={(event) => setMaxHubs(event.target.value)}
              />
            </div>
            <div className="flex flex-col gap-1">
              <Label htmlFor="cfg-max-hunts">{t("hsrv_cfg_max_hunts")}</Label>
              <Input
                id="cfg-max-hunts"
                className="w-28"
                inputMode="numeric"
                value={maxHunts}
                onChange={(event) => setMaxHunts(event.target.value)}
              />
            </div>
          </div>
          <div className="flex items-center gap-3">
            <Switch
              id="cfg-fresh-start"
              checked={freshStart}
              onCheckedChange={setFreshStart}
            />
            <Label htmlFor="cfg-fresh-start">{t("hsrv_cfg_fresh_start")}</Label>
          </div>
          <div className="flex flex-wrap items-center gap-3">
            <Button
              size="sm"
              disabled={saving || working || !countsValid}
              onClick={() => void onSave()}
            >
              {saving && <Spinner />}
              {t("hsrv_cfg_save")}
            </Button>
            {saved && (
              <span className="text-xs text-muted-foreground" role="status">
                {t("hsrv_cfg_saved")}
              </span>
            )}
          </div>
        </Panel>
      )}
    </>
  )
}

function DoctorPanel({ kit }: { kit: HostKit }) {
  const { t } = useI18n()
  const { report } = useLauncher()
  const [doc, setDoc] = useState<DoctorReportView | null>(null)
  const [running, setRunning] = useState(false)
  if (!kit.workspaceOk) return null
  const runDoctor = async () => {
    setRunning(true)
    try {
      const out = await api.serverDoctor()
      if (out.ok) setDoc(out.report)
      else await report(out)
    } finally {
      setRunning(false)
    }
  }
  return (
    <Panel title={t("hsrv_doc_title")} text={t("hsrv_doc_text")}>
      <div className="flex flex-wrap items-center gap-3">
        <Button
          variant="outline"
          size="sm"
          disabled={running}
          onClick={() => void runDoctor()}
        >
          {running ? <Spinner /> : <RefreshCwIcon />}
          {t("hsrv_doc_run")}
        </Button>
        {doc && (
          <span className="text-sm font-medium" role="status">
            {t("hsrv_doc_summary", {
              problems: doc.problems,
              warnings: doc.warnings,
            })}
          </span>
        )}
      </div>
      {doc && (
        <ul className="flex flex-col" data-doctor-checks="">
          {doc.checks.map((c, idx) => (
            <li
              key={`${c.label}-${idx}`}
              className="flex items-start gap-2 border-b py-1.5 text-xs last:border-b-0"
            >
              {c.level === "ok" ? (
                <CheckIcon className="mt-0.5 size-4 shrink-0 text-emerald-500" />
              ) : c.level === "warning" ? (
                <CircleAlertIcon className="mt-0.5 size-4 shrink-0 text-amber-500" />
              ) : (
                <XIcon className="mt-0.5 size-4 shrink-0 text-destructive" />
              )}
              <span className="w-28 shrink-0 font-mono text-muted-foreground">
                {c.label}
              </span>
              <span className="flex-1 break-all">{c.detail}</span>
            </li>
          ))}
        </ul>
      )}
    </Panel>
  )
}

function Console({ server }: { server: HostServerView }) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  const [filter, setFilter] = useState("all")
  const tags = Array.from(
    new Set([
      ...server.components.map((c) => c.name),
      ...server.lines
        .map((line) => /^\[([a-z0-9_-]+)\]/.exec(line)?.[1])
        .filter((v): v is string => Boolean(v)),
    ])
  )
  const filteredLines =
    filter === "all"
      ? server.lines
      : server.lines.filter((line) => line.startsWith(`[${filter}]`))
  return (
    <Panel title={t("hsrv_console")}>
      <div className="flex flex-wrap items-center gap-2">
        <Button variant="outline" size="sm" onClick={() => setOpen(!open)}>
          {t(open ? "hsrv_console_hide" : "hsrv_console_show")}
        </Button>
        {open && tags.length > 0 && (
          <div className="flex flex-wrap items-center gap-1">
            <Button
              variant={filter === "all" ? "secondary" : "ghost"}
              size="xs"
              onClick={() => setFilter("all")}
            >
              {t("hsrv_console_filter_all")}
            </Button>
            {tags.map((tag) => (
              <Button
                key={tag}
                variant={filter === tag ? "secondary" : "ghost"}
                size="xs"
                onClick={() => setFilter(tag)}
              >
                {tag}
              </Button>
            ))}
          </div>
        )}
      </div>
      {open &&
        (filteredLines.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t("hsrv_console_empty")}
          </p>
        ) : (
          <pre className="max-h-80 overflow-auto bg-muted p-2 font-mono text-xs whitespace-pre-wrap">
            {filteredLines.join("\n")}
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
      <ServerPanel kit={kit} server={server} snap={snap} setTab={setTab} />
      <Workspace kit={kit} snap={snap} />
      <SetupPanel kit={kit} server={server} snap={snap} />
      {snap.app.serverGui && <ServerConfigSection kit={kit} server={server} />}
      <InvitePanel kit={kit} server={server} snap={snap} />
      <Multipliers kit={kit} />
      {snap.app.serverGui && <DoctorPanel kit={kit} />}
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

function AdminTab({ server }: { server: HostServerView }) {
  const { t } = useI18n()
  const { report } = useLauncher()
  const [player, setPlayer] = useState("")
  const [reason, setReason] = useState("")
  const [until, setUntil] = useState("")
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<AdminResultView | null>(null)
  const running = server.state === "running"
  const canRun = running && !busy && player.trim() !== ""

  const runAdmin = async (action: AdminActionKind) => {
    if (!canRun) return
    setBusy(true)
    try {
      const out = await api.adminAction({
        action,
        player: player.trim(),
        reason: reason.trim() === "" ? null : reason.trim(),
        until: until.trim() === "" ? null : until.trim(),
        module: null,
      })
      if (out.ok) setResult(out.result)
      else await report(out)
    } finally {
      setBusy(false)
    }
  }

  return (
    <Panel title={t("hadm_title")} text={t("hadm_text")}>
      {!running && (
        <p className="text-sm text-muted-foreground">
          {t("err_server_stopped")}
        </p>
      )}
      <div className="flex flex-col gap-1">
        <Label htmlFor="admin-player">{t("hadm_player")}</Label>
        <Input
          id="admin-player"
          value={player}
          maxLength={64}
          autoComplete="off"
          onChange={(event) => setPlayer(event.target.value)}
        />
      </div>
      <div className="flex flex-wrap gap-4">
        <div className="flex min-w-56 flex-1 flex-col gap-1">
          <Label htmlFor="admin-reason">{t("hadm_reason")}</Label>
          <Input
            id="admin-reason"
            value={reason}
            maxLength={500}
            autoComplete="off"
            onChange={(event) => setReason(event.target.value)}
          />
        </div>
        <div className="flex min-w-56 flex-1 flex-col gap-1">
          <Label htmlFor="admin-until">{t("hadm_until")}</Label>
          <Input
            id="admin-until"
            value={until}
            maxLength={40}
            autoComplete="off"
            onChange={(event) => setUntil(event.target.value)}
          />
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          disabled={!canRun}
          onClick={() => void runAdmin("lookup")}
        >
          {t("hadm_lookup")}
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={!canRun}
          onClick={() => void runAdmin("kick")}
        >
          {t("hadm_kick")}
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={!canRun}
          onClick={() => void runAdmin("timeout")}
        >
          {t("hadm_timeout")}
        </Button>
        <Button
          size="sm"
          variant="destructive"
          disabled={!canRun}
          onClick={() => void runAdmin("ban")}
        >
          {t("hadm_ban")}
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={!canRun}
          onClick={() => void runAdmin("unban")}
        >
          {t("hadm_unban")}
        </Button>
      </div>
      {result && (
        <div
          className="flex flex-col gap-2 border-t pt-3 text-sm"
          data-admin-result={result.action}
          data-admin-ok={result.ok}
        >
          {result.message && <p className="font-medium">{result.message}</p>}
          {result.player && (
            <div
              className="flex flex-col gap-1 bg-muted p-3 text-xs"
              data-admin-player={result.player.displayName}
            >
              <div className="flex flex-wrap items-center gap-2">
                <span className="text-sm font-semibold">
                  {result.player.displayName}
                </span>
                <Badge
                  variant="outline"
                  className={toneClass(result.player.online ? "good" : "idle")}
                >
                  {t(result.player.online ? "hadm_online" : "hadm_offline")}
                </Badge>
                <Badge
                  variant="outline"
                  className={toneClass(result.player.banned ? "bad" : "good")}
                >
                  {t(result.player.banned ? "hadm_banned" : "hadm_active")}
                </Badge>
              </div>
              <span className="font-mono text-muted-foreground">
                ID: {result.player.globalId} ({result.player.moduleUserId})
              </span>
              {result.player.banReason && (
                <span>Reason: {result.player.banReason}</span>
              )}
              {result.player.timeoutUntil && (
                <span>Until: {result.player.timeoutUntil}</span>
              )}
            </div>
          )}
        </div>
      )}
    </Panel>
  )
}

function HistoryTab() {
  const { t } = useI18n()
  const { report } = useLauncher()
  const [account, setAccount] = useState("")
  const [actor, setActor] = useState("")
  const [since, setSince] = useState("")
  const [until, setUntil] = useState("")
  const [rows, setRows] = useState<HistoryEntryView[]>([])
  const [nextCursor, setNextCursor] = useState<string | null>(null)
  const [searched, setSearched] = useState(false)
  const [plan, setPlan] = useState<RevertPlanView | null>(null)
  const [applied, setApplied] = useState<RevertDoneView | null>(null)
  const [guarded, setGuarded] = useState<GuardedActionInfo[]>([])
  const [busy, setBusy] = useState(false)

  const loadGuarded = async () => {
    const out = await api.guardedList()
    if (out.ok) setGuarded(out.actions)
  }

  useEffect(() => {
    let current = true
    void api.guardedList().then((out) => {
      if (current && out.ok) setGuarded(out.actions)
    })
    return () => {
      current = false
    }
  }, [])

  const selection = () => ({
    account: account.trim() === "" ? null : account.trim(),
    actor: actor.trim() === "" ? null : actor.trim(),
    since: since.trim() === "" ? null : since.trim(),
    until: until.trim() === "" ? null : until.trim(),
  })

  const runSearch = async (cursor: string | null = null) => {
    setBusy(true)
    try {
      const out = await api.historyQuery({ ...selection(), cursor })
      if (out.ok) {
        setRows(cursor ? [...rows, ...out.page.entries] : out.page.entries)
        setNextCursor(out.page.next)
        setSearched(true)
      } else {
        await report(out)
      }
    } finally {
      setBusy(false)
    }
  }

  const runDryRun = async () => {
    setBusy(true)
    setApplied(null)
    try {
      const out = await api.historyRevertDryRun(selection())
      if (out.ok) setPlan(out.plan)
      else await report(out)
    } finally {
      setBusy(false)
    }
  }

  const runApply = async () => {
    if (!plan) return
    setBusy(true)
    try {
      const out = await api.historyRevertApply(plan.id)
      if (out.ok) {
        setApplied(out.done)
        setPlan(null)
        await runSearch(null)
      } else {
        await report(out)
      }
    } finally {
      setBusy(false)
    }
  }

  const decideGuarded = async (id: string, approve: boolean) => {
    const out = approve
      ? await api.guardedApprove(id)
      : await api.guardedCancel(id)
    if (out.ok) await loadGuarded()
    else await report(out)
  }

  return (
    <>
      <Panel title={t("hhis_title")} text={t("hhis_text")}>
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <div className="flex flex-col gap-1">
            <Label htmlFor="his-account">{t("hhis_account")}</Label>
            <Input
              id="his-account"
              value={account}
              maxLength={64}
              autoComplete="off"
              onChange={(event) => setAccount(event.target.value)}
            />
          </div>
          <div className="flex flex-col gap-1">
            <Label htmlFor="his-actor">{t("hhis_actor")}</Label>
            <Input
              id="his-actor"
              value={actor}
              maxLength={64}
              autoComplete="off"
              onChange={(event) => setActor(event.target.value)}
            />
          </div>
          <div className="flex flex-col gap-1">
            <Label htmlFor="his-since">{t("hhis_since")}</Label>
            <Input
              id="his-since"
              value={since}
              maxLength={40}
              autoComplete="off"
              onChange={(event) => setSince(event.target.value)}
            />
          </div>
          <div className="flex flex-col gap-1">
            <Label htmlFor="his-until">{t("hhis_until")}</Label>
            <Input
              id="his-until"
              value={until}
              maxLength={40}
              autoComplete="off"
              onChange={(event) => setUntil(event.target.value)}
            />
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            size="sm"
            disabled={busy}
            onClick={() => void runSearch(null)}
          >
            {t("hhis_search")}
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() => void runDryRun()}
          >
            {t("hhis_dry_run")}
          </Button>
          {plan && plan.actionCount > 0 && (
            <Button
              size="sm"
              variant="destructive"
              disabled={busy}
              onClick={() => void runApply()}
            >
              {t("hhis_apply", { count: plan.actionCount })}
            </Button>
          )}
        </div>
        {applied && (
          <p className="text-sm font-medium" role="status">
            {t("hhis_applied", {
              reverted: applied.rowsChanged,
              accounts: applied.databases.length,
            })}
          </p>
        )}
        {plan && (
          <div
            className="flex flex-col gap-2 bg-muted p-3 text-xs"
            data-revert-plan={plan.id}
          >
            <span className="font-semibold">
              Plan {plan.id}: {plan.actionCount} actions ({plan.entries} log
              entries, {plan.conflictCount} conflicts)
            </span>
            {plan.actions.map((a, idx) => (
              <div
                key={`${a.database}-${a.table}-${a.key}-${idx}`}
                className="font-mono"
              >
                {a.database}.{a.table} · {a.kind} · {a.key}
              </div>
            ))}
          </div>
        )}
        {searched &&
          (rows.length === 0 ? (
            <p className="text-sm text-muted-foreground">{t("hhis_empty")}</p>
          ) : (
            <div className="flex flex-col gap-2">
              <ul className="flex flex-col" data-history-rows="">
                {rows.map((r) => (
                  <li
                    key={r.id}
                    className="flex flex-wrap items-center justify-between gap-2 border-b py-1.5 text-xs last:border-b-0"
                  >
                    <span className="font-mono text-muted-foreground">
                      {r.at}
                    </span>
                    <span className="font-mono">{r.actor}</span>
                    <span className="font-medium">
                      {r.database}.{r.table} ({r.op})
                    </span>
                    <span className="flex-1 font-mono text-muted-foreground">
                      {r.key}
                    </span>
                  </li>
                ))}
              </ul>
              {nextCursor && (
                <div>
                  <Button
                    size="xs"
                    variant="outline"
                    disabled={busy}
                    onClick={() => void runSearch(nextCursor)}
                  >
                    {t("hhis_more")}
                  </Button>
                </div>
              )}
            </div>
          ))}
      </Panel>
      <Panel title={t("hhis_guarded_title")} text={t("hhis_guarded_text")}>
        {guarded.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t("hhis_guarded_empty")}
          </p>
        ) : (
          <ul className="flex flex-col gap-2">
            {guarded.map((g) => (
              <li
                key={g.id}
                className="flex flex-wrap items-center justify-between gap-2 border-b pb-2 text-xs last:border-b-0"
              >
                <div className="flex flex-col gap-0.5">
                  <span className="font-medium">
                    {g.kind}: {g.summary}
                  </span>
                  <span className="font-mono text-muted-foreground">
                    {g.id} · {g.initiatedBy} · expires {g.expiresAt}
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  {g.state === "pending" && g.mode === "approval" && (
                    <Button
                      size="xs"
                      onClick={() => void decideGuarded(g.id, true)}
                    >
                      {t("hhis_guarded_approve")}
                    </Button>
                  )}
                  <Button
                    size="xs"
                    variant="outline"
                    onClick={() => void decideGuarded(g.id, false)}
                  >
                    {t("hhis_guarded_cancel")}
                  </Button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </Panel>
    </>
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
          <TabsContent value="admin" className="flex flex-col gap-6 pt-4">
            <AdminTab server={server} />
          </TabsContent>
          <TabsContent value="history" className="flex flex-col gap-6 pt-4">
            <HistoryTab />
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
