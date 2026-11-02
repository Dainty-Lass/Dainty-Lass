// SPDX-License-Identifier: MIT
// The hook against the real generated table. Limits are read from the table, never restated, so a change to the
// backend's spec moves the tests with it.
import { describe, expect, test } from "bun:test"
import { readFileSync } from "node:fs"
import path from "node:path"

import { loadHook, type Shape } from "./hook"

const { hook, commands, settingsKeys } = loadHook()

function allows(cmd: unknown, args?: unknown): boolean {
  // Tauri's payload carries more than the hook looks at
  const payload = { cmd, payload: args, callback: 1, error: 2, options: {} }
  return hook(payload) === payload
}

function shapeOf(cmd: string, arg: string): Shape {
  const shape = commands[cmd]?.[arg]
  if (!shape) throw new Error(`the table has no ${cmd}.${arg}`)
  return shape
}

function limit(cmd: string, arg: string): number {
  const max = shapeOf(cmd, arg).max
  if (max === undefined) throw new Error(`${cmd}.${arg} has no max`)
  return max
}

// A value inside every bound of the shape
function valid(shape: Shape): unknown {
  switch (shape.kind) {
    case "text":
    case "optionalText":
      return "a".repeat(shape.max ?? 0)
    case "flag":
      return true
    case "index":
      return shape.max
    case "name":
      return shape.names?.[0]
    case "number":
      return shape.min ?? shape.max ?? 0
    case "object":
      return validArgs(shape.fields ?? {})
    case "settingsPatch":
      return { [settingsKeys[0] ?? "language"]: true }
    case "channel":
      return "__CHANNEL__:1"
    default:
      throw new Error(`no valid value for kind ${shape.kind}`)
  }
}

function validArgs(fields: Record<string, Shape>): Record<string, unknown> {
  return Object.fromEntries(Object.entries(fields).map(([name, shape]) => [name, valid(shape)]))
}

describe("the generated table", () => {
  test("is loaded and complete", () => {
    expect(Object.keys(commands).length).toBeGreaterThan(90)
    expect(settingsKeys.length).toBeGreaterThan(0)
  })

  for (const [cmd, fields] of Object.entries(commands)) {
    test(`${cmd} passes with well-shaped arguments`, () => {
      expect(allows(cmd, validArgs(fields))).toBe(true)
    })
  }
})

describe("Tauri's own calls", () => {
  test("event listen and unlisten", () => {
    expect(allows("plugin:event|listen", { event: "snapshot", target: { kind: "Any" }, handler: 7 })).toBe(true)
    expect(allows("plugin:event|listen", { event: "snapshot", target: "anything", handler: 7 })).toBe(true)
    expect(allows("plugin:event|listen", { event: "snapshot", target: { kind: "Any" }, handler: "7" })).toBe(false)
    expect(allows("plugin:event|listen", { event: "e".repeat(65), target: null, handler: 7 })).toBe(false)
    expect(allows("plugin:event|unlisten", { event: "snapshot", eventId: 3 })).toBe(true)
    expect(allows("plugin:event|unlisten", { event: "snapshot" })).toBe(false)
  })

  const windowCalls = [
    "minimize",
    "toggle_maximize",
    "is_maximized",
    "start_dragging",
    "internal_toggle_maximize",
    "close",
  ]
  for (const call of windowCalls) {
    test(`plugin:window|${call} with and without a label`, () => {
      expect(allows(`plugin:window|${call}`, { label: "main" })).toBe(true)
      expect(allows(`plugin:window|${call}`, {})).toBe(true)
      expect(allows(`plugin:window|${call}`)).toBe(true)
      expect(allows(`plugin:window|${call}`, { label: "m".repeat(65) })).toBe(false)
      expect(allows(`plugin:window|${call}`, { label: "main", extra: 1 })).toBe(false)
    })
  }
})

describe("unknown and forged names", () => {
  const names: unknown[] = [
    "plugin:fs|read_file",
    "plugin:shell|execute",
    "plugin:opener|open_url",
    "plugin:clipboard-manager|write_text",
    "get_snapshot ",
    " get_snapshot",
    "GET_SNAPSHOT",
    "__proto__",
    "constructor",
    "toString",
    "hasOwnProperty",
    42,
    null,
    undefined,
  ]
  for (const name of names) {
    test(`${JSON.stringify(name) ?? "undefined"} is dropped`, () => {
      const payload = { cmd: name, payload: {} }
      const answer = hook(payload) as { then?: unknown }
      expect(answer).not.toBe(payload)
      expect(typeof answer.then).toBe("function")
    })
  }

  test("a payload that is not an object is dropped", () => {
    for (const payload of [null, undefined, "get_snapshot", 1, true]) {
      const answer = hook(payload) as { then?: unknown }
      expect(answer).not.toBe(payload)
      expect(typeof answer.then).toBe("function")
    }
  })

  test("every drop answers the same promise", () => {
    expect(hook({ cmd: "nope" })).toBe(hook({ cmd: "__proto__" }))
  })
})

