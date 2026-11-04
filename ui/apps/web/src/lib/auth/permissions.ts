/**
 * Roles are ranked; a capability names the lowest role that holds it. Kept as one table so the nav,
 * the pages and the server actions cannot disagree about who may do what.
 *
 *   user       anyone signed in (and approved, when the server asks): the overview, players, controllers
 *   moderator  + reports, chat logs and the chat filter, moderating players, and a player's direct
 *              messages (each read audited, with a reason)
 *   admin      + the audit log and undo, accounts, the filter's terms, controllers, members
 *   owner      + settings
 */
export const ROLES = ["user", "moderator", "admin", "owner"] as const;
export type Role = (typeof ROLES)[number];

const CAPABILITIES = {
  "overview:read": "user",
  "players:read": "user",
  "controllers:read": "user",
  "reports:read": "moderator",
  "reports:write": "moderator",
  "chat:read": "moderator",
  "chat:export": "moderator",
  "chat:direct": "moderator",
  "players:moderate": "moderator",
  "audit:read": "admin",
  "audit:revert": "admin",
  "players:access": "admin",
  "players:roles": "admin",
  // One ban on every controller: a moderator acts per server, an admin across the fleet.
  "players:fleet": "admin",
  "filter:write": "admin",
  "controllers:write": "admin",
  "staff:read": "admin",
  "staff:write": "admin",
  "settings:read": "admin",
  "settings:write": "owner",
  "staff:owners": "owner",
} as const satisfies Record<string, Role>;

export type Capability = keyof typeof CAPABILITIES;

export function rank(role: Role): number {
  return ROLES.indexOf(role);
}

export function can(role: Role | null | undefined, capability: Capability): boolean {
  if (!role) return false;
  return rank(role) >= rank(CAPABILITIES[capability]);
}

/** Who may set `target` on someone: never above your own role, and only an owner makes owners. */
export function canAssign(actor: Role, target: Role): boolean {
  if (target === "owner") return actor === "owner";
  return rank(actor) >= rank(target) && can(actor, "staff:write");
}

export function isRole(value: unknown): value is Role {
  return typeof value === "string" && (ROLES as readonly string[]).includes(value);
}
