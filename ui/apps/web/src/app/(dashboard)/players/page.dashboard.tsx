import type { Metadata } from "next";
import { getTranslations } from "next-intl/server";
import { requirePage } from "@/lib/auth/guard";
import { knownPlayers } from "@/lib/directory";
import { DomainError } from "@/lib/errors";
import { currentScope } from "@/lib/fleet/scope";
import { fleet } from "@/lib/fleet/status";
import { type BackendAccount, backend } from "@/lib/game/backend";
import { op } from "@/lib/players";
import { type FleetAccount, PlayersClient } from "./PlayersClient";

export async function generateMetadata(): Promise<Metadata> {
  return { title: (await getTranslations("players"))("title") };
}

// timedOut is no account status: the backend lists the active accounts with a timeout in force
const STATUSES = ["active", "disabled", "banned", "timedOut"] as const;
const PAGE = 25;

export default async function PlayersPage({
  searchParams,
}: {
  searchParams: Promise<{ q?: string; status?: string; page?: string; online?: string }>;
}) {
  const me = await requirePage("players:read");
  const [sp, scope] = await Promise.all([searchParams, currentScope()]);
  const status = STATUSES.find((s) => s === sp.status);
  const page = Math.max(1, Number(sp.page) || 1);
  const online = sp.online === "1";

  // Every controller has its own accounts, so a fleet-wide list asks each one; a page is page N of each.
  const views = fleet().filter((c) =>
    scope.kind === "one" ? c.id === scope.id : c.state === "online" || c.state === "stale",
  );
  const results = await Promise.all(
    views.map(async (c) => {
      try {
        const api = backend(c.id);
        const [list, live] = await Promise.all([
          online ? null : api.players({ search: sp.q, status, page, limit: PAGE }, op(me)),
          api.onlinePlayers(op(me)),
        ]);
        const onlineIds = live.players.map((p) => p.userId);
        const accounts: BackendAccount[] = online
          ? onlineIds.length
            ? (await api.playersById(onlineIds.slice(0, 100), op(me))).accounts
            : []
          : (list?.accounts ?? []);
        return {
          c,
          accounts,
          total: online ? accounts.length : (list?.total ?? 0),
          onlineIds,
          failure: null as string | null,
        };
      } catch (error) {
        return {
          c,
          accounts: [],
          total: 0,
          onlineIds: [],
          failure: error instanceof DomainError ? error.code : "unknown",
        };
      }
    }),
  );

  const accounts: FleetAccount[] = results.flatMap((r) =>
    r.accounts.map((a) => ({
      ...a,
      controllerId: r.c.id,
      controllerName: r.c.name,
      online: r.onlineIds.includes(a.userId),
    })),
  );
  const failures = results
    .filter((r) => r.failure)
    .map((r) => ({ name: r.c.name, code: r.failure! }));
  const answered = results.some((r) => !r.failure);

  return (
    <PlayersClient
      accounts={answered ? accounts : null}
      total={Math.max(...results.map((r) => r.total), 0)}
      combinedTotal={results.reduce((n, r) => n + r.total, 0)}
      page={page}
      onlineCount={results.reduce((n, r) => n + r.onlineIds.length, 0)}
      failures={failures}
      // Whether rows come from several controllers, not how many answered.
      fleetWide={scope.kind === "all" && fleet().length > 1}
      known={answered ? [] : knownPlayers(sp.q)}
      filters={{ q: sp.q ?? "", status: status ?? "", online }}
    />
  );
}
