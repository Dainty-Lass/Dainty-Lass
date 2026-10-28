// REALMS: hosting a hub or a hunt on this PC, joining a realm by session ID or from the server's open worlds,
// and watching the active session and local relay route (DU5, RF7, DD11, DD14). Shown only when realms are in
// the build and the game supports them.
import { useState } from "react"
import {
  CheckIcon,
  CopyIcon,
  GlobeIcon,
  LogInIcon,
  PlayIcon,
  RadioIcon,
  RefreshCwIcon,
  SearchIcon,
  SquareIcon,
  SwordsIcon,
  TriangleAlertIcon,
} from "lucide-react"

import { api, type RealmSessionView, type Snapshot } from "@workspace/bridge"
import { Badge } from "@workspace/ui/components/badge"
import { Button } from "@workspace/ui/components/button"
import { Input } from "@workspace/ui/components/input"
import { Label } from "@workspace/ui/components/label"

import { useI18n } from "@/app/i18n"
import { useLauncher } from "@/app/launcher"
import { errorMessage, kindKey, locked } from "@/app/model"
import {
  BROWSER_FILTER_KEYS,
  browserItems,
  checkRealmId,
  filterBrowserItems,
  HUB_TARGET_KEYS,
  REALM_ID_MAX,
  REALM_KIND_KEYS,
  REALM_STATE_KEYS,
  realmsPageAvailable,
  realmStateTone,
  relayStatusMessage,
  sessionRoleKey,
  type BrowserFilterKind,
  type HubTarget,
} from "@/app/realms"
import { useRealms, type RealmsState } from "@/app/use-realms"
import { Choice } from "@/components/choice"
import { Heading, Panel, Row, Spinner } from "@/components/parts"
import { Secret } from "@/components/secret"
import { toneClass } from "@/components/tone"

const HUB_TARGETS: HubTarget[] = ["ramsgate", "training"]
const FILTER_KINDS: BrowserFilterKind[] = ["all", "hub", "hunt"]

function SessionItem({
  session,
  gatewayRelayPort,
}: {
  session: RealmSessionView
  gatewayRelayPort: number | null
}) {
  const { t, msg } = useI18n()
  const [copied, setCopied] = useState(false)
  const tone = realmStateTone(session.state, session.result)
  const relay = relayStatusMessage(session, gatewayRelayPort)
  return (
    <div
      className="flex flex-col gap-2 border-b py-3 first:pt-0 last:border-b-0 last:pb-0"
      data-session={session.session}
      data-state={session.state}
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-medium">
            {t(sessionRoleKey(session))}
          </span>
          <Badge variant="secondary">{t(REALM_KIND_KEYS[session.kind])}</Badge>
          <Badge variant="outline" className={toneClass(tone)}>
            {session.state === "starting" && <Spinner />}
            {t(REALM_STATE_KEYS[session.state])}
          </Badge>
          <Badge variant="outline">
            {t("rlm_generation", { n: session.generation })}
          </Badge>
          {session.localPort !== null && (
            <Badge variant="outline">
              {t("rlm_local_port", { port: session.localPort })}
            </Badge>
          )}
        </div>
        <Button
          variant="outline"
          size="xs"
          onClick={() =>
            void api.copyText(session.session).then((ok) => setCopied(ok))
          }
        >
          <CopyIcon />
          {t(copied ? "rlm_copied_id" : "rlm_copy_id")}
        </Button>
      </div>
      <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        <span>{t("rlm_session_id")}:</span>
        <Secret text={session.session} className="font-mono break-all" />
      </div>
      <p className="text-xs text-muted-foreground">{msg(relay)}</p>
    </div>
  )
}

