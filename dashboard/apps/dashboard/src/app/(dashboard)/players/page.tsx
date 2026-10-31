import type { Metadata } from "next";
import { getTranslations } from "next-intl/server";
import { requirePage } from "@/lib/auth/guard";
import { knownPlayers } from "@/lib/directory";
import { DomainError } from "@/lib/errors";
import { type BackendAccount, backend } from "@/lib/game/backend";
import { op } from "@/lib/players";
import { PlayersClient } from "./PlayersClient";

export async function generateMetadata(): Promise<Metadata> {
  return { title: (await getTranslations("players"))("title") };
}

const STATUSES = ["active", "disabled", "banned"] as const;

export default async function PlayersPage({
  searchParams,
}: {
  searchParams: Promise<{ q?: string; status?: string; page?: string; online?: string }>;
}) {
  const me = await requirePage("players:read");
  const sp = await searchParams;
  const status = STATUSES.find((s) => s === sp.status);
  const page = Math.max(1, Number(sp.page) || 1);
  const online = sp.online === "1";

  let accounts: BackendAccount[] | null = null;
  let total = 0;
  let onlineIds: string[] = [];
  let failure: string | null = null;
  try {
    const [list, live] = await Promise.all([
      online ? null : backend.players({ search: sp.q, status, page, limit: 25 }, op(me)),
      backend.onlinePlayers(op(me)),
    ]);
    onlineIds = live.players.map((p) => p.userId);
    if (online) {
      accounts = onlineIds.length
        ? (await backend.playersById(onlineIds.slice(0, 100), op(me))).accounts
        : [];
      total = accounts.length;
    } else if (list) {
      accounts = list.accounts;
      total = list.total;
    }
  } catch (error) {
    failure = error instanceof DomainError ? error.code : "unknown";
  }

  return (
    <PlayersClient
      accounts={accounts}
      total={total}
      page={page}
      onlineIds={onlineIds}
      failure={failure}
      known={accounts ? [] : knownPlayers(sp.q)}
      filters={{ q: sp.q ?? "", status: status ?? "", online }}
    />
  );
}
