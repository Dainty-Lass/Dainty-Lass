// PLAY: one screen per phase of the way to the game, from the invite to the running game. The action bar
// (action-bar.tsx) holds the button that moves it on.
import { useState, type ReactNode } from "react"
import {
  CheckIcon,
  DatabaseIcon,
  DownloadIcon,
  FolderIcon,
  InfoIcon,
  KeyIcon,
  ShieldIcon,
  TriangleAlertIcon,
} from "lucide-react"

import { api, type Snapshot } from "@workspace/bridge"
import { Badge } from "@workspace/ui/components/badge"
import { Button } from "@workspace/ui/components/button"
import { Input } from "@workspace/ui/components/input"
import { Label } from "@workspace/ui/components/label"

import { hostPageAvailable } from "@/app/host-model"
import { useI18n } from "@/app/i18n"
import { useLauncher } from "@/app/launcher"
import { realmsPageAvailable } from "@/app/realms"
import {
  checkUsername,
  errorMessage,
  graphicsKey,
  installingTitle,
  inviteSummary,
  locked,
  shortHex,
  sourceKey,
} from "@/app/model"
import { formatBytes, formatTime } from "@/app/format"
import {
  EditionBadge,
  ExternalLink,
  Heading,
  ModeBadge,
  Panel,
  Row,
  Spinner,
} from "@/components/parts"
import { Secret, useHidden } from "@/components/secret"
import { RuntimeFiles } from "@/components/runtime-files"
import { SavedServers } from "@/screens/servers"

function serverName(s: Snapshot): string {
  return s.status?.name ?? s.server?.name ?? ""
}

function Eyebrow({ snap }: { snap: Snapshot }) {
  const { t } = useI18n()
  if (!snap.server) return <span>{t("join_eyebrow")}</span>
  return (
    <>
      <span>{serverName(snap)}</span>
      <ModeBadge mode={snap.server.mode} />
      <EditionBadge game={snap.server.game} />
    </>
  )
}

function LeaveLink({ snap }: { snap: Snapshot }) {
  const { t } = useI18n()
  const { showModal } = useLauncher()
  return (
    <Button
      variant="link"
      size="xs"
      className="px-0"
      disabled={locked(snap)}
      onClick={() => showModal({ kind: "leave" })}
    >
      {t("connect_use_other")}
    </Button>
  )
}

function Join({ snap }: { snap: Snapshot }) {
  const { t, msg } = useI18n()
  const { inviteText, setInviteText, inviteCheck } = useLauncher()
  const hide = useHidden()
  const text = inviteText.trim()
  const invite = inviteCheck?.ok ? inviteSummary(text) : null
  let hint: ReactNode = t("join_hint")
  let tone = "text-muted-foreground"
  if (text && inviteCheck && !inviteCheck.ok) {
    hint = msg(errorMessage(inviteCheck.error, null))
    tone = "text-destructive"
  } else if (invite) {
    hint = t(invite.mode === "public" ? "join_ok_public" : "join_ok_private")
    tone = "text-emerald-500"
  }
  return (
    <>
      <Heading
        eyebrow={<span>{t("join_eyebrow")}</span>}
        title={t("join_title")}
        text={t("join_text")}
      />
      <SavedServers snap={snap} />
      <Panel>
        <div className="flex flex-col gap-2">
          <Label htmlFor="invite-input">{t("join_label")}</Label>
          <textarea
            id="invite-input"
            className="min-h-20 w-full resize-none border-b border-input bg-transparent py-2 font-mono text-sm outline-none focus-visible:border-b-ring"
            placeholder={t("join_placeholder")}
            spellCheck={false}
            autoComplete="off"
            maxLength={2048}
            value={inviteText}
            aria-invalid={Boolean(text && inviteCheck && !inviteCheck.ok)}
            aria-describedby="invite-hint"
            onChange={(e) => setInviteText(e.target.value)}
          />
          {invite && (
            <div className="flex flex-wrap items-center gap-3 text-sm">
              <CheckIcon className="size-4 text-emerald-500" />
              <span className="font-medium">{invite.name}</span>
              <Secret
                className="font-mono text-xs"
                text={
                  invite.port ? `${invite.host}:${invite.port}` : invite.host
                }
              />
              {invite.fp && (
                <span className="font-mono text-xs text-muted-foreground">
                  {t("link_fingerprint", { fp: hide(shortHex(invite.fp)) })}
                </span>
              )}
              <ModeBadge mode={invite.mode} />
            </div>
          )}
          <p id="invite-hint" aria-live="polite" className={`text-xs ${tone}`}>
            {hint}
          </p>
        </div>
        <p className="text-xs text-muted-foreground">{t("join_next")}</p>
      </Panel>
    </>
  )
}

