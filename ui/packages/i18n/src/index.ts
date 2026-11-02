// The launcher's two catalogs (catalogs/*.json, AGPL-3.0-only: NOTICE.md) and the lookup over them (MIT).
// en.json is the source: every other language has exactly its keys and {placeholders}. DaintyLass makes the
// Tauri backend's text, rust/launcher/src-tauri/gen/strings.json, from these files (RR_UPDATE_GENERATED=1).
import en from "../catalogs/en.json"
import ptBR from "../catalogs/pt-BR.json"

export type Key = keyof typeof en

// In the order the settings list them; the backend's Language enum follows the same files
export const LANGUAGES = ["en", "pt-BR"] as const
export type Language = (typeof LANGUAGES)[number]

export const CATALOGS: Record<Language, Record<Key, string>> = {
  en,
  "pt-BR": ptBR,
}

// Each language in its own words, so a player can find theirs
export const LANGUAGE_NAMES: Record<Language, string> = {
  en: "English",
  "pt-BR": "Português (Brasil)",
}

// Every date, time and number the page renders goes through these (AGENTS.md, "User-facing text")
export const TIME_LOCALES: Record<Language, string> = {
  en: "en-GB",
  "pt-BR": "pt-BR",
}

export type Vars = Record<string, string | number>

export function isKey(key: string): key is Key {
  return Object.prototype.hasOwnProperty.call(en, key)
}

// The language's text, else English, else the key itself, as the backend's core/i18n.rs does
export function translate(lang: Language, key: Key, vars?: Vars): string {
  let text = CATALOGS[lang]?.[key] ?? CATALOGS.en[key] ?? key
  if (vars) {
    for (const [name, value] of Object.entries(vars)) {
      text = text.split(`{${name}}`).join(String(value))
    }
  }
  return text
}

export function isLanguage(value: unknown): value is Language {
  return (LANGUAGES as readonly unknown[]).includes(value)
}

// A first start's language from the system locale: the exact code, else the first language of the same
// family (any Portuguese locale gives pt-BR), else English; as the backend's system_language
export function systemLanguage(locale: string): Language {
  const code = locale.toLowerCase().replace(/_/g, "-")
  const family = code.split("-")[0]
  return (
    LANGUAGES.find((lang) => lang.toLowerCase() === code) ??
    LANGUAGES.find((lang) => lang.toLowerCase().split("-")[0] === family) ??
    "en"
  )
}
