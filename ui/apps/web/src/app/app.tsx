// DU1's shell: one bridge call and its answer. The player screens and their catalogs are DU2's.
import { useCallback, useEffect, useState } from "react"
import { XIcon } from "lucide-react"

import { call, closeWindow, inLauncher, type Snapshot } from "@workspace/bridge"
import { Button } from "@workspace/ui/components/button"

type Check =
  | { state: "waiting" }
  | { state: "ok"; snapshot: Snapshot }
  | { state: "failed"; reason: string }
  | { state: "outside" }

export function App() {
  const [check, setCheck] = useState<Check>(() =>
    inLauncher() ? { state: "waiting" } : { state: "outside" }
  )

  const fetchSnapshot = useCallback(() => {
    if (!inLauncher()) return
    call<Snapshot>("get_snapshot").then(
      (snapshot) => setCheck({ state: "ok", snapshot }),
      (error: unknown) => setCheck({ state: "failed", reason: String(error) })
    )
  }, [])

  useEffect(fetchSnapshot, [fetchSnapshot])

  const run = () => {
    if (!inLauncher()) return
    setCheck({ state: "waiting" })
    fetchSnapshot()
  }

  return (
    <div className="flex min-h-svh flex-col">
      <header
        data-tauri-drag-region
        className="flex h-10 items-center justify-between border-b pl-4 text-sm font-medium select-none"
      >
        <span data-tauri-drag-region>DaintyLass</span>
        {inLauncher() && (
          <Button
            variant="ghost"
            size="icon"
            aria-label="Close"
            onClick={() => void closeWindow()}
          >
            <XIcon />
          </Button>
        )}
      </header>
      <main className="flex flex-1 flex-col items-start gap-4 p-6 text-sm">
        <p id="bridge" data-state={check.state}>
          {describe(check)}
        </p>
        <Button
          onClick={run}
          disabled={check.state === "waiting" || check.state === "outside"}
        >
          get_snapshot
        </Button>
        {check.state === "ok" && (
          <pre className="max-w-full overflow-auto rounded-md border p-3 text-xs">
            {JSON.stringify(check.snapshot, null, 2)}
          </pre>
        )}
      </main>
    </div>
  )
}

function describe(check: Check): string {
  switch (check.state) {
    case "waiting":
      return "bridge: calling get_snapshot…"
    case "ok":
      return `bridge: ok (phase ${check.snapshot.phase})`
    case "failed":
      return `bridge: ${check.reason}`
    case "outside":
      return "bridge: not in the launcher"
  }
}
