#!/usr/bin/env node
// RU3a: extracts every Express route the 1.14.7 backend and Director register and checks it against
// docs/inventory/routes-1.14.7.json. No dependencies; a static pass over the TypeScript source.
//
//   node scripts/check-route-inventory.mjs            check: exit 1 on a missing or stale entry, a router no service
//                                                     mounts, or a registration an earlier one always takes
//   node scripts/check-route-inventory.mjs --write    rewrite the JSON, keeping the group and side of known routes
//   node scripts/check-route-inventory.mjs --table    print the markdown tables (middleware, handler, statuses)
//   node scripts/check-route-inventory.mjs --dump     print the extracted routes as JSON

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const INVENTORY = "docs/inventory/routes-1.14.7.json";
const MARKDOWN = "docs/inventory/routes-1.14.7.md";

// Change these two and the paths in the JSON and the markdown together.
const BACKEND = "reference/1.14.7/ParadoxBackend";
const DIRECTOR = "reference/1.14.7/ParadoxDirector";

const SERVICES = [
    { service: "backend", dir: BACKEND, root: `${BACKEND}/src/app.ts`, app: "app" },
    { service: "director", dir: DIRECTOR, root: `${DIRECTOR}/src/app.ts`, app: "app" },
];

// Calls that hand a router to code the static pass can't follow (the optional p2p module is loaded through
// require() behind GetP2PExtension()). Inside `fn` in `target`, the function's first parameter is the router.
const HOOKS = [
    { file: `${BACKEND}/src/app.ts`, call: "Extension.http.mountEarlyRoutes", target: `${BACKEND}/src/p2p/index.ts`, fn: "mountEarlyRoutes" },
    { file: `${BACKEND}/src/app.ts`, call: "Extension.http.mountRoutes", target: `${BACKEND}/src/p2p/index.ts`, fn: "mountRoutes" },
    { file: `${BACKEND}/src/app.ts`, call: "MountRealmsModule", target: `${BACKEND}/src/realms/index.ts`, fn: "MountRealmsModule" },
    { file: `${DIRECTOR}/src/routes/matchmaker.ts`, call: "GetP2PExtension().mountMatchmakerRoutes", target: `${DIRECTOR}/src/p2p/index.ts`, fn: "mountMatchmakerRoutes" },
];

// Files whose routers no service mounts on purpose, each with its reason.
const UNREACHED_OK = [];

// Registrations that never answer, each with its reason: Express runs the first route that matches, and an
// earlier one that matches every path of a later one (and never calls next()) takes all its requests.
const SHADOWED_OK = [
    { key: "backend GET /dauntless-status", file: `${BACKEND}/src/routes/reforgedApi.ts`, reason: "system.ts registers it first and answers; the Reforged handler is dead code" },
    { key: "backend GET /playertreatments/", file: `${BACKEND}/src/routes/system.ts`, reason: "the same handler as /playertreatments, listed twice in one array" },
    { key: "backend POST /encountered-content/", file: `${BACKEND}/src/routes/progression.ts`, reason: "the same answer as /encountered-content, registered just before it" },
    { key: "backend POST /inventory/:characterId/:changeList", file: `${BACKEND}/src/routes/inventory.ts`, reason: "the regex route above it takes every such path, the empty character id too (inventory.ts's 2026-10-04 fix)" },
    { key: "backend GET /launcher/v1/runtime/:channel/:platform/download", file: `${BACKEND}/src/routes/launcherUpdates.ts`, reason: "GET /launcher/v1/runtime/:target/:channel/:platform takes it first: 400 for an unknown target, a manifest when the channel is a target name" },
];

const METHODS = ["get", "post", "put", "patch", "delete", "all", "head", "options"];
const GROUPS = {
    isles: [
        "auth", "reforged-api", "characters-loadouts", "inventory-reforge", "progression", "bounties-store",
        "matchmaking-parties", "chat", "game-data", "director",
    ],
    rifts: ["p2p", "realms", "guard"],
};

// ---------------------------------------------------------------------------------------------------------------
// Lexing: one kind per character, so brackets inside strings, templates, regexes and comments never count.

const CODE = 0, STR = 1, TPL = 2, RGX = 3, CMT = 4;
const REGEX_AFTER_WORD = new Set(["return", "typeof", "case", "do", "else", "in", "of", "new", "delete", "void", "throw", "yield", "await", "instanceof"]);

