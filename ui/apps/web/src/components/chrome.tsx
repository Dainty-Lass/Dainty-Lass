// The window's own frame (window.rs makes it frameless): the title bar with its controls, and the rail
import {
  CopyIcon,
  DownloadIcon,
  HardDriveIcon,
  MinusIcon,
  PlayIcon,
  ServerIcon,
  SettingsIcon,
  SquareIcon,
  XIcon,
} from "lucide-react"

import { api, inLauncher } from "@workspace/bridge"
import { Button } from "@workspace/ui/components/button"
import { cn } from "@workspace/ui/lib/utils"

import { hostPageAvailable } from "@/app/host-model"
import { useI18n } from "@/app/i18n"
import { useLauncher, type View } from "@/app/launcher"

export function TitleBar() {
  const { t } = useI18n()
  const { maximized } = useLauncher()
  return (
    <header
      data-tauri-drag-region
      className="flex h-10 shrink-0 items-center justify-between border-b pl-4 text-xs font-semibold tracking-widest uppercase select-none"
    >
      <span data-tauri-drag-region>{t("app_title")}</span>
      {inLauncher() && (
        <div className="flex">
          <Button
            variant="ghost"
            size="icon"
            aria-label={t("window_minimize")}
            title={t("window_minimize")}
            onClick={() => void api.minimize()}
          >
            <MinusIcon />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            aria-label={t(maximized ? "window_restore" : "window_maximize")}
            title={t(maximized ? "window_restore" : "window_maximize")}
            onClick={() => void api.toggleMaximize()}
          >
            {maximized ? <CopyIcon /> : <SquareIcon />}
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="hover:bg-destructive hover:text-white"
            aria-label={t("window_close")}
            title={t("window_close")}
            onClick={() => void api.close()}
          >
            <XIcon />
          </Button>
        </div>
      )}
    </header>
  )
}

const NAV: {
  view: View
  key: "nav_play" | "nav_server" | "nav_host" | "nav_settings" | "nav_game"
  icon: typeof PlayIcon
}[] = [
  { view: "play", key: "nav_play", icon: PlayIcon },
  { view: "server", key: "nav_server", icon: ServerIcon },
  { view: "game", key: "nav_game", icon: DownloadIcon },
  { view: "host", key: "nav_host", icon: HardDriveIcon },
  { view: "settings", key: "nav_settings", icon: SettingsIcon },
]

export function Rail() {
  const { t } = useI18n()
  const { view, setView, snap } = useLauncher()
  const user = snap?.account.hasKey ? snap.account.username : null
  const serverName = snap?.status?.name ?? snap?.server?.name ?? null
  // The Host page is the server GUI's, or a player build's once Host tools are on
  const items = NAV.filter(
    ({ view: target }) => target !== "host" || hostPageAvailable(snap)
  )
  return (
    <nav
      aria-label={t("nav_label")}
      className="flex w-52 shrink-0 flex-col justify-between border-r"
    >
      <div className="flex flex-col gap-1 p-3">
        <div className="px-3 pt-2 pb-4">
          <div className="font-heading text-lg font-semibold">
            {t("logo_alt")}
          </div>
          <div className="text-xs text-muted-foreground">
            {t(
              snap?.server?.game === "revived" ? "edition_revived" : "tile_sub"
            )}
          </div>
        </div>
        <ul className="flex flex-col gap-1">
          {items.map(({ view: target, key, icon: Icon }) => (
            <li key={target}>
              <Button
                variant="ghost"
                aria-current={view === target ? "page" : undefined}
                data-view={target}
                className={cn(
                  "w-full justify-start",
                  view === target && "bg-muted text-foreground"
                )}
                onClick={() => setView(target)}
              >
                <Icon />
                {t(key)}
              </Button>
            </li>
          ))}
        </ul>
      </div>
      <div className="flex items-center gap-3 border-t p-4">
        <span
          aria-hidden="true"
          className="flex size-8 shrink-0 items-center justify-center bg-muted text-sm font-semibold"
        >
          {user ? (Array.from(user)[0]?.toUpperCase() ?? "?") : "?"}
        </span>
        <span className="flex min-w-0 flex-col text-xs">
          <span className="truncate text-muted-foreground">
            {serverName ?? t("rail_not_joined")}
          </span>
          <span className="truncate font-medium">
            {user ?? t("rail_not_signed_in")}
          </span>
        </span>
      </div>
    </nav>
  )
}
