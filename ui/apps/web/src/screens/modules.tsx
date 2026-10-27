// The server GUI's module panels (DU3, DD11): everything on a card is drawn from what the module says about
// itself, so no module ships UI code. A module is named by its manifest id, never by a path (DD5). The
// "unverified" mark is the manifest's (DD9); a plugin's look (DD25) is drawn in the theme slot.
import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import type { RefObject } from "react"
import {
  PaletteIcon,
  PlayIcon,
  RefreshCwIcon,
  RotateCwIcon,
  SquareIcon,
  TerminalIcon,
  TriangleAlertIcon,
} from "lucide-react"

import {
  api,
  type LauncherError,
  type ModuleCommandOutcome,
  type ModuleCommandView,
  type ModuleOutcome,
  type ModuleView,
} from "@workspace/bridge"
import { Alert, AlertDescription } from "@workspace/ui/components/alert"
import { Badge } from "@workspace/ui/components/badge"
import { Button } from "@workspace/ui/components/button"
import { Input } from "@workspace/ui/components/input"
import { Label } from "@workspace/ui/components/label"
import { Switch } from "@workspace/ui/components/switch"
import { cn } from "@workspace/ui/lib/utils"

import { useHost } from "@/app/host-store"
import {
  activeTheme,
  applyPalette,
  blankInput,
  capabilityLines,
  commandInput,
  inputProblem,
  MODULE_ORIGIN_KEYS,
  MODULE_ROLE_KEYS,
  MODULE_STATE_KEYS,
  moduleButtons,
  modulePorts,
  moduleTone,
  settingId,
  settingKind,
  settingsPatch,
  settingText,
  type ActiveTheme,
} from "@/app/host-model"
import { useI18n } from "@/app/i18n"
import { errorMessage } from "@/app/model"
import { Panel, Spinner } from "@/components/parts"
import { toneClass } from "@/components/tone"

// How often a module that failed on its own shows as failed, when nothing else told the page
const REFRESH_MS = 5000

function ErrorLine({ error }: { error: LauncherError }) {
  const { msg } = useI18n()
  const message = errorMessage(error, null)
  return (
    <Alert variant="destructive" data-code={error.code}>
      <TriangleAlertIcon />
      <AlertDescription>
        <span>{msg(message)}</span>
        {message.detail && (
          <span className="block font-mono text-xs break-all">
            {message.detail}
          </span>
        )}
      </AlertDescription>
    </Alert>
  )
}

function Swatch({ colour, name }: { colour: string; name: string }) {
  const box = useRef<HTMLSpanElement>(null)
  useEffect(() => {
    // Through the CSSOM: the page's CSP has no inline style
    box.current?.style.setProperty("background-color", colour)
  }, [colour])
  return (
    <span className="flex items-center gap-2 text-xs">
      <span
        ref={box}
        aria-hidden="true"
        className="inline-block size-5 border"
      />
      <span className="font-mono">{name}</span>
    </span>
  )
}

// The look a loaded plugin brings (DD25): its palette and logo, and a preview on the server screens
function ThemeSlot({
  modules,
  scope,
}: {
  modules: ModuleView[]
  scope: RefObject<HTMLElement | null>
}) {
  const { t } = useI18n()
  const active = useMemo<ActiveTheme | null>(
    () => activeTheme(modules),
    [modules]
  )
  const [preview, setPreview] = useState(false)
  const palette = active?.theme.palette
  useEffect(() => {
    const element = scope.current
    if (!preview || !element || !palette) return
    return applyPalette(element, palette)
  }, [preview, palette, scope])
  return (
    <Panel
      title={t("mod_theme_title")}
      text={t("mod_theme_text")}
      icon={<PaletteIcon />}
    >
      {active === null ? (
        <p className="text-sm text-muted-foreground">{t("mod_theme_none")}</p>
      ) : (
        <>
          <p className="text-sm font-medium">
            {t("mod_theme_from", {
              theme: active.theme.displayName,
              module: active.moduleName,
            })}
          </p>
          <div className="flex flex-wrap gap-x-6 gap-y-2">
            {active.theme.palette.map((entry) => (
              <Swatch key={entry.name} name={entry.name} colour={entry.value} />
            ))}
          </div>
          {active.theme.layout && (
            <p className="text-xs text-muted-foreground">
              {t("mod_theme_layout", { layout: active.theme.layout })}
            </p>
          )}
          {active.theme.logo && (
            <p className="text-xs break-all text-muted-foreground">
              {t("mod_theme_logo", { logo: active.theme.logo })}
            </p>
          )}
          <Label className="flex items-center gap-3">
            <Switch checked={preview} onCheckedChange={setPreview} />
            {t("mod_theme_preview")}
          </Label>
        </>
      )}
    </Panel>
  )
}