function lex(src) {
    const kind = new Uint8Array(src.length);
    const tplStack = []; // brace depth at each `${` of an open template
    let depth = 0, i = 0, lastSig = "", lastWord = "";
    const regexAllowed = () => lastSig === "" || "(,=:[!&|?{};+-*%<>~^".includes(lastSig) || REGEX_AFTER_WORD.has(lastWord);
    while (i < src.length) {
        const c = src[i], n = src[i + 1];
        if (c === "/" && n === "/") {
            while (i < src.length && src[i] !== "\n") kind[i++] = CMT;
            continue;
        }
        if (c === "/" && n === "*") {
            const end = src.indexOf("*/", i + 2);
            const stop = end < 0 ? src.length : end + 2;
            while (i < stop) kind[i++] = CMT;
            continue;
        }
        if (c === "'" || c === '"') {
            kind[i++] = STR;
            while (i < src.length && src[i] !== c && src[i] !== "\n") {
                if (src[i] === "\\") kind[i++] = STR;
                kind[i++] = STR;
            }
            kind[i++] = STR;
            lastSig = c; lastWord = "";
            continue;
        }
        if (c === "`" || (c === "}" && tplStack.length && tplStack[tplStack.length - 1] === depth)) {
            if (c === "}") tplStack.pop();
            kind[i++] = TPL;
            while (i < src.length && src[i] !== "`") {
                if (src[i] === "\\") { kind[i++] = TPL; kind[i++] = TPL; continue; }
                if (src[i] === "$" && src[i + 1] === "{") { kind[i++] = TPL; kind[i++] = TPL; tplStack.push(depth); break; }
                kind[i++] = TPL;
            }
            if (src[i] === "`") kind[i++] = TPL;
            lastSig = "`"; lastWord = "";
            continue;
        }
        if (c === "/" && regexAllowed()) {
            let inClass = false;
            kind[i++] = RGX;
            while (i < src.length && src[i] !== "\n") {
                const d = src[i];
                if (d === "\\") { kind[i++] = RGX; kind[i++] = RGX; continue; }
                if (d === "[") inClass = true;
                else if (d === "]") inClass = false;
                else if (d === "/" && !inClass) break;
                kind[i++] = RGX;
            }
            kind[i++] = RGX;
            while (/[a-z]/.test(src[i] ?? "")) kind[i++] = RGX;
            lastSig = "/"; lastWord = "";
            continue;
        }
        if (c === "{") depth++;
        else if (c === "}") depth--;
        if (/[A-Za-z0-9_$]/.test(c)) {
            let j = i;
            while (j < src.length && /[A-Za-z0-9_$]/.test(src[j])) j++;
            lastWord = src.slice(i, j); lastSig = "a";
            i = j;
            continue;
        }
        if (!/\s/.test(c)) { lastSig = c; lastWord = ""; }
        i++;
    }
    return kind;
}

// Comments blanked (newlines kept) so regexes over the code never match commented-out routes.
function maskComments(src, kind) {
    let out = "";
    for (let i = 0; i < src.length; i++) out += kind[i] === CMT && src[i] !== "\n" ? " " : src[i];
    return out;
}

function matchClose(f, open) {
    const pairs = { "(": ")", "[": "]", "{": "}" };
    const stack = [pairs[f.code[open]]];
    for (let i = open + 1; i < f.code.length; i++) {
        if (f.kind[i] !== CODE) continue;
        const c = f.code[i];
        if (pairs[c]) stack.push(pairs[c]);
        else if (c === stack[stack.length - 1]) { stack.pop(); if (!stack.length) return i; }
    }
    return -1;
}

// Top-level arguments of the call whose `(` is at `open`: [{ text, start, end }].
function splitArgs(f, open) {
    const close = matchClose(f, open);
    const args = [];
    let start = open + 1, depth = 0;
    for (let i = open + 1; i < close; i++) {
        if (f.kind[i] !== CODE) continue;
        const c = f.code[i];
        if ("([{".includes(c)) depth++;
        else if (")]}".includes(c)) depth--;
        else if (c === "," && depth === 0) { args.push({ start, end: i }); start = i + 1; }
    }
    args.push({ start, end: close });
    return {
        close,
        args: args.map((a) => ({ ...a, text: f.code.slice(a.start, a.end).trim() })).filter((a) => a.text.length > 0),
    };
}

const lineOf = (f, idx) => f.code.slice(0, idx).split("\n").length;

// ---------------------------------------------------------------------------------------------------------------
// Files: imports, string constants, functions and router variables.

const files = new Map();

