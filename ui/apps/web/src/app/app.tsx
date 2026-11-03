// The launcher's page: the frame, the rail and one screen at a time, over the bridge's snapshot
import { useEffect } from "react"

import { inLauncher } from "@workspace/bridge"

import { useI18n, I18nProvider } from "@/app/i18n"
import { LauncherProvider, useLauncher } from "@/app/launcher"
import { dismiss, settle } from "@/app/splash"
import { ActionBar } from "@/components/action-bar"
import { Banners } from "@/components/banners"
import { Rail, TitleBar } from "@/components/chrome"
import { Dialogs } from "@/components/dialogs"
import { GameScreen } from "@/screens/game"
import { HostScreen } from "@/screens/host"
import { PlayScreen } from "@/screens/play"
import { RealmsScreen } from "@/screens/realms"
import { ServerScreen } from "@/screens/servers"
import { SettingsScreen } from "@/screens/settings"

function Shell() {
  const { t, lang } = useI18n()
  const { view, snap } = useLauncher()
  const serverGui = snap?.app.serverGui ?? false
  useEffect(() => {
    document.documentElement.lang = lang
    document.title = t(serverGui ? "app_title_manager" : "app_title")
  }, [lang, serverGui, t])
  return (
    <div className="flex h-svh flex-col bg-background text-foreground">
      <TitleBar />
      <div className="flex min-h-0 flex-1">
        <Rail />
        <main className="flex min-w-0 flex-1 flex-col">
          <div className="min-h-0 flex-1 overflow-y-auto">
            <div className="flex flex-col gap-4 p-6" data-view={view}>
              <Banners />
              {view === "play" && <PlayScreen />}
              {view === "realms" && <RealmsScreen />}
              {view === "server" && <ServerScreen />}
              {view === "game" && <GameScreen />}
              {view === "host" && <HostScreen />}
              {view === "settings" && <SettingsScreen />}
            </div>
          </div>
          <ActionBar />
        </main>
      </div>
      <Dialogs />
    </div>
  )
}

// The splash spins from the first frame (index.astro) until the first snapshot, then settles over the page and
// holds the lit ring for a beat before fading
function Splash() {
  const { booted } = useLauncher()
  useEffect(() => {
    if (booted) void settle().then(() => setTimeout(dismiss, 600))
  }, [booted])
  return null
}

function Localized() {
  const { snap } = useLauncher()
  return (
    <I18nProvider language={snap?.settings.language ?? null}>
      <Shell />
      <Splash />
    </I18nProvider>
  )
}

export function App() {
  // No browser context menu, as in the Electron launcher, except Paste in a text field (debug builds still
  // open devtools with F12)
  useEffect(() => {
    if (!inLauncher()) return
    const block = (event: MouseEvent) => {
      const target = event.target as HTMLElement | null
      if (!target?.closest("input, textarea")) event.preventDefault()
    }
    document.addEventListener("contextmenu", block)
    return () => document.removeEventListener("contextmenu", block)
  }, [])
  return (
    <LauncherProvider>
      <Localized />
    </LauncherProvider>
  )
}
