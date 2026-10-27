// The servers: the one joined now (its address, certificate, status, who is online), the others saved
// on this PC to switch to or remove, and a new one from an invite
import { useEffect, useState, type ReactNode } from "react"
import { PlusIcon, RefreshCwIcon, UsersIcon, XIcon } from "lucide-react"

import {
  api,
  type SavedServerInfo,
  type ServerStatus,
  type Snapshot,
} from "@workspace/bridge"
import { Button } from "@workspace/ui/components/button"

import { useI18n } from "@/app/i18n"
import { useLauncher } from "@/app/launcher"
import {
  groupedHex,
  instanceTitle,
  kindKey,
  locked,
  runtimeStateKey,
  whereKey,
} from "@/app/model"
import { formatDuration, runningMinutes } from "@/app/format"
import {
  EditionBadge,
  ExternalLink,
  Heading,
  ModeBadge,
  Panel,
  Row,
} from "@/components/parts"
import { Secret, useStreamer } from "@/components/secret"

function SavedSub({ server }: { server: SavedServerInfo }) {
  const { t } = useI18n()
  if (server.unsupported) return <span>{t("server_revived_disabled")}</span>
  return (
    <span className="flex flex-wrap items-center gap-2">
      {server.game === "revived" && <span>{t("edition_revived")}</span>}
      {server.username && (
        <span>{t("saved_account", { name: server.username })}</span>
      )}
      <Secret className="font-mono" text={`${server.host}:${server.port}`} />
    </span>
  )
}

// The other servers this PC has a key for: one click joins, Remove only takes it off the list
export function SavedServers({ snap }: { snap: Snapshot }) {
  const { t } = useI18n()
  const { switchTo, showModal } = useLauncher()
  const others = snap.savedServers.filter((sv) => !sv.current)
  if (others.length === 0) return null
  const busy = locked(snap)
  return (
    <Panel title={t("saved_title")} text={t("saved_text")}>
      <div>
        {others.map((sv) => (
          <Row
            key={sv.id}
            title={
              <span className="flex flex-wrap items-center gap-3">
                {sv.name}
                <ModeBadge mode={sv.mode} />
                <EditionBadge game={sv.game} />
              </span>
            }
            sub={<SavedSub server={sv} />}
          >
            <Button
              size="sm"
              disabled={busy || sv.unsupported}
              onClick={() => void switchTo(sv.id)}
            >
              {t("saved_join")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              disabled={busy}
              onClick={() =>
                showModal({ kind: "remove_server", id: sv.id, name: sv.name })
              }
            >
              <XIcon />
              {t("saved_remove")}
            </Button>
          </Row>
        ))}
      </div>
    </Panel>
  )
}

function Players({ status }: { status: ServerStatus }) {
  const { t, msg } = useI18n()
  const streamer = useStreamer()
  const byId = new Map(status.instances.map((i) => [i.id, i]))
  if (status.players.length === 0) {
    return <p className="text-sm text-muted-foreground">{t("sp_nobody")}</p>
  }
  return (
    <ul className="flex flex-col gap-2 text-sm">
      {status.players.map((p, index) => {
        const instance = p.instance ? byId.get(p.instance) : undefined
        return (
          <li key={`${p.name}-${index}`} className="flex flex-col">
            <span className={streamer ? "blur-xs select-none" : "font-medium"}>
              {streamer ? "••••••" : p.name}
            </span>
            <span className="flex gap-2 text-xs text-muted-foreground">
              <span>{t(whereKey(p.where))}</span>
              {instance?.kind === "hunt" && (
                <span>{msg(instanceTitle(instance))}</span>
              )}
            </span>
          </li>
        )
      })}
    </ul>
  )
}

// The clock the running times are read against, moved on twice a minute
function useNow(): number {
  const [now, setNow] = useState(Date.now)
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 30_000)
    return () => clearInterval(timer)
  }, [])
  return now
}

function Worlds({ status }: { status: ServerStatus }) {
  const { t, msg } = useI18n()
  const now = useNow()
  if (status.instances.length === 0) {
    return <p className="text-sm text-muted-foreground">{t("sp_no_worlds")}</p>
  }
  return (
    <ul className="flex flex-col divide-y text-sm">
      {status.instances.map((i) => {
        const minutes = runningMinutes(i.startedAt, now)
        return (
          <li key={i.id} className="flex flex-col gap-1 py-2">
            <span className="font-medium">{msg(instanceTitle(i))}</span>
            <span className="flex flex-wrap gap-3 text-xs text-muted-foreground">
              <span>{t(kindKey(i.kind))}</span>
              <span>
                {i.maxPlayers > 0
                  ? t("inst_players", { n: i.players, max: i.maxPlayers })
                  : t("inst_players_nomax", { n: i.players })}
              </span>
              {minutes !== null && (
                <span>
                  {t("inst_running", {
                    time:
                      minutes < 60
                        ? `${minutes} min`
                        : formatDuration(minutes * 60),
                  })}
                </span>
              )}
            </span>
          </li>
        )
      })}
    </ul>
  )
}