function Connect({ snap }: { snap: Snapshot }) {
  const { t, lang } = useI18n()
  const hide = useHidden()
  const server = snap.server
  if (!server) return null
  const name = serverName(snap)
  const pub = server.mode === "public"
  const foot = (
    <div className="flex flex-wrap items-center gap-4 text-xs text-muted-foreground">
      {snap.connect.lastCheckedAt && (
        <span>
          {t("connect_last_checked", {
            time: formatTime(snap.connect.lastCheckedAt, lang),
          })}
        </span>
      )}
      <LeaveLink snap={snap} />
    </div>
  )
  if (
    snap.connect.checking ||
    (snap.connect.problem === null && snap.connect.lastCheckedAt === null)
  ) {
    return (
      <>
        <Heading
          eyebrow={<Eyebrow snap={snap} />}
          title={t("connect_checking_title", { name })}
          text={t(
            pub ? "connect_checking_text_public" : "connect_checking_text"
          )}
        />
        <Panel>
          <div className="flex items-center gap-3 text-sm">
            <Spinner />
            {t("connect_checking_wait")}
          </div>
        </Panel>
      </>
    )
  }
  switch (snap.connect.problem) {
    case "tailscale_missing":
      return (
        <>
          <Heading
            eyebrow={<Eyebrow snap={snap} />}
            title={t("connect_ts_title")}
            text={t("connect_ts_text", { name })}
          />
          <Panel>
            <ol className="flex list-decimal flex-col gap-3 pl-5 text-sm">
              <li>
                <div>{t("connect_step_install")}</div>
                <ExternalLink target="tailscale_download">
                  {t("connect_btn_install_ts")}
                </ExternalLink>
              </li>
              <li>
                <div>
                  {t(
                    server.hasShare
                      ? "connect_step_accept"
                      : "connect_step_accept_nolink"
                  )}
                </div>
                {server.hasShare && (
                  <ExternalLink target="tailscale_share">
                    {t("connect_btn_accept")}
                  </ExternalLink>
                )}
              </li>
              <li>{t("connect_step_retry")}</li>
            </ol>
          </Panel>
          {foot}
        </>
      )
    case "cert_mismatch":
      return (
        <>
          <Heading
            eyebrow={<Eyebrow snap={snap} />}
            title={t("connect_cert_title")}
          />
          <Panel
            tone="danger"
            icon={<ShieldIcon className="size-4" />}
            title={t("connect_cert_subtitle")}
          >
            <p className="text-sm">
              {t("connect_cert_text", { host: hide(server.host) })}
            </p>
            <p className="text-sm">{t("connect_cert_next")}</p>
          </Panel>
          {foot}
        </>
      )
    case "bad_answer":
      return (
        <>
          <Heading
            eyebrow={<Eyebrow snap={snap} />}
            title={t("connect_bad_title")}
            text={t("connect_bad_text", {
              host: hide(server.host),
              port: server.port,
            })}
          />
          {foot}
        </>
      )
    default:
      return (
        <>
          <Heading
            eyebrow={<Eyebrow snap={snap} />}
            title={t("connect_unreach_title", { name })}
            text={t(
              pub ? "connect_unreach_text_public" : "connect_unreach_text",
              { host: hide(server.host) }
            )}
          />
          <Panel>
            <ul className="flex list-disc flex-col gap-2 pl-5 text-sm">
              {(pub
                ? ([
                    "connect_check_internet",
                    "connect_check_server",
                    "connect_check_firewall",
                  ] as const)
                : ([
                    "connect_check_ts_on",
                    "connect_check_share",
                    "connect_check_server",
                  ] as const)
              ).map((key) => (
                <li key={key}>{t(key)}</li>
              ))}
            </ul>
          </Panel>
          {foot}
        </>
      )
  }
}

