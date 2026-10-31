import { desc, like, sql } from "drizzle-orm";
import { db, schema } from "@/lib/db";

export interface KnownPlayer {
  userId: string;
  name: string;
  lastSeenAt: number;
  messages: number;
}

/**
 * Everyone the chat log has seen, newest first: the player list's fallback while the backend's admin
 * routes are unreachable, so a moderator can still find someone to look up.
 */
export function knownPlayers(search?: string, limit = 100): KnownPlayer[] {
  return db
    .select({
      userId: schema.chatMessages.senderUserId,
      name: sql<string>`max(${schema.chatMessages.senderName})`,
      lastSeenAt: sql<number>`max(${schema.chatMessages.at})`,
      messages: sql<number>`count(*)`,
    })
    .from(schema.chatMessages)
    .where(
      search ? like(schema.chatMessages.senderName, `%${search.replace(/[%_]/g, "")}%`) : undefined,
    )
    .groupBy(schema.chatMessages.senderUserId)
    .orderBy(desc(sql`max(${schema.chatMessages.at})`))
    .limit(limit)
    .all();
}