// One command of the module, as a form drawn from its registered arguments
function CommandForm({
  module,
  command,
}: {
  module: ModuleView
  command: ModuleCommandView
}) {
  const { t, msg } = useI18n()
  const [values, setValues] = useState(() => blankInput(command))
  const [running, setRunning] = useState(false)
  const [answer, setAnswer] = useState<ModuleCommandOutcome | null>(null)
  const [problem, setProblem] = useState<LauncherError | null>(null)
  const id = `cmd-${module.id}-${command.name}`
  const run = async () => {
    const wrong = inputProblem(command, values)
    if (wrong) {
      setProblem({
        code: "module_input_invalid",
        detail: `${wrong.reason}:${wrong.name}`,
      })
      return
    }
    setProblem(null)
    setRunning(true)
    const out = await api.moduleRunCommand(
      module.id,
      command.name,
      commandInput(command, values)
    )
    setRunning(false)
    setAnswer(out)
  }
  const data = answer?.ok && answer.data !== null ? pretty(answer.data) : null
  return (
    <form
      className="flex flex-col gap-3 border p-3"
      aria-label={command.name}
      onSubmit={(event) => {
        event.preventDefault()
        void run()
      }}
    >
      <div className="flex flex-col gap-1">
        <span className="font-mono text-sm font-medium">{command.name}</span>
        <span className="text-xs text-muted-foreground">{command.summary}</span>
      </div>
      {command.args.map((arg) => (
        <div key={arg.name} className="flex flex-col gap-1">
          <Label htmlFor={`${id}-${arg.name}`} className="gap-2">
            <span className="font-mono">{arg.name}</span>
            {arg.required && (
              <span className="text-xs text-muted-foreground">
                {t("mod_cmd_required")}
              </span>
            )}
          </Label>
          <Input
            id={`${id}-${arg.name}`}
            value={values[arg.name] ?? ""}
            maxLength={1024}
            autoComplete="off"
            onChange={(event) =>
              setValues({ ...values, [arg.name]: event.target.value })
            }
          />
          <span className="text-xs text-muted-foreground">{arg.help}</span>
        </div>
      ))}
      <div className="flex items-center gap-3">
        <Button type="submit" size="sm" disabled={running}>
          {running ? <Spinner /> : <TerminalIcon />}
          {t("mod_cmd_run")}
        </Button>
      </div>
      {problem && <ErrorLine error={problem} />}
      {answer && !answer.ok && <ErrorLine error={answer.error} />}
      {answer?.ok && (
        <div className="flex flex-col gap-2" role="status">
          <span className="text-xs font-semibold tracking-widest text-muted-foreground uppercase">
            {t("mod_cmd_output")}
          </span>
          {answer.text === "" && data === null ? (
            <p className="text-sm text-muted-foreground">
              {t("mod_cmd_empty_output")}
            </p>
          ) : (
            answer.text !== "" && (
              <pre className="overflow-x-auto bg-muted p-2 font-mono text-xs whitespace-pre-wrap">
                {answer.text}
              </pre>
            )
          )}
          {data !== null && (
            <>
              <span className="text-xs font-semibold tracking-widest text-muted-foreground uppercase">
                {t("mod_cmd_data")}
              </span>
              <pre className="overflow-x-auto bg-muted p-2 font-mono text-xs">
                {data}
              </pre>
            </>
          )}
        </div>
      )}
      {/* The message of the last problem is also for screen readers */}
      <span className="sr-only" aria-live="polite">
        {problem ? msg(errorMessage(problem, null)) : ""}
      </span>
    </form>
  )
}

function pretty(json: string): string {
  try {
    return JSON.stringify(JSON.parse(json), null, 2)
  } catch {
    return json
  }
}

