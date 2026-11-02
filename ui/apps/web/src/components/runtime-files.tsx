import type { RuntimeFileInfo } from "@workspace/bridge"

import { useI18n } from "@/app/i18n"
import { formatBytes } from "@/app/format"
import { shortHex } from "@/app/model"

// What the server offers as its game runtime: each file's name, size and SHA-256, the same list the
// player allows
export function RuntimeFiles({ files }: { files: RuntimeFileInfo[] }) {
  const { lang } = useI18n()
  return (
    <ul className="flex flex-col divide-y text-sm">
      {files.map((f) => (
        <li key={f.name} className="flex flex-wrap items-center gap-4 py-2">
          <span className="flex-1 font-mono">{f.name}</span>
          <span className="text-muted-foreground">
            {formatBytes(f.size, lang)}
          </span>
          <span
            className="font-mono text-xs text-muted-foreground"
            title={f.sha256}
          >
            {shortHex(f.sha256)}
          </span>
        </li>
      ))}
    </ul>
  )
}
