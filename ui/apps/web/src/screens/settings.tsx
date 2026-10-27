// Settings: the game (edition, folder, check, language), graphics, the account and the servers, and the
// launcher itself. Each change goes to the backend, which answers with a new snapshot.
import type { ReactNode } from "react"
import {
  FolderIcon,
  KeyIcon,
  LogOutIcon,
  MonitorIcon,
  RefreshCwIcon,
  SettingsIcon,
  UserIcon,
  Gamepad2Icon,
  XIcon,
} from "lucide-react"

import {
  api,
  type Game,
  type GameLanguage,
  type GraphicsPreset,
  type RuntimeProfile,
  type Snapshot,
} from "@workspace/bridge"
import { LANGUAGE_NAMES, LANGUAGES } from "@workspace/i18n"
import { Button } from "@workspace/ui/components/button"
import { Switch } from "@workspace/ui/components/switch"
import {
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from "@workspace/ui/components/tabs"

import { useI18n } from "@/app/i18n"
import { useLauncher } from "@/app/launcher"
import { graphicsKey, locked } from "@/app/model"
import { Choice } from "@/components/choice"
import { ExternalLink, Heading, Panel, Row } from "@/components/parts"
import { Secret } from "@/components/secret"
import { SavedServers } from "@/screens/servers"

// The game's own text languages, each in its own words (a fixed list, not the launcher's languages)
const GAME_LANGUAGES: [Exclude<GameLanguage, "auto">, string][] = [
  ["en-US", "English"],
  ["de-DE", "Deutsch"],
  ["es-ES", "Español"],
  ["fr-FR", "Français"],
  ["it-IT", "Italiano"],
  ["ja-JP", "日本語"],
  ["pt-BR", "Português (Brasil)"],
  ["ru-RU", "Русский"],
]
const GRAPHICS: GraphicsPreset[] = [-1, 0, 1, 2, 3, 4]
const PROFILES: RuntimeProfile[] = ["production", "development"]
const EDITIONS: Game[] = ["reborn", "legacy"]

function Toggle({
  label,
  checked,
  onChange,
}: {
  label: string
  checked: boolean
  onChange(on: boolean): void
}) {
  return (
    <Switch
      aria-label={label}
      checked={checked}
      onCheckedChange={(on) => onChange(on)}
    />
  )
}

function GameTab({ snap }: { snap: Snapshot }) {
  const { t } = useI18n()
  const { report } = useLauncher()
  const busy = locked(snap)
  const legacy = snap.settings.gameEdition === "legacy"
  const revived = snap.revived
  let folder: ReactNode = null
  let check: ReactNode = null
  if (!legacy) {
    folder = (
      <Row
        title={t("set_folder")}
        sub={
          snap.game.root ? (
            <Secret text={snap.game.root} />
          ) : (
            t("set_folder_none")
          )
        }
      >
        <Button
          variant="outline"
          size="sm"
          disabled={busy}
          onClick={() => void api.chooseGameFolder().then(report)}
        >
          <FolderIcon />
          {t("set_change_folder")}
        </Button>
        {snap.game.root && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void api.openGameFolder()}
          >
            {t("set_open_folder")}
          </Button>
        )}
      </Row>
    )
    check = (
      <Row
        title={t("set_verify")}
        sub={t(snap.game.exeVerified ? "set_verify_ok" : "set_verify_text")}
      >
        <Button
          variant="outline"
          size="sm"
          disabled={busy || !snap.game.root}
          onClick={() => void api.verifyGame().then(report)}
        >
          <RefreshCwIcon />
          {t("set_verify_button")}
        </Button>
      </Row>
    )
  } else if (revived) {
    folder = (
      <Row title={t("set_folder")} sub={<Secret text={revived.root} />}>
        <Button
          variant="outline"
          size="sm"
          disabled={busy}
          onClick={() => void api.revivedChooseFolder().then(report)}
        >
          <FolderIcon />
          {t("set_change_folder")}
        </Button>
        {revived.problem !== "not_set" && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void api.openGameFolder()}
          >
            {t("set_open_folder")}
          </Button>
        )}
      </Row>
    )
    if (revived.problem !== "not_set") {
      check = (
        <Row title={t("set_verify")} sub={t("revived_check_text")}>
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() => void api.revivedInstall("repair").then(report)}
          >
            <RefreshCwIcon />
            {t("revived_repair")}
          </Button>
        </Row>
      )
    }
  }
  return (
    <Panel title={t("set_game")}>
      <div>
        {snap.app.legacyAvailable && (
          <Row title={t("set_game_edition")} sub={t("set_game_edition_help")}>
            <Choice<Game>
              id="edition-select"
              value={snap.settings.gameEdition}
              options={EDITIONS.map((e) => ({
                value: e,
                label: t(e === "legacy" ? "edition_legacy" : "edition_reborn"),
              }))}
              onChange={(gameEdition) => void api.setSettings({ gameEdition })}
            />
          </Row>
        )}
        {folder}
        {check}
        <Row title={t("set_game_language")} sub={t("set_game_language_text")}>
          <Choice<GameLanguage>
            id="game-language-select"
            value={snap.settings.gameLanguage}
            options={[
              { value: "auto", label: t("game_lang_auto") },
              ...GAME_LANGUAGES.map(([value, label]) => ({
                value,
                label,
                lang: value,
              })),
            ]}
            onChange={(gameLanguage) => void api.setSettings({ gameLanguage })}
          />
        </Row>
      </div>
    </Panel>
  )
}

