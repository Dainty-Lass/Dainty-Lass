// SPDX-License-Identifier: MIT OR Apache-2.0
// One running desktop app, driven over WebView2's DevTools port. Calls go through the page's own
// __TAURI_INTERNALS__, so the isolation hook checks them as it checks the page's.
import { mkdirSync } from "node:fs"
import path from "node:path"
import type { Subprocess } from "bun"
import { type Browser, chromium, type Page } from "playwright-core"

import { repoDir } from "../../scripts/restricted-data"

const CONNECT_MS = 30_000
const INTERNALS_MS = 16_000
const INVOKE_MS = 15_000
const LANGUAGE_MS = 2_000
const POLL_MS = 200

const screenshots = path.join(repoDir, "docs", "port", "screenshots")

// The backend's JSON, read loosely: each script checks only the fields it needs
export type Answer = any

interface TauriWindow {
  __TAURI_INTERNALS__?: { invoke?: (cmd: string, args: unknown) => Promise<unknown> }
}

export interface LaunchOptions {
  profile: string
  port: number
  label: string
  env?: Record<string, string>
  viewport?: { width: number; height: number }
}

export interface App {
  label: string
  page: Page
  invoke(cmd: string, args?: Record<string, unknown>): Promise<Answer>
  setLanguage(language: string): Promise<void>
  view(name: string): Promise<void>
  tab(index: number): Promise<void>
  shot(name: string): Promise<void>
  shots(base: string, languages?: string[]): Promise<void>
  waitFor(
    cmd: string,
    args: Record<string, unknown>,
    predicate: (answer: Answer) => boolean,
    timeoutMs: number
  ): Promise<Answer>
  close(): Promise<void>
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

// The app's own page, not the isolation frame's
function appPage(browser: Browser): Page | undefined {
  for (const context of browser.contexts()) {
    for (const page of context.pages()) {
      const url = page.url()
      if (url.startsWith("http://tauri.localhost") && !url.includes("__tauri_isolation__")) return page
    }
  }
  return undefined
}

async function connect(port: number, label: string): Promise<{ browser: Browser; page: Page }> {
  const deadline = Date.now() + CONNECT_MS
  let last: unknown
  while (Date.now() < deadline) {
    try {
      const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`)
      const page = appPage(browser)
      if (page) return { browser, page }
      await browser.close()
      last = new Error("no app page yet")
    } catch (error) {
      last = error
    }
    await sleep(500)
  }
  throw new Error(`${label}: no DevTools connection on port ${port}: ${String(last)}`)
}

export async function launch(exe: string, options: LaunchOptions): Promise<App> {
  const { profile, port, label } = options
  const child: Subprocess = Bun.spawn([exe], {
    env: {
      ...process.env,
      ...options.env,
      DAINTY_DEV_PROFILE: profile,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
    },
    stdio: ["ignore", "inherit", "inherit"],
  })

  let browser: Browser | undefined
  const close = async () => {
    try {
      await browser?.close()
    } catch {
      // The app may already be gone
    }
    if (child.exitCode === null) child.kill()
  }

  try {
    const connected = await connect(port, label)
    browser = connected.browser
    const page = connected.page
    if (options.viewport) await page.setViewportSize(options.viewport)
    await page.waitForFunction(
      () => typeof (window as unknown as TauriWindow).__TAURI_INTERNALS__?.invoke === "function",
      undefined,
      { timeout: INTERNALS_MS }
    )

    const invoke = async (cmd: string, args: Record<string, unknown> = {}): Promise<Answer> => {
      let timer: ReturnType<typeof setTimeout> | undefined
      const timeout = new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error(`invoke timeout for ${cmd}`)), INVOKE_MS)
      })
      const call = page.evaluate(
        ([c, a]) => (window as unknown as TauriWindow).__TAURI_INTERNALS__?.invoke?.(c, a),
        [cmd, args] as const
      )
      try {
        return await Promise.race([call, timeout])
      } finally {
        clearTimeout(timer)
      }
    }

    const setLanguage = async (language: string) => {
      await invoke("set_settings", { patch: { language } })
      await page.waitForFunction((lang) => document.documentElement.lang === lang, language, {
        timeout: LANGUAGE_MS,
      })
    }

    const shot = async (name: string) => {
      mkdirSync(screenshots, { recursive: true })
      await page.screenshot({ path: path.join(screenshots, `${name}.png`) })
    }

    return {
      label,
      page,
      invoke,
      setLanguage,
      view: (name) => page.locator(`[data-view="${name}"]`).click(),
      tab: (index) => page.locator('[role="tab"]').nth(index).click(),
      shot,
      shots: async (base, languages = ["en", "pt-BR"]) => {
        try {
          for (const language of languages) {
            await setLanguage(language)
            await shot(`${base}-${language}`)
          }
        } finally {
          await setLanguage("en")
        }
      },
      waitFor: async (cmd, args, predicate, timeoutMs) => {
        const deadline = Date.now() + timeoutMs
        for (;;) {
          const answer = await invoke(cmd, args)
          if (predicate(answer)) return answer
          if (Date.now() >= deadline) {
            throw new Error(`${label}: ${cmd} never matched in ${timeoutMs} ms: ${JSON.stringify(answer)}`)
          }
          await sleep(POLL_MS)
        }
      },
      close,
    }
  } catch (error) {
    await close()
    throw error
  }
}
