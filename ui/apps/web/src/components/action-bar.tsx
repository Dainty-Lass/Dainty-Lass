// The bar under every screen: where the player is on the way to the game and the one button that moves on,
// or a running task's progress
import { CheckIcon } from "lucide-react"

import { Button } from "@workspace/ui/components/button"
import { Progress, ProgressLabel } from "@workspace/ui/components/progress"
import { cn } from "@workspace/ui/lib/utils"

import { useI18n } from "@/app/i18n"
import { useLauncher } from "@/app/launcher"
import {
  hintKey,
  primaryButton,
  STEP_KEYS,
  stepIndex,
  taskView,
} from "@/app/model"
import { formatPercentText } from "@/app/format"

function Steps({ current, running }: { current: number; running: boolean }) {
  const { t } = useI18n()
  return (
    <ol
      aria-label={t("steps_label")}
      className="flex items-center gap-2 text-xs"
    >
      {STEP_KEYS.map((key, i) => {
        const done = i < current || running
        return (
          <li
            key={key}
            aria-current={i === current ? "step" : undefined}
            className={cn(
              "flex items-center gap-1.5",
              done
                ? "text-foreground"
                : i === current
                  ? "text-primary"
                  : "text-muted-foreground"
            )}
          >
            <span
              className={cn(
                "flex size-5 items-center justify-center border text-[0.625rem]",
                done && "border-foreground",
                i === current && !done && "border-primary"
              )}
            >
              {done ? <CheckIcon className="size-3" /> : i + 1}
            </span>
            <span className="hidden sm:inline">{t(key)}</span>
          </li>
        )
      })}
    </ol>
  )
}

export function ActionBar() {
  const { t, lang } = useI18n()
  const { snap, task, form, primary } = useLauncher()
  if (!snap) {
    return (
      <footer className="flex h-20 items-center justify-end border-t px-6">
        <Button size="lg" disabled>
          {t("btn_wait")}
        </Button>
      </footer>
    )
  }
  const button = primaryButton(snap, form)
  const progress = snap.phase === "installing" ? (task ?? snap.task) : null
  const view = progress ? taskView(progress, lang) : null
  const hint = hintKey(snap)
  return (
    <footer className="flex min-h-20 items-center gap-6 border-t px-6 py-3">
      <div className="flex min-w-0 flex-1 flex-col gap-2">
        {progress && view ? (
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
              {progress.currentFile && (
                <span className="truncate">
                  {t("task_current", { file: progress.currentFile })}
                </span>
              )}
            </span>
          </Progress>
        ) : (
          <>
            <Steps
              current={stepIndex(snap.phase)}
              running={snap.phase === "running"}
            />
            {hint && (
              <p aria-live="polite" className="text-xs text-muted-foreground">
                {t(hint)}
              </p>
            )}
          </>
        )}
      </div>
      <Button
        size="lg"
        id="primary"
        data-action={button.action}
        variant={button.action === "cancel" ? "outline" : "default"}
        className="min-w-44"
        disabled={button.disabled}
        onClick={() => void primary()}
      >
        {t(button.key)}
      </Button>
    </footer>
  )
}