function GraphicsTab({ snap }: { snap: Snapshot }) {
  const { t } = useI18n()
  return (
    <Panel title={t("set_graphics")}>
      <div>
        <Row title={t("set_graphics_level")} sub={t("set_graphics_text")}>
          <Choice<GraphicsPreset>
            id="graphics-select"
            value={snap.settings.graphics}
            options={GRAPHICS.map((g) => ({
              value: g,
              label: t(graphicsKey(g)),
            }))}
            onChange={(graphics) => void api.setSettings({ graphics })}
          />
        </Row>
        <Row title={t("set_windowed")} sub={t("set_windowed_help")}>
          <Toggle
            label={t("set_windowed")}
            checked={snap.settings.windowed}
            onChange={(windowed) => void api.setSettings({ windowed })}
          />
        </Row>
      </div>
    </Panel>
  )
}

function AccountTab({ snap }: { snap: Snapshot }) {
  const { t } = useI18n()
  const { report, showModal } = useLauncher()
  const busy = locked(snap)
  const name = snap.status?.name ?? snap.server?.name ?? ""
  return (
    <>
      <Panel title={t("set_account")}>
        <div>
          {snap.server && snap.account.hasKey ? (
            <Row
              title={t("set_signed_in", {
                user: snap.account.username ?? "?",
                name,
              })}
              sub={t("set_key_note")}
            >
              <Button
                variant="outline"
                size="sm"
                onClick={() => void api.saveKeyBackup().then(report)}
              >
                <KeyIcon />
                {t("set_backup")}
              </Button>
              <Button
                variant="destructive"
                size="sm"
                disabled={busy}
                onClick={() => showModal({ kind: "logout" })}
              >
                <LogOutIcon />
                {t("set_logout")}
              </Button>
            </Row>
          ) : (
            <Row title={t("set_not_signed")} />
          )}
          {snap.server && (
            <Row
              title={t("set_server_current")}
              sub={
                <span className="flex flex-wrap gap-2">
                  <span>{name}</span>
                  <Secret
                    className="font-mono"
                    text={`${snap.server.host}:${snap.server.port}`}
                  />
                </span>
              }
            >
              <Button
                variant="outline"
                size="sm"
                disabled={busy}
                onClick={() => showModal({ kind: "leave" })}
              >
                {t("set_add_server")}
              </Button>
            </Row>
          )}
        </div>
      </Panel>
      <SavedServers snap={snap} />
    </>
  )
}

