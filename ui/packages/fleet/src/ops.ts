/**
 * Everything the dashboard may ask a controller to do. An agent runs only these: a GraphQL operation by
 * name (never a query the dashboard wrote), and a backend call whose method and path match a route
 * below. A dashboard that is compromised can still moderate, but cannot reach anything else on the host.
 */

const PLAYER = `userId username status online admin createdAt lastLoginAt
  restriction { kind reason since by until } location { kind instanceId title }`;

export const ADMIN_OPS = {
  serverStatus:
    "{ serverStatus { name mode registration version commit uptimeSeconds onlineCount chat gameData } }",
  huntStatus:
    "{ huntStatus { maxHuntServers running free hunts { id title map behemoth playerCount maxPlayers startedAt players { userId username } } } }",
  player: `query($u: String!) { player(username: $u) { ${PLAYER} } }`,
  kick: "mutation($u: String!) { kick(username: $u) { userId username connectionsClosed } }",
  timeout: `mutation($u: String!, $m: Int, $t: String, $r: String!) { timeout(username: $u, minutes: $m, until: $t, reason: $r) { ${PLAYER} } }`,
  liftTimeout: `mutation($u: String!) { liftTimeout(username: $u) { ${PLAYER} } }`,
  ban: `mutation($u: String!, $r: String!) { ban(username: $u, reason: $r) { ${PLAYER} } }`,
  unban: `mutation($u: String!, $r: String) { unban(username: $u, reason: $r) { ${PLAYER} } }`,
  logSettings: "{ logSettings { format color gelfUrl restartNeeded } }",
  setLogSettings:
    "mutation($i: LogSettingsInput!) { setLogSettings(input: $i) { format color gelfUrl restartNeeded } }",
} as const;

export type AdminOp = keyof typeof ADMIN_OPS;

export function isAdminOp(value: unknown): value is AdminOp {
  return typeof value === "string" && Object.hasOwn(ADMIN_OPS, value);
}

const ID = "[A-Za-z0-9_%.-]{1,128}";

const BACKEND_ROUTES: { method: string; path: RegExp }[] = [
  { method: "GET", path: /^\/admin\/v1\/players$/ },
  { method: "GET", path: /^\/admin\/v1\/online-players$/ },
  { method: "GET", path: /^\/admin\/v1\/audit$/ },
  { method: "PATCH", path: new RegExp(`^/admin/v1/players/${ID}/access$`) },
  { method: "PATCH", path: new RegExp(`^/admin/v1/players/${ID}/roles$`) },
  { method: "GET", path: /^\/admin\/v1\/grant-catalog$/ },
  { method: "POST", path: new RegExp(`^/admin/v1/players/${ID}/grants$`) },
];

/** Whether a backend request target (path and query) is one an agent will forward. */
export function backendAllowed(method: string, target: string): boolean {
  if (target.length > 2048 || !target.startsWith("/")) return false;
  let path: string;
  try {
    // Rejects "..", "//" tricks: the path is matched after the URL parser normalises it, and must be unchanged.
    const url = new URL(target, "http://agent.invalid");
    path = url.pathname;
    if (`${url.pathname}${url.search}` !== target) return false;
  } catch {
    return false;
  }
  return BACKEND_ROUTES.some((r) => r.method === method.toUpperCase() && r.path.test(path));
}
