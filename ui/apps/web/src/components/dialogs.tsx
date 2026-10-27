// The questions the launcher asks before anything it can't undo or shouldn't do unasked: joining from an
// invite link, trusting a new certificate or a moved server, allowing a runtime, switching editions,
// logging out, leaving or removing a server
import { useState, type ReactNode } from "react"
import { InfoIcon, ShieldIcon, TriangleAlertIcon } from "lucide-react"

import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@workspace/ui/components/alert-dialog"
import { Button } from "@workspace/ui/components/button"

import { useI18n } from "@/app/i18n"
import { useLauncher, type Modal } from "@/app/launcher"
import { groupedHex, shortHex } from "@/app/model"
import { ModeBadge } from "@/components/parts"
import { RuntimeFiles } from "@/components/runtime-files"
import { useHidden } from "@/components/secret"

interface Shape {
  title: string
  body: ReactNode
  confirm: string
  cancel?: string
  danger?: boolean
  icon?: ReactNode
}

function Warn({ icon, children }: { icon: ReactNode; children: ReactNode }) {
  return (
    <p className="flex gap-2 text-sm text-amber-500 [&_svg]:mt-0.5 [&_svg]:size-4 [&_svg]:shrink-0">
      {icon}
      <span>{children}</span>
    </p>
  )
}

function useShape(m: Modal): Shape {
  const { t } = useI18n()
  const { snap } = useLauncher()
  const hide = useHidden()
  const name = snap?.status?.name ?? snap?.server?.name ?? ""
  const edition = (e: "reborn" | "legacy") =>
    t(e === "legacy" ? "edition_legacy" : "edition_reborn")
  switch (m.kind) {
    case "invite":
      return {
        title: t("link_title", { name: m.invite.name }),
        body: (
          <>
            <p>
              {t("link_text", {
                name: m.invite.name,
                host: hide(m.invite.host),
              })}
            </p>
            <ModeBadge mode={m.invite.mode} />
            {m.invite.fp && (
              <p className="font-mono text-xs">
                {t("link_fingerprint", { fp: hide(shortHex(m.invite.fp)) })}
              </p>
            )}
            {snap?.server && <p>{t("link_switch_note")}</p>}
          </>
        ),
        confirm: t("link_join"),
        cancel: t("link_cancel"),
      }
    case "switch_edition":
      return {
        title: t("edition_switch_title", { edition: edition(m.edition) }),
        body: (
          <p>
            {t("edition_switch_text", {
              name: m.name,
              edition: edition(m.edition),
            })}
          </p>
        ),
        confirm: t("edition_switch_confirm"),
      }
    case "server_moved":
      return {
        title: t("moved_title", { name: m.name }),
        body: (
          <>
            <p>
              {t("moved_text", {
                name: m.name,
                old: hide(m.oldHost),
                new: hide(m.newHost),
              })}
            </p>
            <Warn icon={<InfoIcon />}>{t("moved_advice")}</Warn>
          </>
        ),
        confirm: t("moved_confirm"),
      }
    case "cert_changed":
      return {
        title: t("certchg_title"),
        icon: <ShieldIcon className="size-5" />,
        danger: true,
        body: (
          <>
            <p>{t("certchg_text", { host: hide(m.host) })}</p>
            <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs">
              <dt>{t("certchg_old")}</dt>
              <dd className="font-mono break-all">
                {m.oldFp ? groupedHex(m.oldFp) : t("certchg_unknown")}
              </dd>
              <dt>{t("certchg_new")}</dt>
              <dd className="font-mono break-all text-amber-500">
                {groupedHex(m.newFp)}
              </dd>
            </dl>
            <Warn icon={<TriangleAlertIcon />}>{t("certchg_advice")}</Warn>
          </>
        ),
        confirm: t("certchg_confirm"),
      }
    case "trust_runtime":
      return {
        title: t("trust_title", { name }),
        icon: <ShieldIcon className="size-5" />,
        body: (
          <>
            <p>{t("trust_text", { name })}</p>
            <p>{t("trust_detail")}</p>
            {snap && <RuntimeFiles files={snap.runtime.files} />}
            <Warn icon={<ShieldIcon />}>{t("trust_advice")}</Warn>
          </>
        ),
        confirm: t("trust_confirm"),
      }
    case "logout":
      return {
        title: t("logout_title", { name }),
        body: <p>{t("logout_text")}</p>,
        confirm: t("logout_confirm"),
        danger: true,
      }
    case "leave":
      return {
        title: t("leave_title"),
        body: <p>{t("leave_text", { name })}</p>,
        confirm: t("leave_confirm"),
      }
    case "remove_server":
      return {
        title: t("remove_server_title", { name: m.name }),
        body: <p>{t("remove_server_text", { name: m.name })}</p>,
        confirm: t("remove_server_confirm"),
      }
  }
}

function Dialog({ modal }: { modal: Modal }) {
  const { t } = useI18n()
  const { showModal, confirm } = useLauncher()
  const [pending, setPending] = useState(false)
  const shape = useShape(modal)
  const run = () => {
    setPending(true)
    void confirm(modal)
      .catch(() => null)
      .then((next) => {
        setPending(false)
        showModal(next)
      })
  }
  return (
    <AlertDialogContent data-kind={modal.kind}>
      <AlertDialogHeader>
        <AlertDialogTitle className="flex items-center gap-2">
          {shape.icon}
          {shape.title}
        </AlertDialogTitle>
        <AlertDialogDescription
          render={<div className="flex flex-col gap-3" />}
        >
          {shape.body}
        </AlertDialogDescription>
      </AlertDialogHeader>
      <AlertDialogFooter>
        {/* Risky choices never take the first focus */}
        <AlertDialogCancel autoFocus>
          {shape.cancel ?? t("cancel")}
        </AlertDialogCancel>
        <Button
          variant={shape.danger ? "destructive" : "default"}
          disabled={pending}
          onClick={run}
        >
          {shape.confirm}
        </Button>
      </AlertDialogFooter>
    </AlertDialogContent>
  )
}

export function Dialogs() {
  const { modal, showModal } = useLauncher()
  return (
    <AlertDialog
      open={modal !== null}
      onOpenChange={(open) => {
        if (!open) showModal(null)
      }}
    >
      {modal && <Dialog key={JSON.stringify(modal)} modal={modal} />}
    </AlertDialog>
  )
}
