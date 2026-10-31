import { desc } from "drizzle-orm";
import { config, discordConfigured } from "@/lib/config";
import { db, schema } from "@/lib/db";
import { DomainError } from "@/lib/errors";
import {
  adminApi,
  adminApiConfigured,
  type HuntStatus,
  type ServerStatus,
} from "@/lib/game/admin-api";
import { backend, backendConfigured } from "@/lib/game/backend";

export type ServiceState = "ok" | "down" | "not_configured";

export interface Health {
  adminApi: ServiceState;
  backend: ServiceState;
  discord: ServiceState;
  ingest: ServiceState;
  lastChatAt: number | null;
  server: ServerStatus | null;
  hunts: HuntStatus | null;
  online: { userId: string; displayName: string }[] | null;
}

const stateOf = (error: unknown): ServiceState =>
  error instanceof DomainError && error.code === "game_not_configured" ? "not_configured" : "down";

/** Everything the overview shows about the game, fetched side by side; a service that fails is a state, not an error page. */
export async function health(operator: { id: string; name: string }): Promise<Health> {
  const [server, hunts, online] = await Promise.allSettled([
    adminApi.serverStatus(operator.id),
    adminApi.huntStatus(operator.id),
    backend.onlinePlayers(operator),
  ]);
  const last = db
    .select({ at: schema.chatMessages.at })
    .from(schema.chatMessages)
    .orderBy(desc(schema.chatMessages.at))
    .limit(1)
    .get();
  return {
    adminApi: !adminApiConfigured()
      ? "not_configured"
      : server.status === "fulfilled"
        ? "ok"
        : stateOf(server.reason),
    backend: !backendConfigured()
      ? "not_configured"
      : online.status === "fulfilled"
        ? "ok"
        : stateOf(online.reason),
    discord: discordConfigured() ? "ok" : "not_configured",
    ingest: config.ingestKey && config.ingestKey.length >= 32 ? "ok" : "not_configured",
    lastChatAt: last?.at ?? null,
    server: server.status === "fulfilled" ? server.value : null,
    hunts: hunts.status === "fulfilled" ? hunts.value : null,
    online: online.status === "fulfilled" ? online.value.players : null,
  };
}
