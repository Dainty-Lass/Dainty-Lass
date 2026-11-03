import { cookies } from "next/headers";
import { getController, LOCAL, listControllers } from "./controllers";

/**
 * Which controllers a page shows: one, or all of them. Chosen in the rail and kept in a cookie, so it
 * follows the person from page to page. A page that acts on one player takes its controller from the URL
 * instead (`?c=`), since links must stay right whatever the reader's scope.
 */
export const SCOPE_COOKIE = "dainty-scope";
export const ALL = "all";

export type Scope = { kind: "all" } | { kind: "one"; id: string };

export async function currentScope(): Promise<Scope> {
  const raw = (await cookies()).get(SCOPE_COOKIE)?.value;
  if (raw && raw !== ALL && getController(raw)) return { kind: "one", id: raw };
  // With a single controller there is nothing to choose between.
  if (!raw && listControllers().length === 1) return { kind: "one", id: LOCAL };
  return { kind: "all" };
}

/** The controller ids a scoped query covers, or undefined for every controller. */
export function scopeIds(scope: Scope): string[] | undefined {
  return scope.kind === "one" ? [scope.id] : undefined;
}

/** A `?c=` value that names a controller, else the scope's, else the local one. */
export function controllerParam(value: string | string[] | undefined, scope: Scope): string {
  const v = Array.isArray(value) ? value[0] : value;
  if (v && getController(v)) return v;
  return scope.kind === "one" ? scope.id : LOCAL;
}
