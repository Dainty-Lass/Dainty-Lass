// The checks the launcher's i18n test made on its language files, now on the catalogs here
import { expect, test } from "bun:test"
import { readFileSync } from "node:fs"

import {
  CATALOGS,
  isKey,
  LANGUAGES,
  systemLanguage,
  TIME_LOCALES,
  translate,
  type Key,
  type Language,
} from "../src"

function placeholders(text: string): string[] {
  return [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]!).sort()
}

test("every language has exactly the English keys, the same placeholders and no empty text", () => {
  const en = CATALOGS.en
  for (const lang of LANGUAGES) {
    const table = CATALOGS[lang]
    expect(Object.keys(table)).toEqual(Object.keys(en))
    for (const key of Object.keys(en) as Key[]) {
      expect(table[key].trim().length, `${lang}.${key}`).toBeGreaterThan(0)
      expect(placeholders(table[key]), `${lang}.${key}`).toEqual(
        placeholders(en[key])
      )
    }
  }
})

test("the files keep one key per line in append order, so a new key is a one-line diff at the end", () => {
  for (const lang of LANGUAGES) {
    const text = readFileSync(
      new URL(`../catalogs/${lang}.json`, import.meta.url),
      "utf8"
    )
    expect(text).toBe(JSON.stringify(CATALOGS[lang], null, 2) + "\n")
  }
})

test("the Portuguese table is really translated", () => {
  // Names (Ramsgate, Legendary, Dauntless 1.14.7...) may stay the same; prose may not
  for (const key of Object.keys(CATALOGS.en) as Key[]) {
    if (CATALOGS.en[key] !== CATALOGS["pt-BR"][key]) continue
    expect(CATALOGS.en[key].split(" ").length, `pt-BR.${key}`).toBeLessThan(5)
  }
})

test("a dauntless-revived:// invite and a saved 1.4.4 server say 1.4.4 (RD5, RD22)", () => {
  for (const lang of LANGUAGES) {
    expect(CATALOGS[lang].invite_err_revived).toContain("1.4.4")
    expect(CATALOGS[lang].server_revived_disabled).toContain("1.4.4")
  }
})

test("no name DD1 retired is left", () => {
  for (const lang of LANGUAGES) {
    for (const [key, text] of Object.entries(CATALOGS[lang])) {
      expect(text, `${lang}.${key}`).not.toContain("Reforged Revival")
    }
  }
})

test("translate fills placeholders and falls back to the key", () => {
  expect(translate("en", "ready_title", { user: "Ana" })).toBe(
    "Welcome back, Ana"
  )
  expect(translate("pt-BR", "ready_title", { user: "Ana" })).toBe(
    "Bem-vindo de volta, Ana"
  )
  expect(translate("en", "no_such_key" as Key)).toBe("no_such_key")
  expect(isKey("btn_play")).toBe(true)
  expect(isKey("__proto__")).toBe(false)
})

test("systemLanguage maps any Portuguese locale to pt-BR, as the backend does", () => {
  const cases: [string, Language][] = [
    ["pt-BR", "pt-BR"],
    ["pt-PT", "pt-BR"],
    ["pt_BR", "pt-BR"],
    ["PT-br", "pt-BR"],
    ["pt", "pt-BR"],
    ["ptx", "en"],
    ["en-US", "en"],
    ["en", "en"],
    ["fi", "en"],
    ["", "en"],
  ]
  for (const [locale, lang] of cases) expect(systemLanguage(locale)).toBe(lang)
})

test("every language has a time locale", () => {
  for (const lang of LANGUAGES) {
    expect(() => new Intl.DateTimeFormat(TIME_LOCALES[lang])).not.toThrow()
  }
})
