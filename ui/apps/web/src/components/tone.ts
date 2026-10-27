// The colour of a state's badge: green when it works, amber while it changes, red when it failed
import type { Tone } from "@/app/host-model"

export function toneClass(tone: Tone): string {
  switch (tone) {
    case "good":
      return "text-emerald-600 dark:text-emerald-400"
    case "busy":
      return "text-amber-600 dark:text-amber-400"
    case "bad":
      return "text-destructive"
    default:
      return "text-muted-foreground"
  }
}