export function ServerScreen() {
  const { t } = useI18n()
  const { snap, showModal, setView } = useLauncher()
  if (!snap) return null
  const server = snap.server
  if (!server) {
    return (
      <div className="flex max-w-3xl flex-col gap-6">
        <Heading title={t("nav_server")} text={t("sp_nojoin")} />
        <SavedServers snap={snap} />
        <div>
          <Button onClick={() => setView("play")}>
            <PlusIcon />
            {t("set_add_server")}
          </Button>
        </div>
      </div>
    )
  }
  const status = snap.status
  const rows: [string, ReactNode][] = [
    [
      t("server_address"),
      <Secret className="font-mono" text={`${server.host}:${server.port}`} />,
    ],
    [
      t("server_connection"),
      t(
        server.mode === "public"
          ? "server_connection_public"
          : "server_connection_private"
      ),
    ],
  ]
  if (server.fingerprint) {
    rows.push([
      t("server_fingerprint"),
      <Secret className="font-mono" text={groupedHex(server.fingerprint)} />,
    ])
  }
  if (status) {
    rows.push([
      t("server_registration"),
      status.registration ? t(`reg_mode_${status.registration}`) : "–",
    ])
    rows.push([
      t("server_version"),
      [status.version, status.commit].filter(Boolean).join(" · ") || "–",
    ])
    rows.push([t("server_uptime"), formatDuration(status.uptimeSeconds)])
    if (status.game) {
      rows.push([
        t("server_game"),
        status.game.changelist
          ? `${status.game.build} (CL ${status.game.changelist})`
          : status.game.build,
      ])
    }
    rows.push([t("server_gamedata"), t(`gamedata_kind_${status.gameData}`)])
    rows.push([
      t("server_chat"),
      t(status.chat ? "server_chat_on" : "server_chat_off"),
    ])
  }
  rows.push([
    t("server_runtime"),
    <span className="flex gap-2">
      {snap.runtime.version && <span>{snap.runtime.version}</span>}
      <span>{t(runtimeStateKey(snap.runtime.state))}</span>
    </span>,
  ])
  const reachable = snap.phase !== "connect"
  const online = reachable && (status ? status.online : true)
  return (
    <div className="flex max-w-4xl flex-col gap-6">
      <Heading
        eyebrow={
          <>
            <span className={online ? "text-emerald-500" : "text-destructive"}>
              {t(online ? "sp_online" : "sp_offline")}
            </span>
            <ModeBadge mode={server.mode} />
            <EditionBadge game={server.game} />
          </>
        }
        title={status?.name ?? server.name}
        text={t("server_auto")}
      />
      <Panel>
        <dl className="grid grid-cols-[minmax(8rem,auto)_1fr] gap-x-6 gap-y-2 text-sm">
          {rows.map(([label, value]) => (
            <div key={label} className="contents">
              <dt className="text-muted-foreground">{label}</dt>
              <dd className="break-all">{value}</dd>
            </div>
          ))}
        </dl>
        <div className="flex flex-wrap items-center gap-4">
          <Button
            variant="outline"
            size="sm"
            onClick={() => void api.refreshStatus()}
          >
            <RefreshCwIcon />
            {t("sp_refresh")}
          </Button>
          {status?.sourceUrl && (
            <ExternalLink target="server_source">{t("sp_source")}</ExternalLink>
          )}
        </div>
      </Panel>
      {snap.phase === "connect" &&
      snap.connect.problem &&
      !snap.connect.checking ? (
        <Panel
          tone={snap.connect.problem === "cert_mismatch" ? "danger" : "warn"}
        >
          <p className="text-sm">
            {t(
              snap.connect.problem === "cert_mismatch"
                ? "connect_cert_subtitle"
                : "sp_no_answer"
            )}
          </p>
        </Panel>
      ) : !status ? (
        <Panel>
          <p className="text-sm text-muted-foreground">
            {t(
              reachable && snap.statusUnsupported
                ? "sp_unsupported"
                : "sp_waiting"
            )}
          </p>
        </Panel>
      ) : status.limited ? (
        <Panel
          icon={<UsersIcon className="size-4" />}
          title={t("server_players")}
          text={t("sp_signin")}
        />
      ) : (
        <div className="grid gap-6 md:grid-cols-2">
          <Panel
            icon={<UsersIcon className="size-4" />}
            title={t("server_players_count", { n: status.playersOnline })}
          >
            <Players status={status} />
          </Panel>
          <Panel title={t("server_worlds")}>
            <Worlds status={status} />
          </Panel>
        </div>
      )}
      <SavedServers snap={snap} />
      <div>
        <Button
          variant="outline"
          disabled={locked(snap)}
          onClick={() => showModal({ kind: "leave" })}
        >
          <PlusIcon />
          {t("set_add_server")}
        </Button>
      </div>
    </div>
  )
}
