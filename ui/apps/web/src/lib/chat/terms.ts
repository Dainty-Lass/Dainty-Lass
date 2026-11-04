import { asc, eq } from "drizzle-orm";
import { type Actor, recordAudit } from "@/lib/audit/record";
import { db, schema, tx } from "@/lib/db";
import type { BlacklistTerm } from "@/lib/db/schema";
import { DomainError } from "@/lib/errors";
import { ensureBaseline, writeRevision } from "@/lib/revisions";
import { compile, findMatches, maskText, validatePattern, verdictOf } from "./filter";

export const CHANNELS = ["city", "party", "guild", "direct"] as const;

export type TermInput = Pick<
  BlacklistTerm,
  "pattern" | "matchType" | "action" | "category" | "note" | "enabled"
> & {
  channels: string[] | null;
};

function check(input: TermInput): TermInput {
  const pattern = input.pattern.trim();
  if (!validatePattern(pattern, input.matchType)) {
    throw new DomainError(input.matchType === "regex" ? "regex_invalid" : "invalid_input");
  }
  const channels =
    input.channels?.filter((c) => (CHANNELS as readonly string[]).includes(c)) ?? null;
  return {
    ...input,
    pattern,
    category: input.category.trim().slice(0, 40) || "general",
    note: input.note?.trim().slice(0, 300) || null,
    channels:
      channels && channels.length > 0 && channels.length < CHANNELS.length ? channels : null,
  };
}

export function listTerms(): BlacklistTerm[] {
  return db
    .select()
    .from(schema.blacklistTerms)
    .orderBy(asc(schema.blacklistTerms.category), asc(schema.blacklistTerms.pattern))
    .all();
}

/** The compiled list, rebuilt when the filter's revision changes rather than on every message. */
let cache: { version: number; terms: ReturnType<typeof compile> } | null = null;

export function filterVersion(): number {
  const row = db
    .select({ version: schema.configRevisions.version })
    .from(schema.configRevisions)
    .where(eq(schema.configRevisions.kind, "chat_filter"))
    .orderBy(schema.configRevisions.version)
    .all()
    .at(-1);
  return row?.version ?? 0;
}

export function compiledTerms() {
  const version = filterVersion();
  if (!cache || cache.version !== version) cache = { version, terms: compile(listTerms()) };
  return cache.terms;
}

export function testText(text: string, channel?: string) {
  const matches = findMatches(text, compiledTerms(), channel);
  return { matches, verdict: verdictOf(matches), masked: maskText(text, matches) };
}

function label(t: Pick<BlacklistTerm, "pattern">) {
  return t.pattern.length > 40 ? `${t.pattern.slice(0, 40)}…` : t.pattern;
}

export function createTerm(actor: Actor, raw: TermInput): BlacklistTerm {
  const input = check(raw);
  return tx((t) => {
    ensureBaseline("chat_filter", t);
    const dupe = t
      .select()
      .from(schema.blacklistTerms)
      .all()
      .find((x) => x.pattern === input.pattern && x.matchType === input.matchType);
    if (dupe) throw new DomainError("conflict");
    const term = t.insert(schema.blacklistTerms).values(input).returning().get();
    writeRevision("chat_filter", `add:${label(term)}`, actor, t);
    recordAudit(
      actor,
      {
        action: "filter.term.create",
        targetType: "filter_term",
        targetId: term.id,
        targetLabel: label(term),
        after: term,
        revertible: true,
      },
      t,
    );
    return term;
  });
}

export function updateTerm(
  actor: Actor,
  id: number,
  raw: TermInput,
  revertOfId?: number,
): BlacklistTerm {
  const input = check(raw);
  return tx((t) => {
    const before = t
      .select()
      .from(schema.blacklistTerms)
      .where(eq(schema.blacklistTerms.id, id))
      .get();
    if (!before) throw new DomainError("not_found");
    ensureBaseline("chat_filter", t);
    const after = t
      .update(schema.blacklistTerms)
      .set({ ...input, updatedAt: Date.now() })
      .where(eq(schema.blacklistTerms.id, id))
      .returning()
      .get();
    writeRevision("chat_filter", `edit:${label(after)}`, actor, t);
    recordAudit(
      actor,
      {
        action: "filter.term.update",
        targetType: "filter_term",
        targetId: id,
        targetLabel: label(after),
        before,
        after,
        revertible: true,
        revertOfId,
      },
      t,
    );
    return after;
  });
}

export function deleteTerm(actor: Actor, id: number, revertOfId?: number): void {
  tx((t) => {
    const before = t
      .select()
      .from(schema.blacklistTerms)
      .where(eq(schema.blacklistTerms.id, id))
      .get();
    if (!before) throw new DomainError("not_found");
    ensureBaseline("chat_filter", t);
    t.delete(schema.blacklistTerms).where(eq(schema.blacklistTerms.id, id)).run();
    writeRevision("chat_filter", `remove:${label(before)}`, actor, t);
    recordAudit(
      actor,
      {
        action: "filter.term.delete",
        targetType: "filter_term",
        targetId: id,
        targetLabel: label(before),
        before,
        revertible: true,
        revertOfId,
      },
      t,
    );
  });
}

/** Puts a deleted term back with its old id, so audit links to it keep working. */
export function recreateTerm(actor: Actor, term: BlacklistTerm, revertOfId: number): void {
  tx((t) => {
    if (t.select().from(schema.blacklistTerms).where(eq(schema.blacklistTerms.id, term.id)).get())
      throw new DomainError("conflict");
    ensureBaseline("chat_filter", t);
    t.insert(schema.blacklistTerms)
      .values({ ...term, updatedAt: Date.now() })
      .run();
    writeRevision("chat_filter", `add:${label(term)}`, actor, t);
    recordAudit(
      actor,
      {
        action: "filter.term.create",
        targetType: "filter_term",
        targetId: term.id,
        targetLabel: label(term),
        after: term,
        revertible: true,
        revertOfId,
      },
      t,
    );
  });
}
