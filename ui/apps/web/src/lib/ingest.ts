import { createHash, timingSafeEqual } from "node:crypto";
import type { IncomingChat } from "@/lib/chat/store";
import { config } from "@/lib/config";
import { REPORT_CATEGORIES } from "@/lib/reports/categories";
import type { NewReport } from "@/lib/reports/store";

/**
 * The machine side: the backend posts chat and reports here and reads the chat filter. One bearer key,
 * compared by digest so neither its length nor a prefix shows in the timing.
 */
export function ingestAuthorized(request: Request): boolean {
  const key = config.ingestKey;
  if (!key || key.length < 32) return false;
  const given = request.headers.get("authorization")?.replace(/^Bearer\s+/i, "") ?? "";
  const a = createHash("sha256").update(given).digest();
  const b = createHash("sha256").update(key).digest();
  return timingSafeEqual(a, b);
}

const CHANNELS = new Set(["city", "party", "guild", "direct", "system"]);
const str = (v: unknown, max: number) =>
  typeof v === "string" && v.length > 0 && v.length <= max ? v : undefined;

/** Parses one chat line; anything malformed is dropped, not stored half-right. */
export function parseChat(raw: unknown): IncomingChat | null {
  if (!raw || typeof raw !== "object") return null;
  const r = raw as Record<string, unknown>;
  const sender = r.sender as Record<string, unknown> | undefined;
  const recipient = r.recipient as Record<string, unknown> | undefined | null;
  const at =
    typeof r.at === "number" ? r.at : typeof r.at === "string" ? Date.parse(r.at) : Number.NaN;
  const body = typeof r.body === "string" ? r.body.slice(0, 2000) : undefined;
  const channel =
    typeof r.channel === "string" && CHANNELS.has(r.channel)
      ? (r.channel as IncomingChat["channel"])
      : undefined;
  const senderId = str(sender?.userId, 64);
  const senderName = str(sender?.name, 64);
  if (!Number.isFinite(at) || body === undefined || !channel || !senderId || !senderName)
    return null;
  const verdict = ["allowed", "flagged", "masked", "blocked"].includes(r.verdict as string)
    ? (r.verdict as IncomingChat["verdict"])
    : undefined;
  return {
    id: str(r.id, 128),
    at,
    channel,
    roomId: str(r.roomId, 128) ?? null,
    sender: { userId: senderId, name: senderName },
    recipient:
      recipient && str(recipient.userId, 64)
        ? {
            userId: String(recipient.userId),
            name: str(recipient.name, 64) ?? String(recipient.userId),
          }
        : null,
    body,
    verdict,
  };
}

export function parseReport(raw: unknown): NewReport | null {
  if (!raw || typeof raw !== "object") return null;
  const r = raw as Record<string, unknown>;
  const target = r.target as Record<string, unknown> | undefined;
  const reporter = r.reporter as Record<string, unknown> | undefined;
  const targetName = str(target?.name, 64);
  const description = typeof r.description === "string" ? r.description.trim().slice(0, 2000) : "";
  const category = (REPORT_CATEGORIES as readonly string[]).includes(r.category as string)
    ? (r.category as NewReport["category"])
    : "other";
  if (!targetName) return null;
  const ctx = r.context as Record<string, unknown> | undefined;
  return {
    source: r.source === "discord" ? "discord" : "game",
    externalId: str(r.id, 128) ?? null,
    reporter: reporter
      ? { userId: str(reporter.userId, 64) ?? null, name: str(reporter.name, 64) ?? null }
      : null,
    target: { userId: str(target?.userId, 64) ?? null, name: targetName },
    category,
    description: description || "(no description)",
    context: ctx
      ? {
          roomId: str(ctx.roomId, 128),
          instanceId: str(ctx.instanceId, 128),
          at: typeof ctx.at === "number" ? ctx.at : undefined,
        }
      : null,
  };
}