function resolveModule(fromRel, spec) {
    if (!spec.startsWith(".")) return undefined;
    const base = path.posix.join(path.posix.dirname(fromRel), spec).replace(/\.(js|cjs|mjs)$/, "");
    for (const cand of [`${base}.ts`, `${base}/index.ts`, `${base}.js`, `${base}/index.js`]) {
        if (fs.existsSync(path.join(ROOT, cand))) return cand;
    }
    return undefined;
}

function loadFile(rel) {
    if (files.has(rel)) return files.get(rel);
    const src = fs.readFileSync(path.join(ROOT, rel), "utf8").replace(/\r\n/g, "\n");
    const kind = lex(src);
    const f = { rel, src, kind, code: maskComments(src, kind), imports: new Map(), consts: new Map(), functions: [], routerVars: [] };
    files.set(rel, f);

    for (const m of f.code.matchAll(/import\s+(?:type\s+)?(?:(\w+)\s*,?\s*)?(?:\{([^}]*)\})?\s*from\s*["']([^"']+)["']/g)) {
        const target = resolveModule(rel, m[3]);
        if (m[1]) f.imports.set(m[1], { file: target, name: "default" });
        for (const part of (m[2] ?? "").split(",")) {
            const [orig, local] = part.replace(/^\s*type\s+/, "").split(/\s+as\s+/).map((s) => s.trim());
            if (orig) f.imports.set(local ?? orig, { file: target, name: orig });
        }
    }
    for (const m of f.code.matchAll(/\b(?:const|let)\s+(\w+)\s*(?::\s*string\s*)?=\s*(["'`])((?:(?!\2)[^\\\n]|\\.)*)\2\s*;/g)) {
        f.consts.set(m[1], m[3]);
    }
    // Functions: `function N(`, `N = (...) =>`, `N: (...) =>` and method shorthand `N(...) {`.
    const fnRe = /(?:\bfunction\s+(\w+)\s*\(|\b(\w+)\s*[:=]\s*(?:async\s+)?(?:\(|function\s*\()|^\s*(?:async\s+)?(\w+)\s*\()/gm;
    for (const m of f.code.matchAll(fnRe)) {
        const name = m[1] ?? m[2] ?? m[3];
        if (!name || ["if", "for", "while", "switch", "catch", "return"].includes(name) || f.kind[m.index] !== CODE) continue;
        const open = f.code.indexOf("(", m.index + m[0].length - 1);
        if (open < 0) continue;
        const close = matchClose(f, open);
        if (close < 0) continue;
        const after = f.code.slice(close + 1, close + 200);
        const body = /^\s*(?::\s*[^={]+?)?\s*(=>\s*)?\{/.exec(after);
        if (!body) continue;
        const bodyOpen = close + 1 + body[0].length - 1;
        const bodyClose = matchClose(f, bodyOpen);
        if (bodyClose < 0) continue;
        const params = f.code.slice(open + 1, close).split(",").map((p) => p.trim().split(/[\s:=?]/)[0]).filter(Boolean);
        f.functions.push({ name, params, start: bodyOpen, end: bodyClose, line: lineOf(f, m.index) });
    }
    for (const m of f.code.matchAll(/\b(?:export\s+)?(?:const|let)\s+(\w+)\s*(?::\s*[\w.]+\s*)?=\s*(?:express\.)?(Router|express)\([^)]*\)/g)) {
        const fn = enclosingFunction(f, m.index);
        f.routerVars.push({ name: m[1], fn: fn?.name, id: `${rel}#${fn ? `${fn.name}/` : ""}${m[1]}`, scope: fn ?? { start: 0, end: f.code.length } });
    }
    return f;
}

function enclosingFunction(f, idx) {
    let best;
    for (const fn of f.functions) if (fn.start < idx && idx < fn.end && (!best || fn.start > best.start)) best = fn;
    return best;
}

function findFunction(f, name) {
    return f.functions.find((fn) => fn.name === name);
}

// The router a factory function returns: its own router variable.
function factoryRouter(f, fnName) {
    const fn = findFunction(f, fnName);
    if (!fn) return undefined;
    return f.routerVars.find((r) => r.fn === fnName)?.id;
}

// A file-level name for a router: a router variable, an alias of a factory call, or an import of either.
function exportedRouter(rel, name, seen = new Set()) {
    if (!rel || seen.has(`${rel}#${name}`)) return undefined;
    seen.add(`${rel}#${name}`);
    const f = loadFile(rel);
    const own = f.routerVars.find((r) => r.name === name && !r.fn);
    if (own) return own.id;
    const alias = new RegExp(`\\b(?:const|let)\\s+${name}\\s*=\\s*(\\w+)\\(`).exec(f.code);
    if (alias) {
        const local = factoryRouter(f, alias[1]);
        if (local) return local;
        const imp = f.imports.get(alias[1]);
        if (imp?.file) return factoryRouter(loadFile(imp.file), imp.name);
    }
    const imp = f.imports.get(name);
    if (imp?.file) return exportedRouter(imp.file, imp.name, seen);
    return undefined;
}

// Resolve a `.use()` argument to a router id, or undefined when it is middleware.
function argRouter(f, at, text) {
    let m = /^\(?\s*require\(\s*["']([^"']+)["']\s*\)[\s\S]*?\.\s*(\w+)$/.exec(text);
    if (m) return exportedRouter(resolveModule(f.rel, m[1]), m[2]);
    m = /^(\w+)\s*\(/.exec(text);
    if (m) {
        const local = factoryRouter(f, m[1]);
        if (local) return local;
        const imp = f.imports.get(m[1]);
        if (imp?.file) return factoryRouter(loadFile(imp.file), imp.name);
        return undefined;
    }
    if (/^\w+$/.test(text)) {
        const scoped = scopedRouterVar(f, at, text);
        if (scoped) return scoped;
        return exportedRouter(f.rel, text);
    }
    return undefined;
}

function scopedRouterVar(f, at, name) {
    let best;
    for (const r of f.routerVars) {
        if (r.name === name && r.scope.start <= at && at <= r.scope.end && (!best || r.scope.start > best.scope.start)) best = r;
    }
    return best?.id;
}

// ---------------------------------------------------------------------------------------------------------------
// Paths

function evalPath(f, text) {
    let m = /^(["'])((?:(?!\1)[^\\]|\\.)*)\1$/.exec(text);
    if (m) return [m[2]];
    m = /^`([^`]*)`$/.exec(text);
    if (m) {
        let unresolved = false;
        const out = m[1].replace(/\$\{\s*(\w+)\s*\}/g, (_, name) => {
            const v = f.consts.get(name);
            if (v === undefined) unresolved = true;
            return v ?? `\${${name}}`;
        });
        return unresolved ? undefined : [out];
    }
    if (/^\/.*\/[a-z]*$/s.test(text) && f.kind[f.code.indexOf(text)] === RGX) return [text];
    if (text.startsWith("[") && text.endsWith("]")) {
        const parts = text.slice(1, -1).split(",").map((s) => s.trim()).filter(Boolean);
        const out = parts.map((p) => evalPath(f, p));
        return out.every(Boolean) ? out.flat() : undefined;
    }
    if (/^\w+$/.test(text) && f.consts.has(text)) return [f.consts.get(text)];
    return undefined;
}

const isRegexPath = (p) => /^\/.*\/[a-z]*$/s.test(p) && p.length > 2 && (p.includes("^") || p.includes("\\/"));

function joinPath(prefix, p) {
    if (prefix === "" || prefix === "/") return p;
    if (isRegexPath(p)) return `${prefix} + ${p}`;
    if (p === "/") return prefix;
    return prefix.replace(/\/$/, "") + p;
}

// ---------------------------------------------------------------------------------------------------------------
// Registrations: routes and mounts per router id.

const routers = new Map(); // id -> { routes: [], mounts: [], middleware: [] }
const routerOf = (id) => {
    if (!routers.has(id)) routers.set(id, { routes: [], mounts: [], middleware: [] });
    return routers.get(id);
};
const problems = [];
const registered = new Set(); // a file scanned twice (hooks, the unreached pass) registers once

// Every `recv.method(` in the file where recv names a router in scope (or a hooked parameter).
function scanRegistrations(rel, bindings = []) {
    const f = loadFile(rel);
    const re = new RegExp(`\\b(\\w+)\\s*\\.\\s*(${[...METHODS, "use", "route"].join("|")})\\s*\\(`, "g");
    for (const m of f.code.matchAll(re)) {
        if (f.kind[m.index] !== CODE) continue;
        const at = m.index;
        const bound = bindings.find((b) => b.name === m[1] && b.start <= at && at <= b.end);
        const id = bound?.router ?? scopedRouterVar(f, at, m[1]);
        if (!id || registered.has(`${id}|${rel}:${at}`)) continue;
        registered.add(`${id}|${rel}:${at}`);
        const open = at + m[0].length - 1;
        const { args } = splitArgs(f, open);
        const router = routerOf(id);
        const line = lineOf(f, at);
        const condition = conditionOf(f, at);
        if (m[2] === "route") {
            problems.push(`${rel}:${line}: ${m[1]}.route() isn't read by the static pass; register the methods directly or teach the script`);
            continue;
        }
        if (m[2] === "use") {
            let rest = args, prefix = "";
            const p = args.length ? evalPath(f, args[0].text) : undefined;
            if (p) { prefix = p[0]; rest = args.slice(1); }
            for (const a of rest) {
                const child = argRouter(f, a.start, a.text);
                if (child) router.mounts.push({ prefix, child, file: rel, line, condition });
                else router.middleware.push({ prefix, text: a.text, name: middlewareName(a.text), file: rel, line, order: router.routes.length });
            }
            continue;
        }
        if (!args.length) continue;
        const paths = evalPath(f, args[0].text);
        if (!paths) {
            problems.push(`${rel}:${line}: ${m[1]}.${m[2]}(${args[0].text.slice(0, 60)}) has a path the static pass can't read`);
            continue;
        }
        const handlerArgs = args.slice(1);
        for (const p of paths) {
            router.routes.push({
                method: m[2].toUpperCase(), path: p, file: rel, line, condition,
                middleware: handlerArgs.slice(0, -1).map((a) => middlewareName(a.text)),
                // The app's own middleware is described once in the markdown, not per route.
                routerMiddleware: id.endsWith("#app") ? [] : router.middleware
                    .filter((mw) => mw.order <= router.routes.length && (mw.prefix === "" || p.startsWith(mw.prefix)))
                    .map((mw) => mw.name),
                handlerArgs,
            });
        }
    }
}

function middlewareName(text) {
    const m = /^(?:async\s+)?\(?\s*\w*\s*(?:,|\)|=>)/.test(text) || text.startsWith("function") ? null : /^([\w.]+)/.exec(text);
    return m ? m[1] + (text.includes("(") ? "()" : "") : "inline";
}

function conditionOf(f, at) {
    const lineStart = f.code.lastIndexOf("\n", at) + 1;
    const before = f.code.slice(lineStart, at);
    const m = /^\s*(?:\}\s*else\s+)?if\s*\((.*)\)\s*\{?\s*$/.exec(before);
    if (m) return m[1].trim();
    // Inside an `if (...) {` block opened on an earlier line of the same function.
    const fn = enclosingFunction(f, at);
    const from = fn ? fn.start : 0;
    const re = /\bif\s*\(/g;
    re.lastIndex = from;
    let found;
    for (let m2; (m2 = re.exec(f.code)) && m2.index < at;) {
        if (f.kind[m2.index] !== CODE) continue;
        const open = m2.index + m2[0].length - 1;
        const close = matchClose(f, open);
        const brace = /^\s*\{/.exec(f.code.slice(close + 1));
        if (!brace) continue;
        const bodyOpen = close + 1 + brace[0].length - 1;
        const bodyClose = matchClose(f, bodyOpen);
        if (bodyOpen < at && at < bodyClose) found = f.code.slice(open + 1, close).trim();
    }
    return found;
}

// ---------------------------------------------------------------------------------------------------------------
// Walk from each service's app, through mounts and hooks.

function extract() {
    const hookBindings = new Map(); // target file -> bindings
    const reached = new Set();
    const visited = new Set();
    const routes = [];
    for (const svc of SERVICES) {
        const rootFile = loadFile(svc.root);
        const rootVar = rootFile.routerVars.find((r) => r.name === svc.app && !r.fn);
        if (!rootVar) { problems.push(`${svc.root}: no \`${svc.app} = express()\``); continue; }
        const scanned = new Set();
        const scan = (rel, bindings = []) => {
            const key = `${rel}|${bindings.map((b) => `${b.router}@${b.start}`).join(",")}`;
            if (scanned.has(key)) return;
            scanned.add(key);
            reached.add(rel);
            scanRegistrations(rel, bindings);
        };
        // Hooks: bind the target function's first parameter to the router passed at the call site.
        const applyHooks = (rel) => {
            const f = loadFile(rel);
            for (const h of HOOKS.filter((x) => x.file === rel)) {
                const at = f.code.indexOf(`${h.call}(`);
                if (at < 0) { problems.push(`${rel}: hook call ${h.call}( not found; update HOOKS`); continue; }
                const { args } = splitArgs(f, at + h.call.length);
                const router = argRouter(f, args[0].start, args[0].text) ?? scopedRouterVar(f, at, args[0].text);
                const target = loadFile(h.target);
                const fn = findFunction(target, h.fn);
                if (!router || !fn) { problems.push(`${rel}: hook ${h.call} -> ${h.target}:${h.fn} unresolved; update HOOKS`); continue; }
                const binding = { name: fn.params[0], start: fn.start, end: fn.end, router };
                hookBindings.set(h.target, [...(hookBindings.get(h.target) ?? []), binding]);
                scan(h.target, [binding]);
                applyHooks(h.target);
            }
        };
        const queue = [svc.root];
        const done = new Set();
        while (queue.length) {
            const rel = queue.shift();
            if (done.has(rel)) continue;
            done.add(rel);
            scan(rel, hookBindings.get(rel) ?? []);
            applyHooks(rel);
            for (const [id, r] of routers) {
                if (done.has(id.split("#")[0])) for (const mnt of r.mounts) queue.push(mnt.child.split("#")[0]);
            }
        }
        // Collect routes with their mount prefixes.
        const visit = (id, prefix, chain, conditions) => {
            const r = routers.get(id);
            if (!r) return;
            visited.add(id);
            for (const rt of r.routes) {
                routes.push({ service: svc.service, ...rt, path: joinPath(prefix, rt.path), router: id, chain, conditions: [...conditions, rt.condition].filter(Boolean) });
            }
            for (const mnt of r.mounts) {
                visit(mnt.child, joinPath(prefix, mnt.prefix || "/") === "/" ? "" : joinPath(prefix, mnt.prefix || ""), [...chain, mnt], [...conditions, mnt.condition].filter(Boolean));
            }
        };
        visit(rootVar.id, "", [], []);
    }
    // Routers the walk never visited: in files it didn't reach, or declared and never mounted.
    for (const svc of SERVICES) {
        for (const rel of listTs(`${svc.dir}/src`)) if (!reached.has(rel) && loadFile(rel).routerVars.length) scanRegistrations(rel);
    }
    for (const [id, r] of routers) {
        const rel = id.split("#")[0];
        if (visited.has(id) || !r.routes.length || UNREACHED_OK.some((u) => u.file === rel)) continue;
        problems.push(`${id}: ${r.routes.length} route(s) on a router no service mounts; mount it, add a HOOK or list it in UNREACHED_OK`);
    }
    return routes;
}

function listTs(dir) {
    const out = [];
    const walk = (rel) => {
        for (const e of fs.readdirSync(path.join(ROOT, rel), { withFileTypes: true })) {
            const r = `${rel}/${e.name}`;
            if (e.isDirectory()) { if (e.name !== "node_modules" && e.name !== "build") walk(r); }
            else if (/\.ts$/.test(e.name) && !/\.test\.ts$|\.d\.ts$/.test(e.name)) out.push(r);
        }
    };
    walk(dir);
    return out;
}

// ---------------------------------------------------------------------------------------------------------------
// Handler details for the markdown: what it calls and which statuses its code can set.

const STATUS_RES = [/\bstatus\(\s*(\d{3})\s*\)/g, /\bsendStatus\(\s*(\d{3})\s*\)/g, /\bstatusCode\s*=\s*(\d{3})\b/g, /\bstatus:\s*(\d{3})\b/g, /\b[A-Z]\w*\(\s*([1-5]\d\d)\s*,/g];

function statusesIn(text, out) {
    for (const re of STATUS_RES) for (const m of text.matchAll(re)) out.add(m[1]);
    if (/\.(json|send|end|sendFile)\(/.test(text)) out.add("200");
    if (/\.redirect\(/.test(text)) out.add("302");
    if (/\bstatus\(\s*[A-Za-z_]/.test(text)) out.add("dyn");
    if (/\b(SendLauncherError|LauncherApiError)\b/.test(text)) out.add("LE");
}

function functionText(f, name) {
    const fn = findFunction(f, name);
    if (fn) return { f, text: f.code.slice(fn.start, fn.end + 1), fn };
    const imp = f.imports.get(name);
    if (imp?.file) {
        const g = loadFile(imp.file);
        const gf = findFunction(g, imp.name);
        if (gf) return { f: g, text: g.code.slice(gf.start, gf.end + 1), fn: gf };
    }
    return undefined;
}

function describe(rt) {
    const f = loadFile(rt.file);
    const statuses = new Set();
    const calls = [];
    const last = rt.handlerArgs[rt.handlerArgs.length - 1];
    let handler = "inline";
    const spread = last && /^\.\.\.(\w+)$/.exec(last.text);
    if (spread) {
        // `...Handlers`: a const holding the array a factory returns.
        const factory = new RegExp(`\\b(?:const|let)\\s+${spread[1]}\\s*=\\s*(\\w+)\\(`).exec(f.code)?.[1];
        const t = factory && functionText(f, factory);
        if (t) statusesIn(t.text, statuses);
        const order = (s) => ({ LE: 998, dyn: 999 })[s] ?? Number(s);
        return { handler: t ? `${t.f.rel}:${t.fn.line} ${factory}` : spread[1], calls: [], statuses: [...statuses].sort((a, b) => order(a) - order(b)) };
    }
    const seen = new Set();
    const follow = (text, from, depth) => {
        statusesIn(text, statuses);
        if (depth === 0) return;
        for (const m of text.matchAll(/\b([A-Za-z_]\w*)\s*\(/g)) {
            if (seen.has(m[1])) continue;
            seen.add(m[1]);
            const t = functionText(from, m[1]);
            if (t) {
                if (depth === 2 && t.f.rel !== rt.file) calls.push(`${path.posix.basename(t.f.rel)}:${m[1]}`);
                follow(t.text, t.f, depth - 1);
            }
        }
    };
    if (last) {
        const named = /^([\w.]+)(\(|$)/.exec(last.text);
        if (named && !/=>/.test(last.text.split("\n")[0]) && !last.text.startsWith("async") && !last.text.startsWith("function")) {
            const t = functionText(f, named[1].split(".")[0]);
            handler = t ? `${t.f.rel}:${t.fn.line} ${named[1]}` : named[1];
            if (t) follow(t.text, t.f, 1);
            else statusesIn(last.text, statuses);
        } else {
            follow(last.text, f, 2);
        }
    }
    for (const a of rt.handlerArgs.slice(0, -1)) {
        const name = /^([\w]+)/.exec(a.text)?.[1];
        const t = name && functionText(f, name);
        if (t) statusesIn(t.text, statuses);
        else statusesIn(a.text, statuses);
    }
    const order = (s) => ({ LE: 998, dyn: 999 })[s] ?? Number(s);
    return { handler, calls: calls.slice(0, 4), statuses: [...statuses].sort((a, b) => order(a) - order(b)) };
}

// ---------------------------------------------------------------------------------------------------------------
// A new route's side and group for --write, to review: an isles route gets no group, so the check fails until one
// of RU5's is picked by hand.

function suggest(svc, p, file) {
    if (file.includes("/realms/") || file.endsWith("/realmHosting.ts")) return ["rifts", "realms"];
    if (/guard/i.test(p) || file.endsWith("/launcherGuard.ts")) return ["rifts", "guard"];
    if (file.includes("/p2p/")) return ["rifts", "p2p"];
    return ["isles", svc === "director" ? "director" : ""];
}

// ---------------------------------------------------------------------------------------------------------------

const key = (r) => `${r.service} ${r.method} ${r.path}`;

// Express 5's matching: `:name` is one non-empty segment, case-insensitive, optional trailing slash.
function matcherOf(p) {
    if (isRegexPath(p)) {
        const m = /^\/(.*)\/([a-z]*)$/s.exec(p);
        return new RegExp(m[1], m[2]);
    }
    const body = p.replace(/\/$/, "").split(/(:\w+)/).map((s) => (s.startsWith(":") ? "[^/]+" : s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))).join("");
    return new RegExp(`^${body}/?$`, "i");
}

function callsNext(r) {
    const last = r.handlerArgs[r.handlerArgs.length - 1];
    if (!last) return false;
    const f = loadFile(r.file);
    const named = /^(?:\.\.\.)?(\w+)$/.exec(last.text);
    const text = named ? (functionText(f, named[1])?.text ?? "") : last.text;
    return /\bnext\s*\(/.test(text);
}

function main() {
    const mode = process.argv[2] ?? "--check";
    const routes = extract();
    const unique = new Map();
    for (const r of routes) {
        const k = key(r);
        if (unique.has(k)) {
            const prev = unique.get(k);
            prev.duplicates = [...(prev.duplicates ?? []), `${r.file}:${r.line}`];
        } else unique.set(k, r);
    }

    if (mode === "--dump") {
        process.stdout.write(JSON.stringify([...unique.values()].map((r) => ({ ...r, handlerArgs: undefined, chain: r.chain.map((c) => c.prefix || "/") })), null, 2) + "\n");
        return;
    }

    const invPath = path.join(ROOT, INVENTORY);
    const inventory = fs.existsSync(invPath) ? JSON.parse(fs.readFileSync(invPath, "utf8")) : [];
    const known = new Map(inventory.map((e) => [key(e), e]));

    if (mode === "--write") {
        const out = [...unique.values()].map((r) => {
            const prev = known.get(key(r));
            const [side, group] = prev ? [prev.side, prev.group] : suggest(r.service, r.path, r.file);
            return { service: r.service, method: r.method, path: r.path, file: r.file, group, side };
        });
        fs.mkdirSync(path.dirname(invPath), { recursive: true });
        fs.writeFileSync(invPath, JSON.stringify(out, null, 2) + "\n");
        console.log(`wrote ${out.length} routes to ${INVENTORY}`);
        for (const p of problems) console.log(`problem: ${p}`);
        return;
    }

    if (mode === "--table") {
        for (const r of unique.values()) {
            const d = describe(r);
            const e = known.get(key(r)) ?? {};
            const mw = [...r.routerMiddleware.map((x) => `${x} (router)`), ...r.middleware];
            const cond = r.conditions.length ? r.conditions.join(" && ") : "";
            console.log(JSON.stringify({ key: key(r), file: r.file, line: r.line, side: e.side, group: e.group, middleware: mw, handler: d.handler, calls: d.calls, statuses: d.statuses, conditions: cond, chain: r.chain.map((c) => `${c.file}:${c.line} ${c.prefix || "/"}`), duplicates: r.duplicates }));
        }
        for (const p of problems) console.error(`problem: ${p}`);
        return;
    }

    // --check
    const errors = [...problems];
    const shadowed = (k, at) => {
        if (!SHADOWED_OK.some((s) => s.key === k && at.startsWith(`${s.file}:`))) {
            errors.push(`${k} at ${at} never answers: an earlier registration matches first; remove it or list it in SHADOWED_OK`);
        }
    };
    const firstByMatch = new Map();
    for (const r of unique.values()) {
        for (const d of r.duplicates ?? []) shadowed(key(r), d);
        // Express matches string paths case-insensitively and with an optional trailing slash.
        const match = isRegexPath(r.path) ? key(r) : `${r.service} ${r.method} ${r.path.toLowerCase().replace(/(.)\/$/, "$1")}`;
        if (firstByMatch.has(match)) { shadowed(key(r), `${r.file}:${r.line}`); continue; }
        firstByMatch.set(match, r);
        // An earlier pattern (a regex or :params) that matches every path of this one, and never calls next().
        if (isRegexPath(r.path)) continue;
        const sample = r.path.replace(/:\w+/g, "x");
        for (const prev of firstByMatch.values()) {
            if (prev === r || prev.service !== r.service || prev.method !== r.method || callsNext(prev)) continue;
            if (matcherOf(prev.path).test(sample)) { shadowed(key(r), `${r.file}:${r.line}`); break; }
        }
    }
    const mdPath = path.join(ROOT, MARKDOWN);
    if (!fs.existsSync(mdPath)) errors.push(`${MARKDOWN} is missing`);
    const md = fs.existsSync(mdPath) ? fs.readFileSync(mdPath, "utf8") : "";
    for (const r of unique.values()) {
        const e = known.get(key(r));
        if (!e) { errors.push(`missing from ${INVENTORY}: ${key(r)} (${r.file}:${r.line})`); continue; }
        if (e.file !== r.file) errors.push(`${key(r)}: file is ${r.file}, the inventory says ${e.file}`);
    }
    for (const e of inventory) {
        if (!unique.has(key(e))) errors.push(`stale entry in ${INVENTORY}: ${key(e)}`);
        const groups = GROUPS[e.side];
        if (!groups) errors.push(`${key(e)}: side must be isles or rifts`);
        else if (!groups.includes(e.group)) errors.push(`${key(e)}: group '${e.group}' is not one of ${e.side}'s (${groups.join(", ")})`);
        if (md && !md.includes(`| ${e.method} | \`${e.path.replace(/\|/g, "\\|")}\` |`)) errors.push(`${key(e)}: no row in ${MARKDOWN}`);
    }
    if (errors.length) {
        for (const e of errors) console.error(e);
        console.error(`\n${errors.length} problem(s). Add new routes with --write, then review their group and side by hand.`);
        process.exit(1);
    }
    console.log(`route inventory ok: ${unique.size} routes (${SERVICES.map((s) => `${s.service} ${[...unique.values()].filter((r) => r.service === s.service).length}`).join(", ")})`);
}

main();
