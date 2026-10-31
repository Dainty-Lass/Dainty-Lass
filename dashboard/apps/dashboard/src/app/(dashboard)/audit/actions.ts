"use server";

import { revertEvent } from "@/lib/audit/revert";
import { requireAction } from "@/lib/auth/guard";
import { attempt } from "@/lib/errors";
import { restoreRevision } from "@/lib/revisions";

export async function revertAction(eventId: number) {
  return attempt(async () => {
    const me = await requireAction("audit:revert");
    return revertEvent(me, eventId);
  });
}

export async function restoreRevisionAction(revisionId: number) {
  return attempt(async () => {
    const me = await requireAction("audit:revert");
    restoreRevision(revisionId, me);
  });
}
