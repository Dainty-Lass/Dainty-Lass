import {
  and,
  between,
  desc,
  eq,
  gte,
  inArray,
  like,
  lt,
  lte,
  ne,
  or,
  type SQL,
  sql,
} from "drizzle-orm";
import { recordAudit } from "@/lib/audit/record";
import { db, schema } from "@/lib/db";
import type { ChatMessage } from "@/lib/db/schema";
import { findMatches, verdictOf } from "./filter";
import { compiledTerms, listTerms } from "./terms";

export interface ChatQuery {
  controllerIds?: string[];
  userId?: string;
  name?: string;
  roomId?: string;
  channel?: ChatMessage["channel"];
  text?: string;
  /** Only lines the filter matched. */
  flagged?: boolean;
  from?: number;
  to?: number;
  /** Older than this id, for "load more". */
  before?: number;
  limit?: number;
  /** Include direct messages: only readDirectMessages passes this, so every read of them is audited. */
  direct?: boolean;
}

function where(q: ChatQuery): SQL | undefined {
  const parts: (SQL | undefined)[] = [];
  if (q.controllerIds) parts.push(inArray(schema.chatMessages.controllerId, q.controllerIds));
  if (q.userId)
    parts.push(
      or(
        eq(schema.chatMessages.senderUserId, q.userId),
        eq(schema.chatMessages.recipientUserId, q.userId),
      ),
    );
  if (q.name) parts.push(like(schema.chatMessages.senderName, `%${q.name.replace(/[%_]/g, "")}%`));
  if (q.roomId) parts.push(eq(schema.chatMessages.roomId, q.roomId));
  if (q.channel) parts.push(eq(schema.chatMessages.channel, q.channel));
  if (!q.direct) parts.push(ne(schema.chatMessages.channel, "direct"));
  // LIKE is case-insensitive for ASCII in SQLite, which is what a moderator searching expects.
  if (q.text) parts.push(like(schema.chatMessages.body, `%${q.text.replace(/[%_]/g, "")}%`));
  if (q.flagged) parts.push(inArray(schema.chatMessages.verdict, ["flagged", "masked", "blocked"]));
  if (q.from) parts.push(gte(schema.chatMessages.at, q.from));
  if (q.to) parts.push(lte(schema.chatMessages.at, q.to));
  if (q.before) parts.push(lt(schema.chatMessages.id, q.before));
  const defined = parts.filter((p): p is SQL => Boolean(p));
  return defined.length ? and(...defined) : undefined;
}

export function searchChat(q: ChatQuery): { messages: ChatMessage[]; hasMore: boolean } {
  const limit = Math.min(Math.max(q.limit ?? 100, 1), 1000);
  const rows = db
    .select()
    .from(schema.chatMessages)
    .where(where(q))
    .orderBy(desc(schema.chatMessages.at), desc(schema.chatMessages.id))
    .limit(limit + 1)
    .all();
  return { messages: rows.slice(0, limit), hasMore: rows.length > limit };
}

/**
 * What was said around a moment: the room's lines (or the player's) either side of it, oldest first.
 * Never direct messages, which only readDirectMessages returns.
 */
export function chatAround(
  at: number,
  minutes: number,
  scope: { controllerId: string; roomId?: string; userId?: string },
): ChatMessage[] {
  const span = minutes * 60_000;
  // Room and account ids are each controller's own, so the same id elsewhere is someone else.
  const filters: SQL[] = [
    between(schema.chatMessages.at, at - span, at + span),
    eq(schema.chatMessages.controllerId, scope.controllerId),
    ne(schema.chatMessages.channel, "direct"),
  ];
  if (scope.roomId) filters.push(eq(schema.chatMessages.roomId, scope.roomId));
  else if (scope.userId) {
    filters.push(
      or(
        eq(schema.chatMessages.senderUserId, scope.userId),
        eq(schema.chatMessages.recipientUserId, scope.userId),
      )!,
    );
  }
  return db
    .select()
    .from(schema.chatMessages)
    .where(and(...filters))
    .orderBy(schema.chatMessages.at)
    .limit(500)
    .all();
}

export interface IncomingChat {
  id?: string;
  at: number;
  channel: ChatMessage["channel"];
  roomId?: string | null;
  sender: { userId: string; name: string };
  recipient?: { userId: string; name: string } | null;
  body: string;
  /** The backend's own verdict when it enforced the filter; rated here otherwise. */
  verdict?: ChatMessage["verdict"];
}