// The keys the module registered with `host.toml`'s values, saved back to it
function SettingsEditor({
  module,
  onChange,
}: {
  module: ModuleView
  onChange(module: ModuleView): void
}) {
  const { t, msg } = useI18n()
  const [edits, setEdits] = useState<Record<string, string>>({})
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(false)
  const [error, setError] = useState<LauncherError | null>(null)
  if (module.settings.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        {t(
          module.state === "not_loaded" || module.state === "starting"
            ? "mod_settings_not_loaded"
            : "mod_settings_none"
        )}
      </p>
    )
  }
  const result = settingsPatch(module.settings, edits)
  const dirty = result.ok && Object.keys(result.patch).length > 0
  const save = async () => {
    if (!result.ok) {
      setError({ code: "module_settings_invalid", detail: `type:${result.id}` })
      return
    }
    setSaving(true)
    setSaved(false)
    const out = await api.moduleSetSettings(module.id, result.patch)
    setSaving(false)
    if (out.ok) {
      setError(null)
      setEdits({})
      setSaved(true)
      onChange(out.module)
    } else {
      setError(out.error)
    }
  }
  return (
    <form
      className="flex flex-col gap-4"
      onSubmit={(event) => {
        event.preventDefault()
        void save()
      }}
    >
      {module.settings.map((section) => (
        <fieldset key={section.name} className="flex flex-col gap-3">
          <legend className="font-mono text-sm font-medium">
            {section.name}
          </legend>
          {section.keys.map((key) => {
            const id = `set-${module.id}-${settingId(section.name, key.name)}`
            const text = edits[settingId(section.name, key.name)]
            const kind = settingKind(key.default)
            const edit = (next: string) => {
              setSaved(false)
              setEdits({ ...edits, [settingId(section.name, key.name)]: next })
            }
            return (
              <div key={key.name} className="flex flex-col gap-1">
                <div className="flex flex-wrap items-center justify-between gap-3">
                  <Label htmlFor={id} className="font-mono">
                    {key.name}
                  </Label>
                  {kind === "flag" ? (
                    <Switch
                      id={id}
                      checked={(text ?? settingText(key.value)) === "true"}
                      onCheckedChange={(on) => edit(String(on))}
                    />
                  ) : (
                    <Input
                      id={id}
                      className="max-w-72"
                      inputMode={kind === "number" ? "decimal" : "text"}
                      value={text ?? settingText(key.value)}
                      maxLength={4096}
                      autoComplete="off"
                      onChange={(event) => edit(event.target.value)}
                    />
                  )}
                </div>
                <span className="text-xs text-muted-foreground">
                  {key.help}
                </span>
                <span className="text-xs break-all text-muted-foreground">
                  {t("mod_settings_default", {
                    value:
                      kind === "flag"
                        ? t(
                            key.default === true
                              ? "mod_value_on"
                              : "mod_value_off"
                          )
                        : settingText(key.default),
                  })}
                </span>
              </div>
            )
          })}
        </fieldset>
      ))}
      <div className="flex flex-wrap items-center gap-3">
        <Button type="submit" size="sm" disabled={saving || !dirty}>
          {saving && <Spinner />}
          {t("mod_settings_save")}
        </Button>
        {saved && (
          <span role="status" className="text-xs text-muted-foreground">
            {t("mod_settings_saved")}
          </span>
        )}
      </div>
      {error && <ErrorLine error={error} />}
      <span className="sr-only" aria-live="polite">
        {error ? msg(errorMessage(error, null)) : ""}
      </span>
    </form>
  )
}