function LauncherTab({ snap }: { snap: Snapshot }) {
  const { t } = useI18n()
  const { report } = useLauncher()
  const busy = locked(snap)
  return (
    <>
      <Panel title={t("set_language")}>
        <div
          role="group"
          aria-label={t("set_language")}
          className="flex flex-wrap gap-2"
        >
          {LANGUAGES.map((lang) => (
            <Button
              key={lang}
              lang={lang}
              variant={snap.settings.language === lang ? "default" : "outline"}
              aria-pressed={snap.settings.language === lang}
              onClick={() => void api.setSettings({ language: lang })}
            >
              {LANGUAGE_NAMES[lang]}
            </Button>
          ))}
        </div>
      </Panel>
      <Panel title={t("set_advanced")}>
        <div>
          {snap.app.hostToolsAvailable && (
            <Row title={t("set_host_tools")} sub={t("set_host_tools_text")}>
              <Toggle
                label={t("set_host_tools")}
                checked={snap.settings.hostTools}
                onChange={(hostTools) => void api.setSettings({ hostTools })}
              />
            </Row>
          )}
          <Row
            title={t("set_runtime_profile")}
            sub={t("set_runtime_profile_text")}
          >
            <Choice<RuntimeProfile>
              id="profile-select"
              value={snap.settings.runtimeProfile}
              options={PROFILES.map((p) => ({
                value: p,
                label: t(`profile_${p}`),
              }))}
              onChange={(runtimeProfile) =>
                void api.setSettings({ runtimeProfile })
              }
            />
          </Row>
          <Row
            title={t("set_local_runtime")}
            sub={
              snap.runtime.localDir ? (
                <Secret text={snap.runtime.localDir} />
              ) : (
                t("set_local_runtime_text")
              )
            }
          >
            <Button
              variant="outline"
              size="sm"
              disabled={busy}
              onClick={() => void api.chooseLocalRuntime().then(report)}
            >
              <FolderIcon />
              {t("set_local_runtime_choose")}
            </Button>
            {snap.runtime.localDir && (
              <Button
                variant="ghost"
                size="sm"
                disabled={busy}
                onClick={() => void api.clearLocalRuntime().then(report)}
              >
                <XIcon />
                {t("set_local_runtime_clear")}
              </Button>
            )}
          </Row>
          <Row title={t("set_streamer")} sub={t("set_streamer_help")}>
            <Toggle
              label={t("set_streamer")}
              checked={snap.settings.streamerMode}
              onChange={(streamerMode) =>
                void api.setSettings({ streamerMode })
              }
            />
          </Row>
          <Row title={t("set_logs")} sub={t("set_logs_text")}>
            <Button
              variant="outline"
              size="sm"
              onClick={() => void api.openLogsFolder()}
            >
              <FolderIcon />
              {t("set_logs_open")}
            </Button>
          </Row>
        </div>
      </Panel>
      <Panel title={t("set_about")}>
        <p className="text-sm">{t("about_version", { v: snap.app.version })}</p>
        <p className="text-sm">{t("about_license")}</p>
        <div className="flex flex-wrap gap-4">
          <ExternalLink target="project_source">
            {t("about_source")}
          </ExternalLink>
          <ExternalLink target="project_license">
            {t("about_license_link")}
          </ExternalLink>
          <ExternalLink target="project_issues">
            {t("about_issues")}
          </ExternalLink>
        </div>
        <p className="text-xs text-muted-foreground">{t("about_credits")}</p>
        <p className="text-xs text-muted-foreground">{t("about_disclaimer")}</p>
        <p className="text-xs text-muted-foreground">
          {t(snap.app.packaged ? "about_auto_updates" : "about_dev_build")}
        </p>
        {snap.app.updateReady && (
          <div>
            <Button
              disabled={snap.running.game}
              onClick={() => void api.installUpdate()}
            >
              {t("update_restart")}
            </Button>
          </div>
        )}
      </Panel>
    </>
  )
}

export function SettingsScreen() {
  const { t } = useI18n()
  const { snap } = useLauncher()
  if (!snap) return null
  const tabs = [
    {
      id: "game",
      key: "set_tab_game",
      icon: <Gamepad2Icon />,
      body: <GameTab snap={snap} />,
    },
    {
      id: "graphics",
      key: "set_tab_graphics",
      icon: <MonitorIcon />,
      body: <GraphicsTab snap={snap} />,
    },
    {
      id: "account",
      key: "set_tab_account",
      icon: <UserIcon />,
      body: <AccountTab snap={snap} />,
    },
    {
      id: "launcher",
      key: "set_tab_launcher",
      icon: <SettingsIcon />,
      body: <LauncherTab snap={snap} />,
    },
  ] as const
  return (
    <div className="flex max-w-4xl flex-col gap-6">
      <Heading title={t("set_title")} />
      <Tabs defaultValue="game">
        <TabsList aria-label={t("set_title")}>
          {tabs.map((tab) => (
            <TabsTrigger key={tab.id} value={tab.id}>
              {tab.icon}
              {t(tab.key)}
            </TabsTrigger>
          ))}
        </TabsList>
        {tabs.map((tab) => (
          <TabsContent
            key={tab.id}
            value={tab.id}
            className="flex flex-col gap-6 pt-4"
          >
            {tab.body}
          </TabsContent>
        ))}
      </Tabs>
    </div>
  )
}
