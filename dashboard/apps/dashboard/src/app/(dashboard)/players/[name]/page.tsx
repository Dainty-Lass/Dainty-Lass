import type { Metadata } from "next";
import { getTranslations } from "next-intl/server";
import { queryAudit } from "@/lib/audit/query";
import { requirePage } from "@/lib/auth/guard";
import { can } from "@/lib/auth/permissions";
import { present } from "@/lib/chat/present";
import { searchChat } from "@/lib/chat/store";
import { knownPlayers } from "@/lib/directory";
import { DomainError } from "@/lib/errors";
import { adminApi, type GamePlayer } from "@/lib/game/admin-api";
import { type BackendAccount, backend } from "@/lib/game/backend";
import { op } from "@/lib/players";
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

export default async function PlayerPage({ params }: { params: Promise<{ name: string }> }) {
  const me = await requirePage("players:read");
  const name = decodeURIComponent((await params).name);
  const t = await getTranslations("players");

  let player: GamePlayer | null = null;
  let gameFailure: string | null = null;
  try {
    player = await adminApi.player(name, me.discordId);
  } catch (error) {
    gameFailure = error instanceof DomainError ? error.code : "unknown";
  }
  // While the game cannot answer, the chat log still knows who a name belongs to.
  const known = knownPlayers(name, 20).find((p) => p.name.toLowerCase() === name.toLowerCase());
  const userId = player?.userId ?? known?.userId ?? null;

  let account: BackendAccount | null = null;
  if (userId) {
    try {
      account = (await backend.playersById([userId], op(me))).accounts[0] ?? null;
    } catch {
      // The backend's view is extra detail; the page works without it.
    }
  }

  const displayName = player?.username ?? account?.displayName ?? known?.name ?? name;
  const chat = userId ? present(searchChat({ userId, limit: 100 }).messages) : [];
  const history = userId
    ? queryAudit({ targetType: "player", targetId: userId, pageSize: 100 }).events
    : [];

  return (
    <PlayerDetailClient
      name={displayName}
      userId={userId}
      player={player}
      account={account}
      gameFailure={gameFailure}
      notFound={!player && !account && !known && !gameFailure}
      chat={chat}
      reports={reportsAgainst(userId ?? "", displayName)}
      history={history}
      reasons={getSettings().moderationReasons}
      can={{
        moderate: can(me.role, "players:moderate"),
        access: can(me.role, "players:access"),
        roles: can(me.role, "players:roles"),
        report: can(me.role, "reports:write"),
        revert: can(me.role, "audit:revert"),
      }}
      backLabel={t("title")}
    />
  );
}
