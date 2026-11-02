"use server";

import { requireAction } from "@/lib/auth/guard";
import { createTerm, deleteTerm, type TermInput, testText, updateTerm } from "@/lib/chat/terms";
import { attempt } from "@/lib/errors";

export async function createTermAction(input: TermInput) {
  return attempt(async () => {
    createTerm(await requireAction("filter:write"), input);
  });
}

export async function updateTermAction(id: number, input: TermInput) {
  return attempt(async () => {
    updateTerm(await requireAction("filter:write"), id, input);
  });
}

export async function deleteTermAction(id: number) {
  return attempt(async () => {
    deleteTerm(await requireAction("filter:write"), id);
  });
}

/** The tester: what the current list would do to a line, without storing it. */
export async function testAction(text: string, channel: string) {
  return attempt(async () => {
    await requireAction("chat:read");
    const { matches, verdict, masked } = testText(text.slice(0, 2000), channel || undefined);
    return { verdict, masked, termIds: [...new Set(matches.map((m) => m.termId))] };
  });
}