function Register({ snap }: { snap: Snapshot }) {
  const { t } = useI18n()
  const { username, setUsername, report } = useLauncher()
  const [keyOpen, setKeyOpen] = useState(false)
  const [keyText, setKeyText] = useState("")
  const name = serverName(snap)
  const check = checkUsername(username.trim())
  const submitKey = async () => {
    const result = await api.useExistingKey(keyText)
    if (result.ok) {
      setKeyText("")
      setKeyOpen(false)
    }
    await report(result)
  }
  return (
    <>
      <Heading
        eyebrow={<Eyebrow snap={snap} />}
        title={t("reg_title")}
        text={t("reg_text", { name })}
      />
      {snap.lastError?.code === "key_rejected" && (
        <Panel
          tone="warn"
          icon={<KeyIcon className="size-4" />}
          title={t("reg_rejected_title")}
          text={t("reg_rejected", { name })}
        />
      )}
      {snap.status?.registration === "NONE" && (
        <Panel
          tone="warn"
          icon={<TriangleAlertIcon className="size-4" />}
          title={t("reg_closed_title")}
          text={t("reg_closed")}
        />
      )}
      <Panel>
        <div className="flex flex-col gap-2">
          <Label htmlFor="username-input">{t("reg_label")}</Label>
          <Input
            id="username-input"
            maxLength={32}
            spellCheck={false}
            autoComplete="off"
            placeholder={t("reg_placeholder")}
            value={username}
            aria-invalid={check !== "ok" && username.length > 0}
            aria-describedby="username-hint"
            onChange={(e) => setUsername(e.target.value)}
          />
          <p
            id="username-hint"
            aria-live="polite"
            className={`text-xs ${check === "ok" ? "text-emerald-500" : username.length > 0 ? "text-destructive" : "text-muted-foreground"}`}
          >
            {t(`reg_check_${check}`)}
          </p>
        </div>
        <p className="flex items-center gap-2 text-xs text-muted-foreground">
          {snap.server?.hasPendingInvite ? (
            <CheckIcon className="size-3.5" />
          ) : (
            <InfoIcon className="size-3.5" />
          )}
          {t(
            snap.server?.hasPendingInvite
              ? "reg_invite_included"
              : "reg_invite_missing"
          )}
        </p>
      </Panel>
      <Panel>
        <div>
          <Button
            variant="link"
            size="xs"
            className="px-0"
            onClick={() => setKeyOpen(!keyOpen)}
          >
            <KeyIcon />
            {t(keyOpen ? "reg_have_key_hide" : "reg_have_key")}
          </Button>
        </div>
        {keyOpen && (
          <div className="flex flex-col gap-3">
            <Label htmlFor="key-input">{t("reg_key_label")}</Label>
            <Input
              id="key-input"
              type="password"
              maxLength={8192}
              spellCheck={false}
              autoComplete="off"
              placeholder={t("reg_key_placeholder")}
              value={keyText}
              onChange={(e) => setKeyText(e.target.value)}
            />
            <div className="flex flex-wrap gap-2">
              <Button
                disabled={snap.busy || keyText.trim() === ""}
                onClick={() => void submitKey()}
              >
                <KeyIcon />
                {t("reg_key_use")}
              </Button>
              <Button
                variant="outline"
                disabled={snap.busy}
                onClick={() => void api.importKeyFile().then(report)}
              >
                <FolderIcon />
                {t("reg_key_file")}
              </Button>
            </div>
            <p className="text-xs text-muted-foreground">{t("reg_key_note")}</p>
          </div>
        )}
      </Panel>
    </>
  )
}