describe("text", () => {
  const max = limit("register", "username")

  test("register's username at and over its limit", () => {
    expect(allows("register", { username: "u".repeat(max) })).toBe(true)
    expect(allows("register", { username: "" })).toBe(true)
    expect(allows("register", { username: "u".repeat(max + 1) })).toBe(false)
    expect(allows("register", { username: "u".repeat(1 << 20) })).toBe(false)
    expect(allows("register", { username: 5 })).toBe(false)
    expect(allows("register", { username: null })).toBe(false)
    expect(allows("register", {})).toBe(false)
  })

  test("copy_text at and over its limit", () => {
    const copy = limit("copy_text", "text")
    expect(allows("copy_text", { text: "c".repeat(copy) })).toBe(true)
    expect(allows("copy_text", { text: "c".repeat(copy + 1) })).toBe(false)
  })

  test("the limit counts UTF-16 units", () => {
    expect(allows("register", { username: "\u{1F600}".repeat(max / 2) })).toBe(true)
    expect(allows("register", { username: "\u{1F600}".repeat(max / 2 + 1) })).toBe(false)
  })
})

describe("index", () => {
  const max = limit("use_detected_game", "index")

  test("use_detected_game's index", () => {
    expect(allows("use_detected_game", { index: 0 })).toBe(true)
    expect(allows("use_detected_game", { index: max })).toBe(true)
    for (const index of [max + 1, -1, 1.5, NaN, Infinity, -Infinity, "1", null, [1], undefined]) {
      expect(allows("use_detected_game", { index })).toBe(false)
    }
  })
})

describe("name", () => {
  const names = shapeOf("revived_install", "mode").names ?? []

  test("revived_install's mode", () => {
    expect(names.length).toBeGreaterThan(0)
    for (const mode of names) expect(allows("revived_install", { mode })).toBe(true)
    const first = names[0] ?? ""
    for (const mode of [first.toUpperCase(), ` ${first}`, `${first} `, "__proto__", "", 0, null, undefined, [first]]) {
      expect(allows("revived_install", { mode })).toBe(false)
    }
  })
})

describe("object", () => {
  const fields = shapeOf("host_setup", "request").fields ?? {}
  const request = validArgs(fields)

  test("host_setup's request", () => {
    expect(allows("host_setup", { request })).toBe(true)
    expect(allows("host_setup", { request: { ...request, extra: "x" } })).toBe(false)
    const { name: _name, ...noName } = request
    expect(allows("host_setup", { request: noName })).toBe(false)
    expect(allows("host_setup", { request: null })).toBe(false)
    expect(allows("host_setup", { request: [request] })).toBe(false)
    expect(allows("host_setup", { request: "request" })).toBe(false)
    expect(allows("host_setup", {})).toBe(false)
  })

  test("an absent optional text field passes", () => {
    expect(fields.publicHost?.kind).toBe("optionalText")
    const { publicHost: _publicHost, ...noHost } = request
    expect(allows("host_setup", { request: noHost })).toBe(true)
    expect(allows("host_setup", { request: { ...request, publicHost: null } })).toBe(true)
  })

  test("a field named __proto__ is unknown", () => {
    const forged = JSON.parse(`{"name":"n","mode":"${String(request.mode)}","__proto__":{}}`)
    expect(allows("host_setup", { request: forged })).toBe(false)
  })

  test("host_set_multipliers: nullable, numbers within bounds", () => {
    const shape = shapeOf("host_set_multipliers", "multipliers")
    expect(shape.nullable).toBe(true)
    const xp = shape.fields?.xp ?? { kind: "number" }
    const min = xp.min ?? 0
    const max = xp.max ?? 0
    expect(allows("host_set_multipliers", { multipliers: null })).toBe(true)
    expect(allows("host_set_multipliers", { multipliers: { xp: min, rewards: max } })).toBe(true)
    expect(allows("host_set_multipliers", { multipliers: { xp: (min + max) / 2, rewards: 1.25 } })).toBe(true)
    for (const bad of [min - 0.01, max + 0.01, NaN, Infinity, "2", null]) {
      expect(allows("host_set_multipliers", { multipliers: { xp: bad, rewards: min } })).toBe(false)
    }
    expect(allows("host_set_multipliers", { multipliers: { xp: min } })).toBe(false)
    expect(allows("host_set_multipliers", {})).toBe(false)
  })
})

