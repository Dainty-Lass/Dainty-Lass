import type { Metadata } from "next";
import { getTranslations } from "next-intl/server";
import { gameHistory } from "@/lib/audit/game-history";
import { queryAudit } from "@/lib/audit/query";
import { requirePage } from "@/lib/auth/guard";
import { can } from "@/lib/auth/permissions";
import { present } from "@/lib/chat/present";
import { searchChat } from "@/lib/chat/store";
import type { AuditEvent } from "@/lib/db/schema";
import { knownPlayers } from "@/lib/directory";
import { DomainError } from "@/lib/errors";
import { getController } from "@/lib/fleet/controllers";
import { controllerParam, currentScope } from "@/lib/fleet/scope";
import { fleet } from "@/lib/fleet/status";
import { adminApi, type GamePlayer } from "@/lib/game/admin-api";
import { type BackendAccount, backend } from "@/lib/game/backend";
import { op, operatorId } from "@/lib/players";
import { reportsAgainst } from "@/lib/reports/store";
import { getSettings } from "@/lib/settings";
import { PlayerDetailClient } from "./PlayerDetailClient";

export async function generateMetadata({
  params,
}: {
  params: Promise<{ name: string }>;
}): Promise<Metadata> {
  return { title: decodeURIComponent((await params).name) };
}

export default async function PlayerPage({
  params,
  searchParams,
}: {
  params: Promise<{ name: string }>;
  searchParams: Promise<{ c?: string }>;
}) {
  const me = await requirePage("players:read");
  const name = decodeURIComponent((await params).name);
  const controllerId = controllerParam((await searchParams).c, await currentScope());
  const controller = getController(controllerId)!;
  const t = await getTranslations("players");

  let player: GamePlayer | null = null;
  let gameFailure: string | null = null;
  try {
    player = await adminApi(controllerId).player(name, operatorId(me));
  } catch (error) {
    gameFailure = error instanceof DomainError ? error.code : "unknown";
  }
  // While the game cannot answer, the chat log still knows who a name belongs to.
  const known = knownPlayers(name, 50).find(
    (p) => p.controllerId === controllerId && p.name.toLowerCase() === name.toLowerCase(),
  );
  const userId = player?.userId ?? known?.userId ?? null;

  let account: BackendAccount | null = null;
  if (userId) {
    try {
      account = (await backend(controllerId).playersById([userId], op(me))).accounts[0] ?? null;
    } catch {
      // The backend's view is extra detail; the page works without it.
    }
  }

  const displayName = player?.username ?? account?.displayName ?? known?.name ?? name;
  const may = {
    chat: can(me.role, "chat:read"),
    reports: can(me.role, "reports:read"),
    audit: can(me.role, "audit:read"),
  };
  const chat =
    userId && may.chat
      ? present(searchChat({ userId, controllerIds: [controllerId], limit: 100 }).messages)
      : [];
  const own =
    userId && may.audit
      ? queryAudit({
          targetType: "player",
          targetId: userId,
          controllerIds: [controllerId],
          pageSize: 100,
        }).events
      : [];
  let fromGame: AuditEvent[] = [];
  if (userId && may.audit) {
    try {
      const { records } = await backend(controllerId).audit(
        { targetUserId: userId, limit: 100 },
        op(me),
      );
      fromGame = gameHistory(records, userId, displayName, controllerId);
    } catch {
      // The backend's record is extra; the dashboard's own stands without it.
    }
  }
  const history = [...own, ...fromGame].sort((a, b) => b.at - a.at).slice(0, 150);

  return (
    <PlayerDetailClient
      name={displayName}
      userId={userId}
      player={player}
      account={account}
      gameFailure={gameFailure}
      notFound={!player && !account && !known && !gameFailure}
      chat={chat}
      reports={
        may.reports
          ? reportsAgainst(userId ?? "", displayName).filter((r) => r.controllerId === controllerId)
          : []
      }
      history={history}
      reasons={getSettings().moderationReasons}
      can={{
        moderate: can(me.role, "players:moderate"),
        access: can(me.role, "players:access"),
        roles: can(me.role, "players:roles"),
        grant: can(me.role, "players:grant"),
        report: can(me.role, "reports:write"),
        revert: can(me.role, "audit:revert"),
        fleet: can(me.role, "players:fleet"),
        readChat: may.chat,
        readDirect: can(me.role, "chat:direct"),
        readReports: may.reports,
        readAudit: may.audit,
      }}
      backLabel={t("title")}
      controller={{ id: controllerId, name: controller.name }}
      fleetSize={fleet().length}
    />
  );
}