function StatusPanel({
  snap,
  state,
}: {
  snap: Snapshot | null
  state: RealmsState
}) {
  const { t, msg } = useI18n()
  const { outcome, realm, working } = state
  const busy = working !== null || (snap ? locked(snap) : false)
  const active = realm?.active ?? null
  const sessions = realm?.sessions ?? []
  const hasLive =
    sessions.some((s) => s.state !== "ended") ||
    (active !== null && active.state !== "ended")
  const gatewayPort = snap?.running.relayPort ?? null
  return (
    <Panel
      title={t("rlm_status_title")}
      text={t("rlm_status_text")}
      icon={<RadioIcon className="size-4" />}
      tone={active && active.state !== "ended" ? "accent" : "default"}
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        {realm ? (
          realm.available ? (
            <span
              className="flex items-center gap-2 text-sm"
              data-transport="ready"
            >
              <CheckIcon className="size-4 text-emerald-500" />
              {t("rlm_transport_ready")}
            </span>
          ) : (
            <span
              className="flex items-center gap-2 text-sm text-amber-500"
              data-transport="unavailable"
            >
              <TriangleAlertIcon className="size-4" />
              {t("rlm_transport_unavailable")}
            </span>
          )
        ) : (
          <span className="flex items-center gap-2 text-sm text-muted-foreground">
            <Spinner />
            {t("btn_wait")}
          </span>
        )}
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            data-action="refresh-realm"
            disabled={working !== null}
            onClick={() => void state.refresh()}
          >
            {working === "refresh" ? <Spinner /> : <RefreshCwIcon />}
            {t("rlm_refresh")}
          </Button>
          {(hasLive || sessions.length > 0) && (
            <Button
              variant="destructive"
              size="sm"
              data-action="stop-realm"
              disabled={busy}
              onClick={() => void state.stop()}
            >
              {working === "stop" ? <Spinner /> : <SquareIcon />}
              {t(active?.hosting === false ? "rlm_leave" : "rlm_stop")}
            </Button>
          )}
        </div>
      </div>
      {realm && !realm.available && (
        <p className="text-xs text-muted-foreground">
          {t("rlm_unavailable_hint")}
        </p>
      )}
      {outcome && !outcome.ok && (
        <p className="text-sm text-destructive" data-code={outcome.error.code}>
          {msg(errorMessage(outcome.error, snap?.server?.mode ?? null))}
        </p>
      )}
      {active ? (
        <div className="flex flex-col gap-2">
          <h3 className="text-xs font-semibold tracking-widest text-muted-foreground uppercase">
            {t("rlm_active_title")}
          </h3>
          <SessionItem session={active} gatewayRelayPort={gatewayPort} />
        </div>
      ) : (
        <p className="text-sm text-muted-foreground">{t("rlm_no_session")}</p>
      )}
      {sessions.length > 1 && (
        <div className="flex flex-col gap-2">
          <h3 className="text-xs font-semibold tracking-widest text-muted-foreground uppercase">
            {t("rlm_sessions_title", { count: sessions.length })}
          </h3>
          <div className="flex flex-col">
            {sessions.map((s) => (
              <SessionItem
                key={`${s.session}-${s.generation}`}
                session={s}
                gatewayRelayPort={gatewayPort}
              />
            ))}
          </div>
        </div>
      )}
    </Panel>
  )
}

function HostPanel({
  snap,
  state,
}: {
  snap: Snapshot | null
  state: RealmsState
}) {
  const { t } = useI18n()
  const [hubTarget, setHubTarget] = useState<HubTarget>("ramsgate")
  const available = Boolean(state.realm?.available)
  const busy = state.working !== null || (snap ? locked(snap) : false)
  const disabled = !available || busy
  return (
    <Panel
      title={t("rlm_host_title")}
      text={t("rlm_host_text")}
      icon={<GlobeIcon className="size-4" />}
    >
      <div>
        <Row title={t("rlm_hub_kind_label")}>
          <Choice<HubTarget>
            id="realm-hub-target"
            value={hubTarget}
            options={HUB_TARGETS.map((target) => ({
              value: target,
              label: t(HUB_TARGET_KEYS[target]),
            }))}
            onChange={setHubTarget}
          />
        </Row>
        <Row
          title={t("rlm_host_hub_title", {
            hub: t(HUB_TARGET_KEYS[hubTarget]),
          })}
          sub={t("rlm_host_hub_desc")}
        >
          <Button
            data-action="host-hub"
            disabled={disabled}
            onClick={() => void state.host("hub")}
          >
            {state.working === "host_hub" ? <Spinner /> : <PlayIcon />}
            {t("rlm_host_hub_btn")}
          </Button>
        </Row>
        <Row title={t("rlm_host_hunt_title")} sub={t("rlm_host_hunt_desc")}>
          <Button
            variant="outline"
            data-action="host-hunt"
            disabled={disabled}
            onClick={() => void state.host("hunt")}
          >
            {state.working === "host_hunt" ? <Spinner /> : <SwordsIcon />}
            {t("rlm_host_hunt_btn")}
          </Button>
        </Row>
      </div>
    </Panel>
  )
}

function JoinByIdPanel({
  snap,
  state,
}: {
  snap: Snapshot | null
  state: RealmsState
}) {
  const { t } = useI18n()
  const [text, setText] = useState("")
  const check = checkRealmId(text)
  const available = Boolean(state.realm?.available)
  const busy = state.working !== null || (snap ? locked(snap) : false)
  const canJoin = available && !busy && check === "ok"
  const submit = async () => {
    if (!canJoin) return
    if (await state.join(text)) setText("")
  }
  return (
    <Panel
      title={t("rlm_join_title")}
      text={t("rlm_join_text")}
      icon={<LogInIcon className="size-4" />}
    >
      <form
        className="flex flex-col gap-3"
        onSubmit={(event) => {
          event.preventDefault()
          void submit()
        }}
      >
        <div className="flex flex-col gap-2">
          <Label htmlFor="realm-join-input">{t("rlm_join_label")}</Label>
          <div className="flex flex-wrap items-center gap-2">
            <Input
              id="realm-join-input"
              className="min-w-0 flex-1 font-mono text-sm"
              maxLength={REALM_ID_MAX}
              spellCheck={false}
              autoComplete="off"
              placeholder={t("rlm_join_placeholder")}
              value={text}
              onChange={(event) => setText(event.target.value)}
            />
            <Button type="submit" data-action="join-id" disabled={!canJoin}>
              {state.working === "join" ? <Spinner /> : <LogInIcon />}
              {t("rlm_join_btn")}
            </Button>
          </div>
          {text.trim() !== "" && check === "bad_chars" && (
            <p className="text-xs text-destructive">
              {t("rlm_join_invalid_chars")}
            </p>
          )}
          {text.trim() !== "" && check === "too_long" && (
            <p className="text-xs text-destructive">
              {t("rlm_join_invalid_long")}
            </p>
          )}
        </div>
      </form>
    </Panel>
  )
}

