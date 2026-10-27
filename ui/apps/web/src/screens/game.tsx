// GAME: getting Dauntless from the player's own Epic library (DD22, DE7): the Epic account, the library check,
// the versions that can be installed and the progress of the one being installed. The state is use-epic.ts's;
// what each row shows and offers is epic.ts's.
import { useState } from "react"
import {
  CheckIcon,
  CopyIcon,
  DownloadIcon,
  FolderIcon,
  LogInIcon,
  LogOutIcon,
  PauseIcon,
  PlayIcon,
  RefreshCwIcon,
  SquareIcon,
  TriangleAlertIcon,
} from "lucide-react"

import {
  api,
  type EpicLibraryCheck,
  type GameVersionEntry,
} from "@workspace/bridge"
import { Badge } from "@workspace/ui/components/badge"
import { Button } from "@workspace/ui/components/button"
import { Input } from "@workspace/ui/components/input"
import { Label } from "@workspace/ui/components/label"
import { Progress, ProgressLabel } from "@workspace/ui/components/progress"

import {
  ACTION_KEYS,
  EPIC_LOGIN_URL,
  installProgress,
  sameFolder,
  versionRow,
  versionSizes,
  type VersionAction,
} from "@/app/epic"
import { formatBytes, formatDateTime, formatPercentText } from "@/app/format"
import { useI18n } from "@/app/i18n"
import { useLauncher } from "@/app/launcher"
import { errorMessage, taskView } from "@/app/model"
import { useEpic, type EpicState } from "@/app/use-epic"
import { Heading, Panel, Row, Spinner } from "@/components/parts"
import { Secret } from "@/components/secret"

const ACTION_ICONS: Record<VersionAction, typeof PlayIcon> = {
  install: DownloadIcon,
  continue: PlayIcon,
  retry: RefreshCwIcon,
  pause: PauseIcon,
  resume: PlayIcon,
  cancel: SquareIcon,
  use: CheckIcon,
}

// The sign-in window is the way in; this is for when it can't be used: the code Epic shows after signing in at
// its own page, in any browser
function PasteCode({ epic }: { epic: EpicState }) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  const [code, setCode] = useState("")
  const [copied, setCopied] = useState(false)
  const submit = async () => {
    if (!code.trim()) return
    if (await epic.submitCode(code.trim())) {
      setCode("")
      setOpen(false)
    }
  }
  return (
    <div className="flex flex-col gap-3">
      <div>
        <Button
          variant="link"
          size="xs"
          className="px-0"
          onClick={() => setOpen(!open)}
        >
          {t(open ? "gm_paste_hide" : "gm_paste_show")}
        </Button>
      </div>
      {open && (
        <div className="flex flex-col gap-3">
          <p className="text-sm text-muted-foreground">{t("gm_paste_text")}</p>
          <div className="flex flex-wrap items-center gap-2">
            <Input
              readOnly
              aria-label={t("gm_paste_address")}
              className="min-w-0 flex-1 font-mono text-xs"
              value={EPIC_LOGIN_URL}
              onFocus={(e) => e.currentTarget.select()}
            />
            <Button
              variant="outline"
              onClick={() =>
                void api.copyText(EPIC_LOGIN_URL).then((ok) => setCopied(ok))
              }
            >
              <CopyIcon />
              {t(copied ? "gm_paste_copied" : "gm_paste_copy")}
            </Button>
          </div>
          <Label htmlFor="epic-code">{t("gm_paste_label")}</Label>
          <Input
            id="epic-code"
            maxLength={1024}
            spellCheck={false}
            autoComplete="off"
            placeholder={t("gm_paste_placeholder")}
            value={code}
            onChange={(e) => setCode(e.target.value)}
          />
          <div>
            <Button
              disabled={epic.working !== null || !code.trim()}
              onClick={() => void submit()}
            >
              <CheckIcon />
              {t("gm_paste_use")}
            </Button>
          </div>
          <p className="text-xs text-muted-foreground">{t("gm_paste_note")}</p>
        </div>
      )}
    </div>
  )
}

