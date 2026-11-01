import type { Metadata } from "next";
import { getTranslations } from "next-intl/server";
import { requirePage } from "@/lib/auth/guard";
import { can } from "@/lib/auth/permissions";
import { present } from "@/lib/chat/present";
import { searchChat } from "@/lib/chat/store";
import { currentScope, scopeIds } from "@/lib/fleet/scope";
import { chatQueryFromParams, type SearchParams } from "@/lib/query-params";
import { ChatClient } from "./ChatClient";

export async function generateMetadata(): Promise<Metadata> {
  return { title: (await getTranslations("chat"))("title") };
}

export default async function ChatPage({ searchParams }: { searchParams: Promise<SearchParams> }) {
  const me = await requirePage("chat:read");
  const raw = await searchParams;
  const q = chatQueryFromParams(raw);
  const { messages, hasMore } = searchChat({
    ...q,
    controllerIds: scopeIds(await currentScope()),
    limit: 200,
  });
  const exportQuery = new URLSearchParams(
    Object.entries(raw).flatMap(([k, v]) =>
      typeof v === "string" && k !== "before" ? [[k, v]] : [],
    ),
  ).toString();
  return (
    <ChatClient
      lines={present(messages)}
      hasMore={hasMore}
      filters={{
        user: q.userId ?? "",
        name: q.name ?? "",
        room: q.roomId ?? "",
        channel: q.channel ?? "",
        q: q.text ?? "",
        flagged: Boolean(q.flagged),
        range: q.range ?? "",
      }}
      exportHref={
        can(me.role, "chat:export")
          ? `/api/export/chat${exportQuery ? `?${exportQuery}` : ""}`
          : null
      }
    />
  );
}