export function ingestChat(
  batch: IncomingChat[],
  controllerId = "local",
): { stored: number; duplicates: number } {
  const terms = compiledTerms();
  let stored = 0;
  let patterns: Map<number, string> | null = null;
  for (const m of batch) {
    const matches = findMatches(m.body, terms, m.channel);
    const verdict = m.verdict ?? verdictOf(matches);
    const result = db
      .insert(schema.chatMessages)
      .values({
        controllerId,
        externalId: m.id ?? null,
        at: m.at,
        channel: m.channel,
        roomId: m.roomId ?? null,
        senderUserId: m.sender.userId,
        senderName: m.sender.name,
        recipientUserId: m.recipient?.userId ?? null,
        recipientName: m.recipient?.name ?? null,
        body: m.body,
        verdict,
        matchedTermIds: matches.length ? [...new Set(matches.map((x) => x.termId))] : null,
      })
      .onConflictDoNothing()
      .returning({ id: schema.chatMessages.id })
      .all();
    stored += result.length;
    const row = result[0];
    if (row && verdict !== "allowed") {
      patterns ??= new Map(listTerms().map((t) => [t.id, t.pattern]));
      const termIds = [...new Set(matches.map((x) => x.termId))];
      // The player's History names the verdict and the terms, never the line: a direct message is read only
      // with a reason (readDirectMessages), so History links to the chat log instead.
      recordAudit(
        { ingest: "chat-filter" },
        {
          action: `chat.filter_${verdict}`,
          targetType: "player",
          targetId: m.sender.userId,
          targetLabel: m.sender.name,
          controllerId,
          after: {
            verdict,
            terms: termIds.map((id) => patterns?.get(id) ?? `#${id}`),
            channel: m.channel,
            chatMessageId: row.id,
            at: m.at,
          },
        },
      );
    }
  }
  return { stored, duplicates: batch.length - stored };
}

export function chatStats(
  since: number,
  controllerIds?: string[],
): { total: number; flagged: number; senders: number } {
  const row = db
    .select({
      total: sql<number>`count(*)`,
      flagged: sql<number>`sum(case when ${schema.chatMessages.verdict} != 'allowed' then 1 else 0 end)`,
      senders: sql<number>`count(distinct ${schema.chatMessages.senderUserId})`,
    })
    .from(schema.chatMessages)
    .where(
      and(
        gte(schema.chatMessages.at, since),
        controllerIds ? inArray(schema.chatMessages.controllerId, controllerIds) : undefined,
      ),
    )
    .get();
  return { total: row?.total ?? 0, flagged: row?.flagged ?? 0, senders: row?.senders ?? 0 };
}

/** Hourly message counts for the overview's sparkline, oldest first, gaps filled with zero. */
export function chatHourly(
  hours: number,
  controllerIds?: string[],
): { hour: number; total: number; flagged: number }[] {
  const start = Math.floor((Date.now() - hours * 3_600_000) / 3_600_000) * 3_600_000;
  const rows = db
    .select({
      hour: sql<number>`(${schema.chatMessages.at} / 3600000) * 3600000`,
      total: sql<number>`count(*)`,
      flagged: sql<number>`sum(case when ${schema.chatMessages.verdict} != 'allowed' then 1 else 0 end)`,
    })
    .from(schema.chatMessages)
    .where(
      and(
        gte(schema.chatMessages.at, start),
        controllerIds ? inArray(schema.chatMessages.controllerId, controllerIds) : undefined,
      ),
    )
    .groupBy(sql`1`)
    .all();
  const byHour = new Map(rows.map((r) => [r.hour, r]));
  return Array.from({ length: hours + 1 }, (_, i) => {
    const hour = start + i * 3_600_000;
    const r = byHour.get(hour);
    return { hour, total: r?.total ?? 0, flagged: r?.flagged ?? 0 };
  });
}

export function pruneChat(retentionDays: number): number {
  if (retentionDays <= 0) return 0;
  return db
    .delete(schema.chatMessages)
    .where(lt(schema.chatMessages.at, Date.now() - retentionDays * 86_400_000))
    .returning({ id: schema.chatMessages.id })
    .all().length;
}

let lastPrune = 0;

/** Retention without a timer: the ingest route calls this, and it prunes at most once an hour. */
export function pruneIfDue(retentionDays: number): void {
  if (Date.now() - lastPrune < 3_600_000) return;
  lastPrune = Date.now();
  pruneChat(retentionDays);
}

export function messageById(id: number): ChatMessage | undefined {
  return db.select().from(schema.chatMessages).where(eq(schema.chatMessages.id, id)).get();
}
