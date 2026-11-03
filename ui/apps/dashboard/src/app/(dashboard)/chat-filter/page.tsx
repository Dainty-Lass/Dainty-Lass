import type { Metadata } from "next";
import { getTranslations } from "next-intl/server";
import { requirePage } from "@/lib/auth/guard";
import { can } from "@/lib/auth/permissions";
import { filterVersion, listTerms } from "@/lib/chat/terms";
import { listRevisions } from "@/lib/revisions";
import { ChatFilterClient } from "./ChatFilterClient";

export async function generateMetadata(): Promise<Metadata> {
  return { title: (await getTranslations("filter"))("title") };
}

export default async function ChatFilterPage() {
  const me = await requirePage("chat:read");
  return (
    <ChatFilterClient
      terms={listTerms()}
      version={filterVersion()}
      revisions={listRevisions("chat_filter", 30).map(({ snapshot, ...r }) => ({
        ...r,
        size: Array.isArray(snapshot) ? snapshot.length : 0,
      }))}
      canWrite={can(me.role, "filter:write")}
      canRestore={can(me.role, "audit:revert")}
    />
  );
}
