// The Game screen's state (DE7): the Epic account, the library check, the version list and each install, over
// the bridge's epic_* and game_* commands. The Epic session never reaches the page: epicGet answers who is
// signed in, and nothing else.
import { useCallback, useEffect, useRef, useState } from "react"

import {
  api,
  type EpicLibraryCheck,
  type EpicView,
  type GameVersionList,
  type InstallInfo,
  type InstallOutcome,
  type LauncherError,
} from "@workspace/bridge"

import { isRunning, type VersionAction } from "./epic"
import { useLauncher } from "./launcher"

const ACCOUNT_POLL_MS = 1500
const INSTALL_POLL_MS = 1000

export interface EpicState {
  account: EpicView | null
  // Null until Epic has answered; follows the sign-in
  library: EpicLibraryCheck | null
  list: GameVersionList | null
  installs: Record<string, InstallInfo>
  // What is in flight: "sign_in", "code", "sign_out", "library", "list" or a version
  working: string | null
  signIn(): Promise<void>
  submitCode(code: string): Promise<boolean>
  signOut(): Promise<void>
  checkLibrary(): Promise<void>
  refreshList(): Promise<void>
  act(version: string, action: VersionAction): Promise<void>
}

export function useEpic(): EpicState {
  const { report } = useLauncher()
  const [account, setAccount] = useState<EpicView | null>(null)
  const [libraryNow, setLibrary] = useState<EpicLibraryCheck | null>(null)
  const [list, setList] = useState<GameVersionList | null>(null)
  const [installs, setInstalls] = useState<Record<string, InstallInfo>>({})
  const [working, setWorking] = useState<string | null>(null)
  const installsRef = useRef(installs)
  useEffect(() => {
    installsRef.current = installs
  }, [installs])

  const refreshAccount = useCallback(async () => {
    let next: EpicView | null = null
    try {
      next = await api.epicGet()
    } catch (error) {
      console.error(error)
    }
    if (next) setAccount(next)
  }, [])

  const refreshInstall = useCallback(async (version: string) => {
    const outcome = await api.gameInstallGet(version)
    if (outcome.ok)
      setInstalls((prev) => ({ ...prev, [version]: outcome.install }))
  }, [])

  // The version list and each Epic version's install, read without touching the state
  const fetchList = useCallback(async () => {
    const next = await api.gameVersionList()
    const outcomes = next.ok
      ? await Promise.all(
          next.versions
            .filter((entry) => entry.install === "epic")
            .map((entry) => api.gameInstallGet(entry.version))
        )
      : []
    return { next, outcomes }
  }, [])

  const applyList = useCallback(
    ({
      next,
      outcomes,
    }: {
      next: GameVersionList
      outcomes: InstallOutcome[]
    }) => {
      setList(next)
      setInstalls((prev) => {
        const out = { ...prev }
        for (const outcome of outcomes)
          if (outcome.ok) out[outcome.install.version] = outcome.install
        return out
      })
    },
    []
  )

  // Who is signed in, and the versions (their manifests are asked for, so this takes a moment)
  useEffect(() => {
    let current = true
    api.epicGet().then(
      (next) => {
        if (current) setAccount(next)
      },
      (error: unknown) => console.error(error)
    )
    fetchList().then(
      (read) => {
        if (current) applyList(read)
      },
      (error: unknown) => console.error(error)
    )
    return () => {
      current = false
    }
  }, [fetchList, applyList])

  // The library is checked once a session exists (Epic's answer is kept for a day by the backend)
  const signedIn = account?.signedIn === true
  useEffect(() => {
    if (!signedIn) return
    let current = true
    void api.epicLibraryCheck().then((result) => {
      if (current) setLibrary(result)
    })
    return () => {
      current = false
    }
  }, [signedIn])

  // The sign-in window runs apart from the page: ask until it is done
  const signingIn = account?.signingIn === true
  useEffect(() => {
    if (!signingIn) return
    const timer = setInterval(() => void refreshAccount(), ACCOUNT_POLL_MS)
    return () => clearInterval(timer)
  }, [signingIn, refreshAccount])

  // A running install: its state and numbers, once a second
  const anyRunning = Object.values(installs).some((info) =>
    isRunning(info.state)
  )
  useEffect(() => {
    if (!anyRunning) return
    const timer = setInterval(() => {
      for (const info of Object.values(installsRef.current)) {
        if (isRunning(info.state)) void refreshInstall(info.version)
      }
    }, INSTALL_POLL_MS)
    return () => clearInterval(timer)
  }, [anyRunning, refreshInstall])

  const guard = useCallback(async (what: string, run: () => Promise<void>) => {
    setWorking(what)
    try {
      await run()
    } finally {
      setWorking(null)
    }
  }, [])

  const signIn = useCallback(
    () =>
      guard("sign_in", async () => {
        // Answers once the window closes: signed in, or why not
        const result = await api.epicSignInStart()
        await refreshAccount()
        await report(result)
      }),
    [guard, refreshAccount, report]
  )

  const submitCode = useCallback(
    async (code: string) => {
      let ok = false
      await guard("code", async () => {
        const result = await api.epicSignInCode(code)
        ok = result.ok
        await refreshAccount()
        await report(result)
      })
      return ok
    },
    [guard, refreshAccount, report]
  )

  const signOut = useCallback(
    () =>
      guard("sign_out", async () => {
        const result = await api.epicSignOut()
        setLibrary(null)
        await refreshAccount()
        await report(result)
      }),
    [guard, refreshAccount, report]
  )

  const checkLibrary = useCallback(
    () =>
      guard("library", async () => {
        setLibrary(await api.epicLibraryCheck())
      }),
    [guard]
  )

  const refreshList = useCallback(
    () => guard("list", async () => applyList(await fetchList())),
    [guard, fetchList, applyList]
  )

  const fail = useCallback(
    (error: LauncherError) => report({ ok: false, error }),
    [report]
  )

  const act = useCallback(
    (version: string, action: VersionAction) =>
      guard(version, async () => {
        switch (action) {
          case "install":
          case "continue":
          case "retry": {
            const outcome = await api.gameInstallStart(version)
            if (outcome.ok) {
              setInstalls((prev) => ({ ...prev, [version]: outcome.install }))
            } else {
              await fail(outcome.error)
              await refreshInstall(version)
            }
            return
          }
          case "pause":
            await report(await api.gameInstallPause(version))
            break
          case "resume":
            await report(await api.gameInstallResume(version))
            break
          case "cancel":
            await report(await api.gameInstallCancel(version))
            break
          case "use": {
            const folder = installsRef.current[version]?.folder
            if (folder) await report(await api.useGamePath(folder))
            return
          }
        }
        await refreshInstall(version)
      }),
    [guard, fail, report, refreshInstall]
  )

  return {
    account,
    library: signedIn ? libraryNow : null,
    list,
    installs,
    working,
    signIn,
    submitCode,
    signOut,
    checkLibrary,
    refreshList,
    act,
  }
}
