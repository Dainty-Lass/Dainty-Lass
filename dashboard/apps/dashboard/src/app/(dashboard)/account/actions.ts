"use server";

import { requireSignedIn } from "@/lib/auth/guard";
import { unlinkAccount } from "@/lib/auth/sign-in";
import { attempt } from "@/lib/errors";

export async function unlinkAccountAction(accountRowId: number) {
  return attempt(async () => unlinkAccount(await requireSignedIn(), accountRowId));
}
