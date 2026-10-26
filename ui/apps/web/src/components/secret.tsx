// Streamer mode keeps what could identify a server or this PC off the screen: addresses, fingerprints,
// folders (they hold the Windows user name) and other players' names. One the player needs can be shown
// for a moment.
import { useState } from "react"
import { EyeIcon, EyeOffIcon } from "lucide-react"

import { Button } from "@workspace/ui/components/button"
import { cn } from "@workspace/ui/lib/utils"

import { useI18n } from "@/app/i18n"
import { useLauncher } from "@/app/launcher"

export function useStreamer(): boolean {
  return useLauncher().snap?.settings.streamerMode === true
}

// Same length as the text, so the layout doesn't jump when it's shown
function filler(text: string): string {
  return "•".repeat(Math.min(Math.max(text.length, 6), 32))
}

export function Secret({
  text,
  className,
  revealable = true,
}: {
  text: string
  className?: string
  revealable?: boolean
}) {
  const { t } = useI18n()
  const streamer = useStreamer()
  const [shown, setShown] = useState(false)
  if (!streamer)
    return <span className={cn("select-text", className)}>{text}</span>
  return (
    <span className="inline-flex items-center gap-1">
      <span
        className={cn(shown ? "select-text" : "blur-xs select-none", className)}
      >
        {shown ? text : filler(text)}
      </span>
      {revealable && (
        <Button
          variant="ghost"
          size="icon-xs"
          aria-label={t(shown ? "streamer_hide" : "streamer_show")}
          title={t(shown ? "streamer_hide" : "streamer_show")}
          onClick={() => setShown(!shown)}
        >
          {shown ? <EyeOffIcon /> : <EyeIcon />}
        </Button>
      )}
    </span>
  )
}

// A value spliced into a sentence (a host in "No answer from {host}"): hidden without a button
export function useHidden(): (text: string) => string {
  const streamer = useStreamer()
  return (text) => (streamer ? filler(text) : text)
}