function GamePathForm({ snap }: { snap: Snapshot }) {
  const { t } = useI18n()
  const { report } = useLauncher()
  const [open, setOpen] = useState(false)
  const [path, setPath] = useState("")
  const submitPath = async () => {
    if (!path.trim()) return
    const result = await api.useGamePath(path)
    if (result.ok) {
      setPath("")
      setOpen(false)
    }
    await report(result)
  }
  return (
    <Panel>
      <div className="flex flex-wrap items-center gap-3">
        <Button
          variant="outline"
          disabled={snap.busy}
          onClick={() => void api.chooseGameFolder().then(report)}
        >
          <FolderIcon />
          {t("locate_browse")}
        </Button>
        <Button
          variant="link"
          size="xs"
          disabled={snap.busy}
          onClick={() => setOpen(!open)}
        >
          {t(open ? "locate_path_hide" : "locate_path")}
        </Button>
      </div>
      {open && (
        <div className="flex flex-col gap-3">
          <Label htmlFor="game-path">{t("locate_path_label")}</Label>
          <Input
            id="game-path"
            maxLength={1024}
            spellCheck={false}
            autoComplete="off"
            placeholder={t("locate_path_placeholder")}
            value={path}
            onChange={(e) => setPath(e.target.value)}
          />
          <div>
            <Button
              disabled={snap.busy || !path.trim()}
              onClick={() => void submitPath()}
            >
              <CheckIcon />
              {t("locate_path_use")}
            </Button>
          </div>
          <p className="text-xs text-muted-foreground">
            {t("locate_path_note")}
          </p>
        </div>
      )}
    </Panel>
  )
}

// No Dauntless on this PC: the Game screen installs it from the player's own Epic library (DD22)
function FromEpic() {
  const { t } = useI18n()
  const { setView } = useLauncher()
  return (
    <Panel title={t("locate_epic_title")} text={t("locate_epic_text")}>
      <div>
        <Button variant="outline" onClick={() => setView("game")}>
          <DownloadIcon />
          {t("locate_epic_button")}
        </Button>
      </div>
    </Panel>
  )
}

function Detected({ snap }: { snap: Snapshot }) {
  const { t } = useI18n()
  const { report } = useLauncher()
  const list = snap.game.detected
  if (list.length === 0) return null
  const root = snap.game.root?.toLowerCase() ?? null
  return (
    <Panel title={t("locate_found_title")}>
      {list.map((d, i) => (
        <Row
          key={d.root}
          title={t(sourceKey(d.source))}
          sub={
            <>
              <Secret text={d.root} />
              {d.version && (
                <span className="block">
                  {t("locate_found_version", { version: d.version })}
                </span>
              )}
            </>
          }
        >
          {root === d.root.toLowerCase() ? (
            <Badge>
              <CheckIcon />
              {t("locate_in_use")}
            </Badge>
          ) : (
            <Button
              size="sm"
              disabled={snap.busy}
              onClick={() => void api.useDetectedGame(i).then(report)}
            >
              {t("locate_use")}
            </Button>
          )}
        </Row>
      ))}
    </Panel>
  )
}

// Dauntless 1.4.4, for a Dauntless Revived server in the legacy edition: where it goes, then download or check
function LocateRevived({ snap }: { snap: Snapshot }) {
  const { t, lang } = useI18n()
  const { report } = useLauncher()
  const r = snap.revived
  if (!r) return null
  const fresh = r.problem === "not_set"
  return (
    <>
      <Heading
        eyebrow={<Eyebrow snap={snap} />}
        title={t(fresh ? "revived_download_title" : "revived_check_title")}
        text={
          fresh
            ? t("revived_download_text", {
                size: formatBytes(r.totalBytes, lang),
              })
            : r.missingFiles > 0
              ? t("revived_check_missing", { n: r.missingFiles })
              : t("revived_check_text")
        }
      />
      <Panel>
        <Row
          title={t(fresh ? "revived_folder_target" : "revived_folder_current")}
          sub={<Secret text={r.root} />}
        >
          <Button
            variant="outline"
            disabled={snap.busy}
            onClick={() => void api.revivedChooseFolder().then(report)}
          >
            <FolderIcon />
            {t("revived_choose")}
          </Button>
        </Row>
        <p className="text-xs text-muted-foreground">
          {t("revived_choose_hint")}
        </p>
      </Panel>
      {r.detected.length > 0 && !r.detected.some((d) => d.root === r.root) && (
        <Panel
          tone="accent"
          icon={<CheckIcon className="size-4" />}
          title={t("revived_detected_title")}
        >
          {r.detected.map((d, i) => (
            <Row key={d.root} title={<Secret text={d.root} />}>
              <Button
                size="sm"
                disabled={snap.busy}
                onClick={() => void api.revivedUseDetected(i).then(report)}
              >
                {t("locate_use")}
              </Button>
            </Row>
          ))}
        </Panel>
      )}
      <Panel
        tone="warn"
        icon={<InfoIcon className="size-4" />}
        text={t("legacy_about")}
      />
    </>
  )
}

