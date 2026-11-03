import type { BlacklistTerm } from "@/lib/db/schema";

/**
 * The chat filter's matcher. The backend enforces the same list (GET /api/ingest/chat-filter); the
 * dashboard runs it too, on arrival and in the tester, so both agree on what a term catches.
 */

export type FilterAction = BlacklistTerm["action"];
export type Verdict = "allowed" | "flagged" | "masked" | "blocked";

export interface CompiledTerm {
  id: number;
  action: FilterAction;
  channels: string[] | null;
  regex: RegExp;
}

export interface Match {
  termId: number;
  action: FilterAction;
  start: number;
  end: number;
}

const ESCAPE = /[.*+?^${}()|[\]\\]/g;
// Look-alikes players use to slip past a word list: 0 for o, @ for a and so on.
const LEET: Record<string, string> = {
  a: "[a@4]",
  e: "[e3]",
  i: "[i1!|]",
  o: "[o0]",
  s: "[s5$]",
  t: "[t7]",
  l: "[l1|]",
};

/** A pattern in one of three kinds, as a case-insensitive global RegExp; throws on a bad regex. */
export function compilePattern(pattern: string, matchType: BlacklistTerm["matchType"]): RegExp {
  if (matchType === "regex") return new RegExp(pattern, "giu");
  const body = [...pattern.toLowerCase()]
    .map((ch) => LEET[ch] ?? ch.replace(ESCAPE, "\\$&"))
    .join("[\\s._-]*");
  return matchType === "word"
    ? new RegExp(`(?<![\\p{L}\\p{N}])${body}(?![\\p{L}\\p{N}])`, "giu")
    : new RegExp(body, "giu");
}

export function validatePattern(pattern: string, matchType: BlacklistTerm["matchType"]): boolean {
  if (!pattern.trim() || pattern.length > 200) return false;
  try {
    compilePattern(pattern, matchType);
    return true;
  } catch {
    return false;
  }
}

export function compile(
  terms: Pick<BlacklistTerm, "id" | "pattern" | "matchType" | "action" | "channels" | "enabled">[],
): CompiledTerm[] {
  const out: CompiledTerm[] = [];
  for (const t of terms) {
    if (!t.enabled) continue;
    try {
      out.push({
        id: t.id,
        action: t.action,
        channels: t.channels ?? null,
        regex: compilePattern(t.pattern, t.matchType),
      });
    } catch {
      // A term that no longer compiles is skipped rather than failing every message.
    }
  }
  return out;
}

export function findMatches(text: string, terms: CompiledTerm[], channel?: string): Match[] {
  const matches: Match[] = [];
  for (const term of terms) {
    if (channel && term.channels && !term.channels.includes(channel)) continue;
    term.regex.lastIndex = 0;
    for (const m of text.matchAll(term.regex)) {
      if (m[0].length === 0) continue;
      matches.push({
        termId: term.id,
        action: term.action,
        start: m.index,
        end: m.index + m[0].length,
      });
    }
  }
  return matches.sort((a, b) => a.start - b.start);
}

const SEVERITY: Record<FilterAction, number> = { flag: 1, mask: 2, block: 3 };
const VERDICT: Record<FilterAction, Verdict> = {
  flag: "flagged",
  mask: "masked",
  block: "blocked",
};

export function verdictOf(matches: Match[]): Verdict {
  let worst: FilterAction | null = null;
  for (const m of matches) if (!worst || SEVERITY[m.action] > SEVERITY[worst]) worst = m.action;
  return worst ? VERDICT[worst] : "allowed";
}

/** The text a masked message shows: each masked span replaced by asterisks of its length. */
export function maskText(text: string, matches: Match[]): string {
  const chars = [...text];
  for (const m of matches) {
    if (m.action === "flag") continue;
    for (let i = m.start; i < m.end && i < chars.length; i++)
      if (!/\s/.test(chars[i]!)) chars[i] = "*";
  }
  return chars.join("");
}

/** Splits text into plain and matched runs, for highlighting in the log. */
export function segments(text: string, matches: Match[]): { text: string; hit: boolean }[] {
  const out: { text: string; hit: boolean }[] = [];
  let at = 0;
  for (const m of matches) {
    if (m.start < at) continue;
    if (m.start > at) out.push({ text: text.slice(at, m.start), hit: false });
    out.push({ text: text.slice(m.start, m.end), hit: true });
    at = m.end;
  }
  if (at < text.length) out.push({ text: text.slice(at), hit: false });
  return out;
}
