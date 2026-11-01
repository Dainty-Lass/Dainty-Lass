// What the game's admin APIs answer: the admin service's GraphQL (dainty-lass docs/admin-api.md) and the
// backend's /admin/v1. Shared so the dashboard reads an agent's answer exactly as it reads its own.

export type AccountStatus = "ACTIVE" | "BANNED" | "DISABLED" | "UNKNOWN";
export type RestrictionKind = "BAN" | "TIMEOUT" | "DISABLED";

export interface Restriction {
  kind: RestrictionKind;
  reason: string | null;
  since: string | null;
  by: string | null;
  until: string | null;
}

export interface GamePlayer {
  userId: string;
  username: string;
  status: AccountStatus;
  restriction: Restriction | null;
  online: boolean;
  location: {
    kind: "HUB" | "HUNT" | "TRAINING" | "TUTORIAL";
    instanceId: string;
    title: string;
  } | null;
  admin: boolean;
  createdAt: string | null;
  lastLoginAt: string | null;
}

export interface ServerStatus {
  name: string;
  mode: "PRIVATE" | "PUBLIC";
  registration: string;
  version: string;
  commit: string | null;
  uptimeSeconds: number;
  onlineCount: number;
  chat: boolean;
  gameData: string;
}

export interface Hunt {
  id: string;
  title: string;
  map: string;
  behemoth: string | null;
  playerCount: number;
  maxPlayers: number;
  players: { userId: string; username: string | null }[];
  startedAt: string | null;
}

export interface HuntStatus {
  maxHuntServers: number;
  running: number;
  free: number;
  hunts: Hunt[];
}

export interface BackendAccount {
  userId: string;
  displayName: string;
  email: string;
  approvalStatus: "pending" | "approved" | "rejected";
  status: "active" | "disabled" | "banned";
  roles: string[];
  rolesUpdatedAt?: string;
  rolesUpdatedBy?: string;
  usernameSet: boolean;
  createdAt: string;
  lastLoginAt?: string;
  approvalUpdatedAt?: string;
  approvalUpdatedBy?: string;
  approvalReason?: string;
}

export interface Operator {
  /** The staff member's Discord id; the backend audits the call as manager:<id>. */
  id: string;
  name: string;
}
