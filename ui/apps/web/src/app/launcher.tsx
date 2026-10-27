// The page's state: the backend's snapshot and progress, plus what only the page holds (the open dialog,
// the invite and name being typed, an error an action answered). Every bridge call the screens make goes
// through here.
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react"

import {
  api,
  inLauncher,
  type ActionResult,
  type Game,
  type InviteCheck,
  type LauncherError,
  type Snapshot,
  type TaskProgress,
} from "@workspace/bridge"

import {
  checkUsername,
  inviteSummary,
  primaryButton,
  type FormState,
  type InviteSummary,
} from "./model"

export type View = "play" | "server" | "host" | "settings" | "game"

export type Modal =
  | { kind: "invite"; link: string; invite: InviteSummary }
  | {
      kind: "cert_changed"
      link: string
      host: string
      oldFp: string | null
      newFp: string
    }
  | {
      kind: "server_moved"
      link: string
      name: string
      oldHost: string
      newHost: string
    }
  | {
      kind: "switch_edition"
      edition: Game
      name: string
      link: string | null
      id: string | null
    }
  | { kind: "trust_runtime" }
  | { kind: "logout" }
  | { kind: "leave" }
  | { kind: "remove_server"; id: string; name: string }

type Result = ActionResult | { ok: true; username: string }

export interface Launcher {
  snap: Snapshot | null
  task: TaskProgress | null
  // The Epic install's progress (kind "install"): the Game screen's, kept apart because a snapshot of any
  // phase but "installing" clears `task`
  installTask: TaskProgress | null
  localError: LauncherError | null
  modal: Modal | null
  view: View
  maximized: boolean
  inviteText: string
  inviteCheck: InviteCheck | null
  username: string
  form: FormState
  setView(view: View): void
  setInviteText(text: string): void
  setUsername(text: string): void
  showModal(modal: Modal | null): void
  dismissError(): void
  // An action's answer: its error goes to the banner when the backend didn't keep it
  report(result: Result): Promise<void>
  primary(): Promise<void>
  // The dialog's confirm button: resolves to the next dialog (a changed certificate), or null to close
  confirm(modal: Modal): Promise<Modal | null>
  switchTo(id: string): Promise<void>
}

const LauncherContext = createContext<Launcher | null>(null)

// Asked before any join from an invite: a changed certificate or a moved server get their warning
// first, never a direct join
async function inviteModal(link: string): Promise<Modal | null> {
  const invite = inviteSummary(link)
  const check = await api.checkInvite(link)
  if (!check.ok || invite === null) return null
  if (check.certificateChanged && invite.fp) {
    return {
      kind: "cert_changed",
      link,
      host: invite.host,
      oldFp: check.previousFingerprint,
      newFp: invite.fp,
    }
  }
  if (check.movedFrom) {
    return {
      kind: "server_moved",
      link,
      name: invite.name,
      oldHost: check.movedFrom,
      newHost: invite.host,
    }
  }
  return { kind: "invite", link, invite }
}

// The offer to switch editions, for an answer that says the server plays the other one
function editionModal(
  result: Result,
  name: string,
  to: { link?: string; id?: string }
): Modal | null {
  if (result.ok || result.error.code !== "edition_other") return null
  void api.dismissError()
  return {
    kind: "switch_edition",
    edition: result.error.detail === "reborn" ? "reborn" : "legacy",
    name,
    link: to.link ?? null,
    id: to.id ?? null,
  }
}

let listening = false

