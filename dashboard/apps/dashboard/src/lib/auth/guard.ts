import { redirect } from "next/navigation";
import type { Staff } from "@/lib/db/schema";
import { DomainError } from "@/lib/errors";
import { type Capability, can } from "./permissions";
import { currentStaff } from "./session";

/** For pages: no session goes to sign-in, a missing capability to the overview. */
export async function requirePage(capability: Capability): Promise<Staff> {
  const me = await currentStaff();
  if (!me) redirect("/login");
  if (!can(me.role, capability)) redirect("/?denied=1");
  return me;
}

/** For server actions and route handlers: refused with a code the caller renders. */
export async function requireAction(capability: Capability): Promise<Staff> {
  const me = await currentStaff();
  if (!me) throw new DomainError("unauthenticated");
  if (!can(me.role, capability)) throw new DomainError("forbidden");
  return me;
}