function Locate({ snap }: { snap: Snapshot }) {
  const { t } = useI18n()
  if (snap.revived) return <LocateRevived snap={snap} />
  const g = snap.game
  const guide = (
    <ExternalLink target="installing_guide">{t("locate_guide")}</ExternalLink>
  )
  let head: ReactNode
  let extra: ReactNode = null
  switch (g.problem) {
    case "wrong_version":
      head = (
        <Heading
          eyebrow={<Eyebrow snap={snap} />}
          title={t("locate_wrong_title")}
          text={t("locate_wrong_text")}
        />
      )
      extra = (
        <Panel
          tone="danger"
          icon={<TriangleAlertIcon className="size-4" />}
          title={t("locate_wrong_subtitle")}
        >
          <p className="text-sm">
            {g.versionText
              ? t("locate_wrong_found", { version: g.versionText })
              : t("locate_wrong_unknown")}
          </p>
          <p className="text-sm">{t("locate_wrong_fix")}</p>
          <div>{guide}</div>
        </Panel>
      )
      break
    case "unverified":
      head = (
        <Heading
          eyebrow={<Eyebrow snap={snap} />}
          title={t("locate_verify_title")}
          text={t("locate_verify_text")}
        />
      )
      break
    case "not_found":
      head = (
        <Heading
          eyebrow={<Eyebrow snap={snap} />}
          title={t("locate_missing_title")}
          text={t("locate_missing_text")}
        />
      )
      break
    default:
      head = (
        <Heading
          eyebrow={<Eyebrow snap={snap} />}
          title={t("locate_title")}
          text={t("locate_text")}
        />
      )
      if (g.detected.length === 0) {
        extra = (
          <Panel
            tone="warn"
            icon={<InfoIcon className="size-4" />}
            title={t("locate_none_title")}
            text={t("locate_none_text")}
          >
            <div className="flex gap-4">
              {guide}
              <ExternalLink target="legendary">
                {t("locate_legendary")}
              </ExternalLink>
            </div>
          </Panel>
        )
      }
  }
  return (
    <>
      {head}
      {extra}
      {g.root && (
        <Panel>
          <Row title={t("locate_current")} sub={<Secret text={g.root} />} />
        </Panel>
      )}
      <Detected snap={snap} />
      <GamePathForm snap={snap} />
      <FromEpic />
    </>
  )
}

function Runtime({ snap }: { snap: Snapshot }) {
  const { t } = useI18n()
  const { setView } = useLauncher()
  const name = serverName(snap)
  const r = snap.runtime
  const version = r.version ?? "?"
  switch (r.state) {
    case "untrusted":
      return (
        <>
          <Heading
            eyebrow={<Eyebrow snap={snap} />}
            title={t("runtime_trust_title")}
            text={t("runtime_trust_text", { name })}
          />
          <Panel
            tone="accent"
            icon={<ShieldIcon className="size-4" />}
            title={t("runtime_files_title", { version })}
            text={t("runtime_trust_detail")}
          >
            <RuntimeFiles files={r.files} />
          </Panel>
        </>
      )
    case "missing":
    case "outdated":
      return (
        <>
          <Heading
            eyebrow={<Eyebrow snap={snap} />}
            title={t(
              r.state === "missing"
                ? "runtime_install_title"
                : "runtime_update_title"
            )}
            text={t("runtime_install_text", { name })}
          />
          <Panel title={t("runtime_files_title", { version })}>
            <RuntimeFiles files={r.files} />
          </Panel>
        </>
      )
    default:
      // The Host page that builds a runtime is DU3's; the docs say how meanwhile
      return (
        <>
          <Heading
            eyebrow={<Eyebrow snap={snap} />}
            title={t("runtime_none_title")}
            text={t("runtime_none_text", { name })}
          />
          {snap.app.hostToolsAvailable && (
            <Panel
              tone="warn"
              title={t("runtime_none_host_title")}
              text={t("runtime_none_host_text")}
            >
              <div className="flex flex-wrap items-center gap-4">
                {hostPageAvailable(snap) && (
                  <Button size="sm" onClick={() => setView("host")}>
                    {t("host_teaser_open")}
                  </Button>
                )}
                <ExternalLink target="project_docs_sdk">
                  {t("host_teaser_docs")}
                </ExternalLink>
              </div>
            </Panel>
          )}
        </>
      )
  }
}