function LibraryResult({
  library,
  checking,
}: {
  library: EpicLibraryCheck | null
  checking: boolean
}) {
  const { t, msg, lang } = useI18n()
  const { snap } = useLauncher()
  if (checking || library === null) {
    return (
      <p className="flex items-center gap-2 text-sm text-muted-foreground">
        <Spinner />
        {t("gm_library_checking")}
      </p>
    )
  }
  if (!library.ok) {
    return (
      <p className="text-sm text-destructive" data-code={library.error.code}>
        {msg(errorMessage(library.error, snap?.server?.mode ?? null))}
      </p>
    )
  }
  if (library.owned) {
    return (
      <p className="flex items-center gap-2 text-sm">
        <CheckIcon className="size-4 text-primary" />
        {t("gm_library_owned", {
          time: formatDateTime(library.checkedAt, lang),
        })}
      </p>
    )
  }
  return (
    <div className="flex flex-col gap-1" data-owned="false">
      <p className="flex items-center gap-2 text-sm font-medium">
        <TriangleAlertIcon className="size-4 text-amber-500" />
        {t("gm_library_not_owned_title")}
      </p>
      <p className="text-sm text-muted-foreground">
        {t("gm_library_not_owned_text")}
      </p>
    </div>
  )
}

function Account({ epic }: { epic: EpicState }) {
  const { t } = useI18n()
  const { account, working } = epic
  const signingIn = account?.signingIn === true || working === "sign_in"
  return (
    <Panel
      title={t("gm_account_title")}
      icon={<LogInIcon className="size-4" />}
    >
      {account === null ? (
        <p className="flex items-center gap-2 text-sm text-muted-foreground">
          <Spinner />
          {t("btn_wait")}
        </p>
      ) : account.signedIn ? (
        <>
          <Row
            title={
              account.displayName
                ? t("gm_signed_in", { name: account.displayName })
                : t("gm_signed_in_unnamed")
            }
          >
            <Button
              variant="outline"
              disabled={working !== null}
              onClick={() => void epic.checkLibrary()}
            >
              <RefreshCwIcon />
              {t("gm_library_check")}
            </Button>
            <Button
              variant="outline"
              disabled={working !== null}
              onClick={() => void epic.signOut()}
            >
              <LogOutIcon />
              {t("gm_sign_out")}
            </Button>
          </Row>
          <LibraryResult
            library={epic.library}
            checking={working === "library"}
          />
        </>
      ) : (
        <>
          <p className="text-sm text-muted-foreground">{t("gm_signed_out")}</p>
          <div className="flex flex-wrap items-center gap-3">
            <Button
              disabled={signingIn || working !== null}
              onClick={() => void epic.signIn()}
            >
              <LogInIcon />
              {t("gm_sign_in")}
            </Button>
            {signingIn && (
              <span className="flex items-center gap-2 text-sm text-muted-foreground">
                <Spinner />
                {t("gm_signing_in")}
              </span>
            )}
          </div>
          <PasteCode epic={epic} />
        </>
      )}
    </Panel>
  )
}

