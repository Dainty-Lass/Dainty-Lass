"use server";

import { headers } from "next/headers";
import { redirect } from "next/navigation";
import { recordAudit } from "@/lib/audit/record";
import { auth } from "./server";
import { currentStaff } from "./session";

/** Ends this browser's session. A form action, so it needs no script and cannot be fired by a GET. */
export async function signOutAction(): Promise<void> {
  const me = await currentStaff();
  await auth.api.signOut({ headers: await headers() }).catch(() => null);
  if (me)
    recordAudit(me, {
      action: "session.sign_out",
      targetType: "session",
      targetId: me.id,
      targetLabel: me.username,
    });
  redirect("/login");
}
