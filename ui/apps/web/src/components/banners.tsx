// What the backend wants the player to know right now: the last error, a notice, a launcher update
import { InfoIcon, RefreshCwIcon, TriangleAlertIcon } from "lucide-react"

import { api } from "@workspace/bridge"
import {
  Alert,
  AlertAction,
  AlertDescription,
} from "@workspace/ui/components/alert"
import { Button } from "@workspace/ui/components/button"

import { useI18n } from "@/app/i18n"
import { useLauncher } from "@/app/launcher"
import { errorMessage, NOTICE_KEYS, shownError } from "@/app/model"

export function Banners() {
  const { t, msg } = useI18n()
  const { snap, localError, dismissError } = useLauncher()
  const error = shownError(snap, localError)
  const message = error ? errorMessage(error, snap?.server?.mode ?? null) : null
  return (
    <div className="flex flex-col gap-2 empty:hidden" id="banners">
      {message && (
        <Alert variant="destructive" data-code={error?.code}>
          <TriangleAlertIcon />
          <AlertDescription>
            <span>{msg(message)}</span>
            {message.detail && (
              <span className="block font-mono text-xs break-all">
                {message.detail}
              </span>
            )}
          </AlertDescription>
          <AlertAction>
            <Button variant="ghost" size="xs" onClick={dismissError}>
              {t("err_dismiss")}
            </Button>
          </AlertAction>
        </Alert>
      )}
      {snap?.notice && (
        <Alert role="status">
          <InfoIcon />
          <AlertDescription>{t(NOTICE_KEYS[snap.notice])}</AlertDescription>
          <AlertAction>
            <Button
              variant="ghost"
              size="xs"
              onClick={() => void api.dismissNotice()}
            >
              {t("err_dismiss")}
            </Button>
          </AlertAction>
        </Alert>
      )}
      {snap?.app.updateReady && (
        <Alert role="status">
          <RefreshCwIcon />
          <AlertDescription>{t("update_ready")}</AlertDescription>
          <AlertAction>
            <Button
              size="xs"
              disabled={snap.running.game}
              onClick={() => void api.installUpdate()}
            >
              {t("update_restart")}
            </Button>
          </AlertAction>
        </Alert>
      )}
    </div>
  )
}
