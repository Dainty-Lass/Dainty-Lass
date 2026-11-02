// Numbers, sizes and times in the player's language, each through TIME_LOCALES
import { TIME_LOCALES, type Language } from "@workspace/i18n"

const UNITS = ["B", "KB", "MB", "GB", "TB"]

function decimal(value: number, digits: number, lang: Language): string {
  return new Intl.NumberFormat(TIME_LOCALES[lang], {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
    useGrouping: false,
  }).format(value)
}

// Decimal units, as the backend's download sizes are counted
export function formatBytes(bytes: number, lang: Language): string {
  let value = Number.isFinite(bytes) && bytes > 0 ? bytes : 0
  let unit = 0
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1000
    unit++
  }
  const digits = unit === 0 ? 0 : value >= 100 ? 0 : value >= 10 ? 1 : 2
  return `${decimal(value, digits, lang)} ${UNITS[unit]}`
}

export function formatSpeed(bytesPerSecond: number, lang: Language): string {
  return `${formatBytes(bytesPerSecond, lang)}/s`
}

// 277200 -> "3 d 5 h", 3725 -> "1 h 2 min", 95 -> "1 min 35 s"; the short units read the same in both languages
export function formatDuration(seconds: number | null): string {
  if (seconds === null || !Number.isFinite(seconds) || seconds < 0) return "–"
  const s = Math.round(seconds)
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  const sec = s % 60
  if (h >= 24) return `${Math.floor(h / 24)} d ${h % 24} h`
  if (h > 0) return `${h} h ${m} min`
  if (m > 0) return `${m} min ${sec} s`
  return `${sec} s`
}

export function formatPercent(done: number, total: number): number {
  if (!(total > 0)) return 0
  return Math.max(0, Math.min(100, Math.floor((done / total) * 1000) / 10))
}

export function formatPercentText(value: number, lang: Language): string {
  return new Intl.NumberFormat(TIME_LOCALES[lang], {
    style: "percent",
    maximumFractionDigits: value < 10 ? 1 : 0,
  }).format(value / 100)
}

// A date and time, for something that may be a day old
export function formatDateTime(iso: string, lang: Language): string {
  const t = Date.parse(iso)
  if (!Number.isFinite(t)) return ""
  return new Date(t).toLocaleString(TIME_LOCALES[lang], {
    dateStyle: "medium",
    timeStyle: "short",
  })
}

export function formatTime(iso: string, lang: Language): string {
  const t = Date.parse(iso)
  if (!Number.isFinite(t)) return ""
  return new Date(t).toLocaleTimeString(TIME_LOCALES[lang], {
    hour: "2-digit",
    minute: "2-digit",
  })
}

// How long an instance has run: "42 min", "3 h 5 min"; null under a minute
export function runningMinutes(
  startedAt: string | null,
  now: number
): number | null {
  if (startedAt === null) return null
  const t = Date.parse(startedAt)
  if (!Number.isFinite(t)) return null
  return Math.max(0, Math.floor((now - t) / 60000))
}
