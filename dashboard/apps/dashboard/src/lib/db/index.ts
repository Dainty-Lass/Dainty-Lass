import { Database } from "bun:sqlite";
import { existsSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { drizzle } from "drizzle-orm/bun-sqlite";
import { migrate } from "drizzle-orm/bun-sqlite/migrator";
import { config } from "@/lib/config";
import * as schema from "./schema";

function migrationsFolder(): string {
  const candidates = [
    process.env.DASHBOARD_MIGRATIONS_DIR,
    join(process.cwd(), "drizzle"),
    join(process.cwd(), "apps/dashboard/drizzle"),
  ].filter((p): p is string => Boolean(p));
  const found = candidates.find((p) => existsSync(join(p, "meta", "_journal.json")));
  if (!found) throw new Error(`No migrations found; looked in ${candidates.join(", ")}`);
  return resolve(found);
}

function open() {
  if (config.databaseFile !== ":memory:")
    mkdirSync(dirname(config.databaseFile), { recursive: true });
  const sqlite = new Database(config.databaseFile, { create: true, strict: true });
  // WAL so the ingest writer and page reads do not block each other.
  sqlite.exec("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;");
  const db = drizzle(sqlite, { schema });
  migrate(db, { migrationsFolder: migrationsFolder() });
  return db;
}

type Db = ReturnType<typeof open>;

// One connection per process, kept across dev reloads.
const holder = globalThis as unknown as { __daintyDb?: Db };
export const db: Db = holder.__daintyDb ?? open();
holder.__daintyDb = db;

/**
 * bun:sqlite commits when the synchronous callback returns, so the body must not await: an async
 * body would commit before its first await. Gather what the transaction needs first.
 */
export function tx<T>(fn: (t: Parameters<Parameters<Db["transaction"]>[0]>[0]) => T): T {
  return db.transaction(fn);
}

export { schema };
