import type { AuditQuery } from "@/lib/audit/query";
import type { ChatQuery } from "@/lib/chat/store";
import type { AuditEvent, ChatMessage } from "@/lib/db/schema";

/** One parser per list page, shared by the page and its CSV export so both read the URL alike. */

const RANGES: Record<string, number> = {
  "1h": 3_600_000,
  "24h": 86_400_000,
  "7d": 7 * 86_400_000,
  "30d": 30 * 86_400_000,
};

export type SearchParams = Record<string, string | string[] | undefined>;

function get(p: URLSearchParams | SearchParams, key: string): string | undefined {
  if (p instanceof URLSearchParams) return p.get(key) ?? undefined;
  const v = p[key];
  return (Array.isArray(v) ? v[0] : v) || undefined;
}

export function rangeStart(range: string | undefined): number | undefined {
  return range && RANGES[range] ? Date.now() - RANGES[range] : undefined;
}

export function chatQueryFromParams(
  p: URLSearchParams | SearchParams,
): ChatQuery & { range?: string } {
  const channel = get(p, "channel");
  const range = get(p, "range");
  return {
    userId: get(p, "user"),
    name: get(p, "name"),
    roomId: get(p, "room"),
    // Direct messages are never a filter here: readDirectMessages is their only, audited, way out.
    channel: ["city", "party", "guild", "system"].includes(channel ?? "")
      ? (channel as ChatMessage["channel"])
      : undefined,
    text: get(p, "q"),
    flagged: get(p, "flagged") === "1",
    from: rangeStart(range),
    before: Number(get(p, "before")) || undefined,
    range,
  };
}

const TARGETS = ["player", "report", "chat_filter", "filter_term", "staff", "settings", "session"];

export function auditQueryFromParams(
  p: URLSearchParams | SearchParams,
): AuditQuery & { range?: string } {
  const target = get(p, "target");
  const range = get(p, "range");
  return {
    actorId: Number(get(p, "actor")) || undefined,
    targetType: TARGETS.includes(target ?? "") ? (target as AuditEvent["targetType"]) : undefined,
    targetId: get(p, "id"),
    action: get(p, "action"),
    text: get(p, "q"),
    from: rangeStart(range),
    revertibleOnly: get(p, "revertible") === "1",
    page: Number(get(p, "page")) || 1,
    range,
  };
}