function BrowserPanel({
  snap,
  state,
}: {
  snap: Snapshot | null
  state: RealmsState
}) {
  const { t, msg, lang } = useI18n()
  const [query, setQuery] = useState("")
  const [filterKind, setFilterKind] = useState<BrowserFilterKind>("all")
  const serverName = snap?.status?.name ?? snap?.server?.name ?? null
  const items = browserItems(snap?.status ?? null, state.realm)
  const filtered = filterBrowserItems(items, query, filterKind, lang)
  const busy = state.working !== null || (snap ? locked(snap) : false)
  return (
    <Panel
      title={t("rlm_browser_title")}
      text={
        serverName
          ? t("rlm_browser_text", { server: serverName })
          : t("rlm_browser_text_noserver")
      }
      icon={<SearchIcon className="size-4" />}
    >
      <div className="flex flex-wrap items-center gap-3">
        <Input
          id="realm-browser-search"
          aria-label={t("rlm_browser_search")}
          className="min-w-0 flex-1 text-sm"
          maxLength={128}
          placeholder={t("rlm_browser_search_placeholder")}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
        <div
          role="group"
          aria-label={t("rlm_browser_search")}
          className="flex flex-wrap gap-1"
        >
          {FILTER_KINDS.map((kind) => (
            <Button
              key={kind}
              size="sm"
              data-filter={kind}
              variant={filterKind === kind ? "default" : "outline"}
              aria-pressed={filterKind === kind}
              onClick={() => setFilterKind(kind)}
            >
              {t(BROWSER_FILTER_KEYS[kind])}
            </Button>
          ))}
        </div>
      </div>
      {items.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {t("rlm_browser_empty")}
        </p>
      ) : filtered.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {t("rlm_browser_no_match")}
        </p>
      ) : (
        <div className="flex flex-col">
          {filtered.map((item) => (
            <Row
              key={item.id}
              title={
                <span
                  className="flex flex-wrap items-center gap-2"
                  data-realm-id={item.id}
                >
                  <span>{msg(item.title)}</span>
                  <Badge variant="secondary">
                    {t(kindKey(item.instanceKind))}
                  </Badge>
                  <Badge
                    variant="outline"
                    className={toneClass(
                      item.joined ? "good" : item.full ? "idle" : "good"
                    )}
                  >
                    {t(
                      item.joined
                        ? "rlm_browser_joined"
                        : item.full
                          ? "rlm_browser_full"
                          : "rlm_browser_open"
                    )}
                  </Badge>
                </span>
              }
              sub={
                <span className="flex flex-wrap items-center gap-3">
                  {item.host && (
                    <span>{t("rlm_browser_host", { host: item.host })}</span>
                  )}
                  <span>
                    {item.maxPlayers > 0
                      ? t("inst_players", {
                          n: item.players,
                          max: item.maxPlayers,
                        })
                      : t("inst_players_nomax", { n: item.players })}
                  </span>
                  {item.map && <span>{item.map}</span>}
                  <Secret text={item.id} className="font-mono" />
                </span>
              }
            >
              <Button
                size="sm"
                data-action="join-browser"
                data-target={item.id}
                disabled={!item.joinable || busy}
                onClick={() => void state.join(item.id)}
              >
                <LogInIcon />
                {t(item.joined ? "rlm_browser_joined" : "rlm_join_btn")}
              </Button>
            </Row>
          ))}
        </div>
      )}
    </Panel>
  )
}

export function RealmsContent({
  snap,
  state,
}: {
  snap: Snapshot | null
  state: RealmsState
}) {
  const { t } = useI18n()
  if (!realmsPageAvailable(snap, state.outcome)) return null
  return (
    <div className="flex max-w-4xl flex-col gap-6" data-screen="realms">
      <Heading title={t("rlm_title")} text={t("rlm_text")} />
      <StatusPanel snap={snap} state={state} />
      <HostPanel snap={snap} state={state} />
      <JoinByIdPanel snap={snap} state={state} />
      <BrowserPanel snap={snap} state={state} />
    </div>
  )
}

export function RealmsScreen() {
  const { snap } = useLauncher()
  const state = useRealms()
  return <RealmsContent snap={snap} state={state} />
}
