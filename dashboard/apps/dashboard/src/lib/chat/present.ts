import type { ChatMessage } from "@/lib/db/schema";
import { findMatches, segments } from "./filter";
import { compiledTerms } from "./terms";

export type ChatLine = ChatMessage & { segments: { text: string; hit: boolean }[] };

/** Highlights are worked out on the server, so the browser never receives the filter's patterns. */
export function present(messages: ChatMessage[]): ChatLine[] {
  const terms = compiledTerms();
  return messages.map((m) => ({
    ...m,
    segments: segments(m.body, findMatches(m.body, terms, m.channel)),
  }));
}
