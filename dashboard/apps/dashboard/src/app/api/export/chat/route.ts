import { NextResponse } from "next/server";
import { recordAudit } from "@/lib/audit/record";
import { can } from "@/lib/auth/permissions";
import { currentStaff } from "@/lib/auth/session";
import { searchChat } from "@/lib/chat/store";
import { csv, csvResponse } from "@/lib/csv";
import { chatQueryFromParams } from "@/lib/query-params";

/** The chat log as CSV, with the same filters as the page. Exporting is audited: it copies players' words off the server. */
export async function GET(request: Request) {
  const me = await currentStaff();
  if (!me || !can(me.role, "chat:export"))
    return NextResponse.json({ error: "forbidden" }, { status: 403 });
  const q = chatQueryFromParams(new URL(request.url).searchParams);
  const { messages } = searchChat({ ...q, limit: 1000 });
  recordAudit(me, {
    action: "chat.export",
    targetType: "player",
    targetId: q.userId ?? null,
    targetLabel: q.name ?? null,
    after: { ...q, rows: messages.length },
  });
  return csvResponse(
    `chat-${new Date().toISOString().slice(0, 10)}.csv`,
    csv(
      [
        "time_utc",
        "channel",
        "room",
        "sender_id",
        "sender",
        "recipient_id",
        "recipient",
        "verdict",
        "message",
      ],
      messages.map((m) => [
        new Date(m.at).toISOString(),
        m.channel,
        m.roomId,
        m.senderUserId,
        m.senderName,
        m.recipientUserId,
        m.recipientName,
        m.verdict,
        m.body,
      ]),
    ),
  );
}