function ModuleCard({
  module,
  onChange,
}: {
  module: ModuleView
  onChange(module: ModuleView): void
}) {
  const { t } = useI18n()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<LauncherError | null>(null)
  const buttons = moduleButtons(module.state)
  const act = async (call: () => Promise<ModuleOutcome>) => {
    setBusy(true)
    const out = await call()
    setBusy(false)
    if (out.ok) {
      setError(null)
      onChange(out.module)
    } else {
      setError(out.error)
    }
    return out.ok
  }
  const restart = async () => {
    if (await act(() => api.moduleUnload(module.id))) {
      await act(() => api.moduleLoad(module.id))
    }
  }
  const ports = modulePorts(module)
  const caps = capabilityLines(module)
  return (
    <Panel
      title={
        <span className="flex flex-wrap items-center gap-3">
          <span>{module.displayName ?? module.id}</span>
          <Badge
            variant="outline"
            className={toneClass(moduleTone(module.state))}
          >
            {t(MODULE_STATE_KEYS[module.state])}
          </Badge>
          {module.unverified && (
            <Badge variant="destructive" data-unverified="">
              <TriangleAlertIcon />
              {t("mod_unverified")}
            </Badge>
          )}
        </span>
      }
      text={
        <span className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs">
          <span className="font-mono">{module.id}</span>
          {module.role && <span>{t(MODULE_ROLE_KEYS[module.role])}</span>}
          <span>{t(MODULE_ORIGIN_KEYS[module.origin])}</span>
          {module.gameVersion && (
            <span>{t("mod_version", { version: module.gameVersion })}</span>
          )}
          {module.autoload && <span>{t("mod_autoload")}</span>}
        </span>
      }
      tone={module.state === "failed" ? "danger" : "default"}
    >
      {module.unverified && (
        <p className="text-sm text-muted-foreground">
          {t("mod_unverified_text")}
        </p>
      )}
      {module.detail && (
        <p
          className={cn(
            "font-mono text-xs break-all",
            module.state === "failed"
              ? "text-destructive"
              : "text-muted-foreground"
          )}
        >
          {module.detail}
        </p>
      )}
      <div className="flex flex-wrap items-center gap-2">
        {buttons.load && (
          <Button
            size="sm"
            disabled={busy}
            onClick={() => void act(() => api.moduleLoad(module.id))}
          >
            {busy ? <Spinner /> : <PlayIcon />}
            {t(module.state === "failed" ? "mod_restart" : "mod_load")}
          </Button>
        )}
        {buttons.restart && (
          <Button
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() => void restart()}
          >
            <RotateCwIcon />
            {t("mod_restart")}
          </Button>
        )}
        {buttons.unload && (
          <Button
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() => void act(() => api.moduleUnload(module.id))}
          >
            <SquareIcon />
            {t("mod_unload")}
          </Button>
        )}
      </div>
      {error && <ErrorLine error={error} />}
      {module.displayName === null && (
        <p className="text-sm text-muted-foreground">{t("mod_no_manifest")}</p>
      )}
      {(ports.length > 0 || caps.length > 0) && (
        <div className="grid gap-4 sm:grid-cols-2">
          {ports.length > 0 && (
            <div className="flex flex-col gap-1">
              <h3 className="text-xs font-semibold tracking-widest text-muted-foreground uppercase">
                {t("mod_ports")}
              </h3>
              <ul className="flex flex-col gap-1 text-sm">
                {ports.map((port) => (
                  <li key={port.name}>{t("mod_port_line", port)}</li>
                ))}
              </ul>
            </div>
          )}
          {caps.length > 0 && (
            <div className="flex flex-col gap-1">
              <h3 className="text-xs font-semibold tracking-widest text-muted-foreground uppercase">
                {t("mod_caps")}
              </h3>
              <ul className="flex flex-col gap-1 text-sm">
                {caps.map((cap) => (
                  <li key={cap.key}>{t(cap.key, cap.vars)}</li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}
      <div className="flex flex-col gap-3">
        <h3 className="text-xs font-semibold tracking-widest text-muted-foreground uppercase">
          {t("mod_commands")}
        </h3>
        {module.commands.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t(
              module.state === "ready"
                ? "mod_commands_none"
                : "mod_commands_not_loaded"
            )}
          </p>
        ) : (
          module.commands.map((command) => (
            <CommandForm
              key={`${module.state}-${command.name}`}
              module={module}
              command={command}
            />
          ))
        )}
      </div>
      <div className="flex flex-col gap-3">
        <h3 className="text-xs font-semibold tracking-widest text-muted-foreground uppercase">
          {t("mod_settings")}
        </h3>
        <SettingsEditor module={module} onChange={onChange} />
      </div>
    </Panel>
  )
}

export function ModulesTab({
  scope,
}: {
  scope: RefObject<HTMLElement | null>
}) {
  const { t } = useI18n()
  const { server } = useHost()
  const [modules, setModules] = useState<ModuleView[] | null>(null)
  const [error, setError] = useState<LauncherError | null>(null)
  const refresh = useCallback(() => {
    void api.moduleList().then((out) => {
      if (out.ok) {
        setModules(out.modules)
        setError(null)
      } else {
        setModules(null)
        setError(out.error)
      }
    })
  }, [])
  // A module is a component of the server too: when any component changes, so may the list
  const seen = (server?.components ?? [])
    .map((c) => `${c.name}:${c.phase}`)
    .join(",")
  useEffect(() => {
    refresh()
    const timer = setInterval(refresh, REFRESH_MS)
    return () => clearInterval(timer)
  }, [refresh, seen])
  const replace = useCallback((next: ModuleView) => {
    setModules((now) => now?.map((m) => (m.id === next.id ? next : m)) ?? now)
  }, [])
  return (
    <div className="flex flex-col gap-4">
      <ThemeSlot modules={modules ?? []} scope={scope} />
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex flex-col gap-1">
          <h2 className="font-heading text-lg font-semibold">
            {t("mod_title")}
          </h2>
          <p className="max-w-prose text-sm text-muted-foreground">
            {t("mod_text")}
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={refresh}>
          <RefreshCwIcon />
          {t("mod_refresh")}
        </Button>
      </div>
      {error && <ErrorLine error={error} />}
      {modules?.length === 0 && (
        <p className="text-sm text-muted-foreground">{t("mod_empty")}</p>
      )}
      {modules?.map((module) => (
        <ModuleCard key={module.id} module={module} onChange={replace} />
      ))}
    </div>
  )
}