export function LauncherProvider({ children }: { children: ReactNode }) {
  const [snap, setSnap] = useState<Snapshot | null>(null)
  const [task, setTask] = useState<TaskProgress | null>(null)
  const [installTask, setInstallTask] = useState<TaskProgress | null>(null)
  const [localError, setLocalError] = useState<LauncherError | null>(null)
  const [modal, showModal] = useState<Modal | null>(null)
  const [view, setViewState] = useState<View>("play")
  const [maximized, setMaximized] = useState(false)
  const [inviteText, setInviteText] = useState("")
  const [inviteCheck, setInviteCheck] = useState<{
    text: string
    result: InviteCheck
  } | null>(null)
  const [username, setUsername] = useState("")
  const snapRef = useRef<Snapshot | null>(null)

  const onSnapshot = useCallback((next: Snapshot) => {
    snapRef.current = next
    setSnap(next)
    if (next.phase !== "installing") setTask(null)
  }, [])

  const checkInviteLink = useCallback(async () => {
    const link = await api.takeInviteLink()
    if (!link) return
    const next = await inviteModal(link)
    if (next) showModal(next)
  }, [])

  useEffect(() => {
    if (!inLauncher() || listening) return
    // The bridge's listeners can't be removed, so they are added once, not again on a StrictMode remount
    listening = true
    api.onSnapshot(onSnapshot)
    api.onProgress((progress) =>
      progress.kind === "install" ? setInstallTask(progress) : setTask(progress)
    )
    api.onWindowState(setMaximized)
    api.onInviteLink(() => void checkInviteLink())
    document.addEventListener("visibilitychange", () => {
      void api.setStatusPolling(!document.hidden)
    })
    void api.getSnapshot().then(
      (first) => {
        onSnapshot(first)
        void api.setStatusPolling(!document.hidden)
        void checkInviteLink()
      },
      (error: unknown) => console.error(error)
    )
  }, [onSnapshot, checkInviteLink])

  // The invite is read by the backend as it is typed, for the preview and the JOIN button
  useEffect(() => {
    const text = inviteText.trim()
    if (!inLauncher() || text.length === 0) return
    let current = true
    const timer = setTimeout(() => {
      void api.checkInvite(text).then((result) => {
        if (current) setInviteCheck({ text, result })
      })
    }, 200)
    return () => {
      current = false
      clearTimeout(timer)
    }
  }, [inviteText])

  const currentCheck =
    inviteCheck !== null && inviteCheck.text === inviteText.trim()
      ? inviteCheck.result
      : null
  const form = useMemo<FormState>(
    () => ({
      inviteValid: currentCheck?.ok === true,
      usernameValid: checkUsername(username.trim()) === "ok",
    }),
    [currentCheck, username]
  )

  const report = useCallback(
    async (result: Result) => {
      if (result.ok || result.error.code === "cancelled") return
      const latest = await api.getSnapshot().catch(() => null)
      if (latest) onSnapshot(latest)
      if (!latest?.lastError) setLocalError(result.error)
    },
    [onSnapshot]
  )

  const dismissError = useCallback(() => {
    setLocalError(null)
    void api.dismissError()
  }, [])

  const setView = useCallback((next: View) => {
    setViewState(next)
  }, [])

  const switchTo = useCallback(
    async (id: string) => {
      const result = await api.switchServer(id)
      const name =
        snapRef.current?.savedServers.find((sv) => sv.id === id)?.name ?? ""
      const other = editionModal(result, name, { id })
      if (other) {
        showModal(other)
        return
      }
      if (!result.ok) {
        await report(result)
        return
      }
      setViewState("play")
    },
    [report]
  )

  const submit = useCallback(
    async (link: string, result: ActionResult): Promise<Modal | null> => {
      if (!result.ok && result.error.code === "cert_changed") {
        return inviteModal(link)
      }
      if (result.ok && inviteText.trim() === link) setInviteText("")
      await report(result)
      setViewState("play")
      return null
    },
    [inviteText, report]
  )

  const confirm = useCallback(
    async (m: Modal): Promise<Modal | null> => {
      switch (m.kind) {
        case "invite": {
          const result = await api.submitInvite(m.link)
          const other = editionModal(result, m.invite.name, { link: m.link })
          if (other) return other
          return submit(m.link, result)
        }
        case "switch_edition":
          await api.setSettings({ gameEdition: m.edition })
          if (m.id !== null) {
            await switchTo(m.id)
            return null
          }
          if (m.link === null) return null
          return submit(m.link, await api.submitInvite(m.link))
        case "server_moved":
          return submit(m.link, await api.submitInvite(m.link, false, true))
        case "cert_changed":
          return submit(m.link, await api.submitInvite(m.link, true))
        case "trust_runtime":
          await report(await api.trustRuntime())
          return null
        case "logout":
          await report(await api.logout())
          return null
        case "leave": {
          const result = await api.forgetServer()
          if (!result.ok) await report(result)
          else setViewState("play")
          return null
        }
        case "remove_server":
          await report(await api.removeSavedServer(m.id))
          return null
      }
    },
    [report, submit, switchTo]
  )

  const primary = useCallback(async () => {
    const s = snapRef.current
    if (!s) return
    const button = primaryButton(s, form)
    if (button.disabled || button.action === "none") return
    setLocalError(null)
    let result: Result | undefined
    switch (button.action) {
      case "join": {
        const text = inviteText.trim()
        const next = await inviteModal(text)
        if (next?.kind === "cert_changed" || next?.kind === "server_moved") {
          showModal(next)
          return
        }
        result = await api.submitInvite(text)
        const other = editionModal(result, inviteSummary(text)?.name ?? "", {
          link: text,
        })
        if (other) {
          showModal(other)
          return
        }
        if (result.ok) {
          setInviteText("")
        } else if (result.error.code === "cert_changed") {
          const again = await inviteModal(text)
          if (again) showModal(again)
          return
        }
        break
      }
      case "retry":
        result = await api.retryConnect()
        break
      case "new_invite":
        showModal({ kind: "leave" })
        return
      case "register":
        result = await api.register(username.trim())
        if (result.ok) setUsername("")
        break
      case "locate":
        result = await api.chooseGameFolder()
        break
      case "verify":
        result = await api.verifyGame()
        break
      case "revived_install":
        result = await api.revivedInstall("install")
        break
      case "trust":
        showModal({ kind: "trust_runtime" })
        return
      case "install_runtime":
        result = await api.installRuntime()
        break
      case "play":
        result = await api.play()
        break
      case "cancel":
        await api.cancelTask()
        return
    }
    if (result) await report(result)
  }, [form, inviteText, username, report])

  const value = useMemo<Launcher>(
    () => ({
      snap,
      task,
      installTask,
      localError,
      modal,
      view,
      maximized,
      inviteText,
      inviteCheck: currentCheck,
      username,
      form,
      setView,
      setInviteText,
      setUsername,
      showModal,
      dismissError,
      report,
      primary,
      confirm,
      switchTo,
    }),
    [
      snap,
      task,
      installTask,
      localError,
      modal,
      view,
      maximized,
      inviteText,
      currentCheck,
      username,
      form,
      setView,
      dismissError,
      report,
      primary,
      confirm,
      switchTo,
    ]
  )

  return (
    <LauncherContext.Provider value={value}>
      {children}
    </LauncherContext.Provider>
  )
}

export function useLauncher(): Launcher {
  const value = useContext(LauncherContext)
  if (value === null) throw new Error("useLauncher outside LauncherProvider")
  return value
}