function VersionCard({
  entry,
  epic,
  otherRunning,
}: {
  entry: GameVersionEntry
  epic: EpicState
  otherRunning: boolean
}) {
  const { t, msg, lang } = useI18n()
  const { snap, installTask } = useLauncher()
  const info = epic.installs[entry.version] ?? null
  const inUse = sameFolder(snap?.game.root ?? null, info?.folder ?? null)
  const row = versionRow(entry, info, {
    account: epic.account,
    library: epic.library,
    otherRunning,
    inUse,
  })
  const progress = installProgress(info, installTask)
  const view = progress ? taskView(progress, lang) : null
  const sizes = versionSizes(entry)
  const busy = epic.working !== null
  return (
    <div
      className="flex flex-col gap-3 border-b py-4 first:pt-0 last:border-b-0 last:pb-0"
      data-version={entry.version}
      data-state={info?.state ?? "unknown"}
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-medium">{entry.displayName}</span>
            <Badge variant="secondary">{entry.version}</Badge>
            <Badge variant="outline">
              {t(
                entry.origin === "server"
                  ? "gm_origin_server"
                  : "gm_origin_launcher"
              )}
            </Badge>
            {inUse && info?.state === "installed" && (
              <Badge>{t("gm_in_use")}</Badge>
            )}
          </div>
          <span className="text-xs text-muted-foreground">
            {sizes
              ? t("gm_sizes", {
                  download: formatBytes(sizes.download, lang),
                  install: formatBytes(sizes.install, lang),
                })
              : entry.epic
                ? t("gm_sizes_unknown")
                : null}
          </span>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {row.buttons.map(({ action, disabled }) => {
            const Icon = ACTION_ICONS[action]
            return (
              <Button
                key={action}
                data-action={action}
                variant={
                  action === "install" ||
                  action === "continue" ||
                  action === "use"
                    ? "default"
                    : "outline"
                }
                disabled={disabled || busy}
                onClick={() => void epic.act(entry.version, action)}
              >
                <Icon />
                {t(ACTION_KEYS[action])}
              </Button>
            )
          })}
        </div>
      </div>
      <div className="flex flex-col gap-1 text-sm">
        <span>{msg(row.status)}</span>
        {row.status.detail && (
          <Secret
            text={row.status.detail}
            className="font-mono text-xs break-all text-muted-foreground"
          />
        )}
        {row.hint && (
          <span className="text-xs text-muted-foreground">{t(row.hint)}</span>
        )}
        {row.error && (
          <span className="text-destructive" data-code={row.error.code}>
            {msg(errorMessage(row.error, snap?.server?.mode ?? null))}
          </span>
        )}
      </div>
      {row.running &&
        (progress && view ? (
          <Progress value={view.percent} aria-label={t(view.key)}>
            <ProgressLabel>{t(view.key)}</ProgressLabel>
            <span className="ml-auto text-sm text-muted-foreground tabular-nums">
              {formatPercentText(view.percent, lang)}
            </span>
            <span className="order-last flex w-full flex-wrap gap-3 text-xs text-muted-foreground">
              {view.bytes && <span>{view.bytes}</span>}
              {view.speed && <span>{view.speed}</span>}
              {view.eta && <span>{t("task_eta", { time: view.eta })}</span>}
              <span>
                {t("task_files", {
                  done: progress.filesDone,
                  total: progress.filesTotal,
                })}
              </span>
            </span>
          </Progress>
        ) : (
          <span className="flex items-center gap-2 text-xs text-muted-foreground">
            <Spinner />
            {t("gm_state_preparing")}
          </span>
        ))}
    </div>
  )
}

function Versions({ epic }: { epic: EpicState }) {
  const { t, msg } = useI18n()
  const { snap } = useLauncher()
  const { list } = epic
  const entries = list?.ok ? list.versions : []
  const running = (version: string) => {
    const state = epic.installs[version]?.state
    return (
      state === "preparing" ||
      state === "downloading" ||
      state === "paused" ||
      state === "finishing"
    )
  }
  return (
    <Panel
      title={t("gm_versions_title")}
      text={t("gm_versions_text")}
      icon={<DownloadIcon className="size-4" />}
    >
      <div className="flex flex-wrap items-center gap-3">
        <Button
          variant="outline"
          disabled={epic.working !== null || list === null}
          onClick={() => void epic.refreshList()}
        >
          <RefreshCwIcon />
          {t("gm_versions_refresh")}
        </Button>
        {(list === null || epic.working === "list") && (
          <span className="flex items-center gap-2 text-sm text-muted-foreground">
            <Spinner />
            {t("gm_versions_loading")}
          </span>
        )}
      </div>
      {list && !list.ok && (
        <p className="text-sm text-destructive" data-code={list.error.code}>
          {msg(errorMessage(list.error, snap?.server?.mode ?? null))}
        </p>
      )}
      {list?.ok && entries.length === 0 && (
        <p className="text-sm text-muted-foreground">
          {t("gm_versions_empty")}
        </p>
      )}
      {entries.length > 0 && (
        <div className="flex flex-col">
          {entries.map((entry) => (
            <VersionCard
              key={entry.version}
              entry={entry}
              epic={epic}
              otherRunning={entries.some(
                (other) =>
                  other.version !== entry.version && running(other.version)
              )}
            />
          ))}
        </div>
      )}
    </Panel>
  )
}

export function GameScreen() {
  const { t } = useI18n()
  const epic = useEpic()
  return (
    <>
      <Heading title={t("gm_title")} text={t("gm_text")} />
      <Account epic={epic} />
      <Versions epic={epic} />
      <Panel
        title={t("gm_folder_title")}
        text={t("gm_folder_text")}
        icon={<FolderIcon className="size-4" />}
      />
    </>
  )
}