function Installing({ snap }: { snap: Snapshot }) {
  const { t } = useI18n()
  const { task } = useLauncher()
  const kind = task?.kind ?? snap.task?.kind ?? "verify"
  const { title, text } = installingTitle(snap, kind)
  return (
    <Heading
      eyebrow={<Eyebrow snap={snap} />}
      title={t(title)}
      text={t(text)}
    />
  )
}

function Ready({ snap }: { snap: Snapshot }) {
  const { t } = useI18n()
  const { setView, report, realm } = useLauncher()
  const user = snap.account.username
  const data = snap.status?.gameData
  const showRealms = realmsPageAvailable(snap, realm)
  return (
    <>
      <Heading
        eyebrow={<Eyebrow snap={snap} />}
        title={user ? t("ready_title", { user }) : t("ready_title_nouser")}
        text={t("ready_text")}
      />
      {(data === "synthetic" || data === "missing") && (
        <Panel
          tone="warn"
          icon={<DatabaseIcon className="size-4" />}
          title={t(
            data === "synthetic"
              ? "gamedata_synthetic_title"
              : "gamedata_missing_title"
          )}
          text={t(
            data === "synthetic"
              ? "gamedata_synthetic_text"
              : "gamedata_missing_text"
          )}
        />
      )}
      <Panel>
        <div className="flex flex-wrap items-center gap-4">
          <Badge variant="secondary">
            {t(snap.revived ? "ready_build_revived" : "ready_build")}
          </Badge>
          <Badge variant="secondary">
            {t("ready_graphics", {
              preset: { key: graphicsKey(snap.settings.graphics) },
            })}
          </Badge>
          <Badge variant="secondary">
            {t(snap.settings.windowed ? "ready_windowed" : "ready_fullscreen")}
          </Badge>
          {snap.runtime.state === "local" && (
            <Badge>{t("ready_local_runtime")}</Badge>
          )}
          <Button variant="link" size="xs" onClick={() => setView("settings")}>
            {t("ready_change")}
          </Button>
          {showRealms && (
            <Button variant="link" size="xs" onClick={() => setView("realms")}>
              {t("rlm_open_from_play")}
            </Button>
          )}
          {snap.revived && (
            <Button
              variant="link"
              size="xs"
              disabled={snap.busy}
              onClick={() => void api.revivedInstall("repair").then(report)}
            >
              {t("revived_repair")}
            </Button>
          )}
        </div>
      </Panel>
      {snap.account.offerBackup && (
        <Panel tone="accent" title={t("backup_title")} text={t("backup_text")}>
          <div className="flex flex-wrap gap-2">
            <Button onClick={() => void api.saveKeyBackup().then(report)}>
              <KeyIcon />
              {t("backup_save")}
            </Button>
            <Button
              variant="outline"
              onClick={() => void api.dismissBackupOffer()}
            >
              {t("backup_later")}
            </Button>
          </div>
        </Panel>
      )}
    </>
  )
}

function Running({ snap }: { snap: Snapshot }) {
  const { t } = useI18n()
  return (
    <>
      <Heading
        eyebrow={<Eyebrow snap={snap} />}
        title={t("running_title")}
        text={t("running_text")}
      />
      <Panel
        tone="accent"
        title={t("running_relay_title")}
        text={t("running_relay_text")}
      />
    </>
  )
}

export function PlayScreen() {
  const { t } = useI18n()
  const { snap } = useLauncher()
  if (!snap || snap.phase === "loading") {
    return (
      <div className="flex items-center gap-3 text-sm">
        <Spinner />
        {t("loading")}
      </div>
    )
  }
  const screens: Record<Exclude<Snapshot["phase"], "loading">, ReactNode> = {
    join: <Join snap={snap} />,
    connect: <Connect snap={snap} />,
    register: <Register snap={snap} />,
    locate: <Locate snap={snap} />,
    runtime: <Runtime snap={snap} />,
    installing: <Installing snap={snap} />,
    ready: <Ready snap={snap} />,
    running: <Running snap={snap} />,
  }
  return (
    <div className="flex max-w-3xl flex-col gap-6" data-phase={snap.phase}>
      {screens[snap.phase]}
    </div>
  )
}
