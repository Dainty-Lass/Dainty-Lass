import { recordAudit } from "@/lib/audit/record";
import type { Staff } from "@/lib/db/schema";
import { DomainError } from "@/lib/errors";
import { type ChatLine, present } from "./present";
import { searchChat } from "./store";

export interface DirectRead {
  controllerId: string;
  userId: string;
  name: string;
  reason: string;
  /** The report the read was for, when it came from one. */
  reportId?: number;
}

/**
 * A player's direct messages, sent and received. Private chat, so the only way in: every read needs a
 * reason and leaves an audit event naming who read whose, before anything is returned.
 */
export function readDirectMessages(
  me: Staff,
  read: DirectRead,
  limit = 200,
): { lines: ChatLine[]; hasMore: boolean; eventId: number } {
  const reason = read.reason.trim();
  if (reason.length < 3 || reason.length > 500 || !read.userId)
    throw new DomainError("invalid_input");
  const { messages, hasMore } = searchChat({
    userId: read.userId,
    controllerIds: [read.controllerId],
    channel: "direct",
    direct: true,
    limit,
  });
  const eventId = recordAudit(me, {
    action: "chat.direct_read",
    targetType: "player",
    targetId: read.userId,
    targetLabel: read.name,
    controllerId: read.controllerId,
    reason,
    after: { rows: messages.length, reportId: read.reportId ?? null },
  });
  return { lines: present(messages), hasMore, eventId };
}
