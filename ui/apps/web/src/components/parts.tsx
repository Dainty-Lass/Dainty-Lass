// The pieces every screen is built from
import type { ReactNode } from "react"
import { LockIcon, NetworkIcon } from "lucide-react"

import type { ExternalTarget, ServerGame, ServerMode } from "@workspace/bridge"
import { api } from "@workspace/bridge"
import { Badge } from "@workspace/ui/components/badge"
import { Button } from "@workspace/ui/components/button"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@workspace/ui/components/card"
import { cn } from "@workspace/ui/lib/utils"

import { useI18n } from "@/app/i18n"

export function Heading({
  eyebrow,
  title,
  text,
}: {
  eyebrow?: ReactNode
  title: string
  text?: string | null
}) {
  return (
    <header className="flex flex-col gap-2">
      {eyebrow && (
        <div className="flex flex-wrap items-center gap-3 text-xs font-semibold tracking-widest text-muted-foreground uppercase">
          {eyebrow}
        </div>
      )}
      <h1 className="font-heading text-2xl font-semibold tracking-wide">
        {title}
      </h1>
      {text && <p className="max-w-prose text-muted-foreground">{text}</p>}
    </header>
  )
}

export function Panel({
  title,
  text,
  icon,
  tone = "default",
  children,
  className,
}: {
  title?: ReactNode
  text?: ReactNode
  icon?: ReactNode
  tone?: "default" | "warn" | "danger" | "accent"
  children?: ReactNode
  className?: string
}) {
  return (
    <Card
      className={cn(
        tone === "warn" && "border-l-2 border-l-amber-500",
        tone === "danger" && "border-l-2 border-l-destructive",
        tone === "accent" && "border-l-2 border-l-primary",
        className
      )}
    >
      {(title || text) && (
        <CardHeader>
          {title && (
            <CardTitle className="flex items-center gap-2">
              {icon}
              {title}
            </CardTitle>
          )}
          {text && <CardDescription>{text}</CardDescription>}
        </CardHeader>
      )}
      {children && (
        <CardContent className="flex flex-col gap-4">{children}</CardContent>
      )}
    </Card>
  )
}

// A titled row with its controls on the right, as in the settings
export function Row({
  title,
  sub,
  children,
}: {
  title: ReactNode
  sub?: ReactNode
  children?: ReactNode
}) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-4 border-b py-3 last:border-b-0">
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <span className="text-sm font-medium">{title}</span>
        {sub && (
          <span className="text-xs break-all text-muted-foreground">{sub}</span>
        )}
      </div>
      {children && (
        <div className="flex flex-wrap items-center gap-2">{children}</div>
      )}
    </div>
  )
}

export function ModeBadge({ mode }: { mode: ServerMode }) {
  const { t } = useI18n()
  return (
    <Badge variant="secondary">
      {mode === "public" ? <LockIcon /> : <NetworkIcon />}
      {t(mode === "public" ? "mode_public" : "mode_private")}
    </Badge>
  )
}

// A Dauntless Revived server runs Dauntless 1.4.4, said next to its name wherever servers are listed
export function EditionBadge({ game }: { game: ServerGame }) {
  const { t } = useI18n()
  if (game !== "revived") return null
  return <Badge variant="outline">{t("edition_revived")}</Badge>
}

export function ExternalLink({
  target,
  children,
}: {
  target: ExternalTarget
  children: ReactNode
}) {
  return (
    <Button
      variant="link"
      size="xs"
      className="px-0"
      onClick={() => void api.openExternal(target)}
    >
      {children}
    </Button>
  )
}

export function Spinner() {
  return (
    <span
      role="presentation"
      className="inline-block size-4 animate-spin rounded-full border-2 border-muted-foreground/30 border-t-foreground"
    />
  )
}
