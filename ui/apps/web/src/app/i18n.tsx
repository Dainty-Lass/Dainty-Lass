// The player's language for every screen: the settings' choice once the backend answered, the system's
// before that
import { createContext, useContext, useMemo, type ReactNode } from "react"

import {
  isLanguage,
  systemLanguage,
  translate,
  type Key,
  type Language,
} from "@workspace/i18n"

import type { Message, Var } from "./model"

export interface Translator {
  lang: Language
  t(key: Key, vars?: Record<string, Var>): string
  msg(message: Message): string
}

function make(lang: Language): Translator {
  const t = (key: Key, vars?: Record<string, Var>): string => {
    if (!vars) return translate(lang, key)
    const plain: Record<string, string | number> = {}
    for (const [name, value] of Object.entries(vars)) {
      plain[name] =
        typeof value === "object" ? translate(lang, value.key) : value
    }
    return translate(lang, key, plain)
  }
  return { lang, t, msg: (m) => t(m.key, m.vars) }
}

const I18n = createContext<Translator>(make("en"))

export function I18nProvider({
  language,
  children,
}: {
  language: string | null
  children: ReactNode
}) {
  const lang: Language = isLanguage(language)
    ? language
    : systemLanguage(navigator.language)
  const value = useMemo(() => make(lang), [lang])
  return <I18n.Provider value={value}>{children}</I18n.Provider>
}

export function useI18n(): Translator {
  return useContext(I18n)
}