describe("settingsPatch", () => {
  test("set_settings' patch", () => {
    expect(shapeOf("set_settings", "patch").kind).toBe("settingsPatch")
    expect(allows("set_settings", { patch: {} })).toBe(true)
    for (const key of settingsKeys) {
      for (const value of [true, 2, "pt-BR", "s".repeat(16)]) {
        expect(allows("set_settings", { patch: { [key]: value } })).toBe(true)
      }
    }
    const key = settingsKeys[0] ?? "language"
    expect(allows("set_settings", { patch: { nope: true } })).toBe(false)
    expect(allows("set_settings", { patch: JSON.parse('{"__proto__":true}') })).toBe(false)
    expect(allows("set_settings", { patch: { [key]: "s".repeat(17) } })).toBe(false)
    expect(allows("set_settings", { patch: { [key]: null } })).toBe(false)
    expect(allows("set_settings", { patch: { [key]: {} } })).toBe(false)
    expect(allows("set_settings", { patch: [true] })).toBe(false)
    expect(allows("set_settings", { patch: null })).toBe(false)
    expect(allows("set_settings", { patch: "language" })).toBe(false)
  })
})

describe("channel", () => {
  test("subscribe_progress' onProgress", () => {
    expect(shapeOf("subscribe_progress", "onProgress").kind).toBe("channel")
    for (const onProgress of ["__CHANNEL__:0", "__CHANNEL__:4294967295", `__CHANNEL__:${"9".repeat(16)}`]) {
      expect(allows("subscribe_progress", { onProgress })).toBe(true)
    }
    const bad = [
      "__CHANNEL__:",
      `__CHANNEL__:${"9".repeat(17)}`,
      "__CHANNEL__:12a",
      "__CHANNEL__:-1",
      " __CHANNEL__:1",
      "__CHANNEL__:1\n",
      "__channel__:1",
      1,
      null,
      { id: 1 },
    ]
    for (const onProgress of bad) expect(allows("subscribe_progress", { onProgress })).toBe(false)
  })
})

describe("arguments", () => {
  test("absent or null count as no arguments", () => {
    expect(allows("get_snapshot")).toBe(true)
    expect(allows("get_snapshot", null)).toBe(true)
    expect(allows("get_snapshot", {})).toBe(true)
    expect(allows("register")).toBe(false)
    expect(allows("register", null)).toBe(false)
  })

  test("anything but an object refuses", () => {
    expect(allows("get_snapshot", [])).toBe(false)
    expect(allows("register", ["u"])).toBe(false)
    expect(allows("get_snapshot", "x")).toBe(false)
    expect(allows("get_snapshot", 0)).toBe(false)
  })

  test("an argument the table doesn't name refuses", () => {
    expect(allows("get_snapshot", { extra: 1 })).toBe(false)
    expect(allows("register", { username: "u", extra: 1 })).toBe(false)
  })
})

describe("a dropped call", () => {
  test("never settles", async () => {
    const dropped = hook({ cmd: "plugin:fs|read_file", payload: { path: "C:/" } }) as Promise<unknown>
    const timer = new Promise((resolve) => setTimeout(() => resolve("timer"), 20))
    expect(await Promise.race([dropped.then(() => "settled", () => "settled"), timer])).toBe("timer")
  })
})

describe("the isolation files", () => {
  const isolation = path.resolve(import.meta.dir, "../isolation")

  test("hook.js is strict ES5", () => {
    const source = readFileSync(path.join(isolation, "hook.js"), "utf8")
    expect(source).toContain('"use strict"')
    expect(source).not.toMatch(/=>|`|\b(let|const|class|async|await)\s/)
  })

  test("index.html loads the table, then the hook", () => {
    const html = readFileSync(path.join(isolation, "index.html"), "utf8")
    expect(html).toMatch(/<!doctype html>/i)
    expect(html).toContain('lang="en"')
    expect(html).toMatch(/charset="utf-8"/i)
    const scripts = [...html.matchAll(/<script src="([^"]+)"><\/script>/g)].map((m) => m[1])
    expect(scripts).toEqual(["commands.js", "hook.js"])
  })
})
