import { createHash, randomBytes, randomInt, randomUUID } from "node:crypto";
import { normalizePairingCode, PAIRING_ALPHABET, PAIRING_TTL_MS } from "@dainty/fleet";
import { and, asc, eq, gt } from "drizzle-orm";
import { type Actor, recordAudit } from "@/lib/audit/record";
import { db, schema, tx } from "@/lib/db";
import type { Controller } from "@/lib/db/schema";
import { DomainError } from "@/lib/errors";
import { getSetting } from "@/lib/settings";

export const LOCAL = "local";

const sha = (text: string) => createHash("sha256").update(text).digest("hex");

/** The controller the dashboard runs on always exists, named after the server until someone renames it. */
export function ensureLocal(): Controller {
  const existing = db
    .select()
    .from(schema.controllers)
    .where(eq(schema.controllers.id, LOCAL))
    .get();
  if (existing) return existing;
  return (
    db
      .insert(schema.controllers)
      .values({ id: LOCAL, kind: "local", name: getSetting("serverName") })
      .onConflictDoNothing()
      .returning()
      .get() ?? db.select().from(schema.controllers).where(eq(schema.controllers.id, LOCAL)).get()!
  );
}

export function listControllers(): Controller[] {
  ensureLocal();
  return db
    .select()
    .from(schema.controllers)
    .orderBy(asc(schema.controllers.kind), asc(schema.controllers.name))
    .all();
}

export function getController(id: string): Controller | undefined {
  if (id === LOCAL) return ensureLocal();
  return db.select().from(schema.controllers).where(eq(schema.controllers.id, id)).get();
}

export function requireController(id: string): Controller {
  const c = getController(id);
  if (!c) throw new DomainError("not_found");
  if (c.disabled) throw new DomainError("controller_disabled");
  return c;
}

function pairingCode(): string {
  return Array.from({ length: 8 }, () => PAIRING_ALPHABET[randomInt(PAIRING_ALPHABET.length)]).join(
    "",
  );
}

export const formatCode = (code: string) => `${code.slice(0, 4)}-${code.slice(4)}`;

function cleanName(name: string): string {
  const n = name.trim().replace(/\s+/g, " ");
  if (n.length < 1 || n.length > 64) throw new DomainError("invalid_input");
  return n;
}

/** A controller waiting for its agent. The code is returned once and stored only as a hash. */
export function createController(
  actor: Actor,
  input: { name: string; region?: string | null },
): { controller: Controller; code: string } {
  const code = pairingCode();
  return tx((t) => {
    const controller = t
      .insert(schema.controllers)
      .values({
        id: randomUUID(),
        kind: "agent",
        name: cleanName(input.name),
        region: input.region?.trim().slice(0, 40) || null,
        pairingCodeHash: sha(code),
        pairingExpiresAt: Date.now() + PAIRING_TTL_MS,
      })
      .returning()
      .get();
    recordAudit(
      actor,
      {
        action: "controller.create",
        targetType: "controller",
        targetId: controller.id,
        targetLabel: controller.name,
        controllerId: controller.id,
        after: { name: controller.name, region: controller.region },
      },
      t,
    );
    return { controller, code };
  });
}

/** A new code for an existing controller: re-pairing a reinstalled host. The old token stops working. */
export function repairController(actor: Actor, id: string): string {
  const code = pairingCode();
  tx((t) => {
    const c = t.select().from(schema.controllers).where(eq(schema.controllers.id, id)).get();
    if (c?.kind !== "agent") throw new DomainError("not_found");
    t.update(schema.controllers)
      .set({
        tokenHash: null,
        pairingCodeHash: sha(code),
        pairingExpiresAt: Date.now() + PAIRING_TTL_MS,
      })
      .where(eq(schema.controllers.id, id))
      .run();
    recordAudit(
      actor,
      {
        action: "controller.repair",
        targetType: "controller",
        targetId: id,
        targetLabel: c.name,
        controllerId: id,
      },
      t,
    );
  });
  return code;
}

/** Redeems a code: single use, and only within its window. */
export function pairAgent(
  code: string,
  version: string,
): { agentId: string; token: string } | null {
  const hash = sha(normalizePairingCode(code));
  const token = `dtk_${randomBytes(32).toString("base64url")}`;
  return tx((t) => {
    const c = t
      .select()
      .from(schema.controllers)
      .where(
        and(
          eq(schema.controllers.pairingCodeHash, hash),
          gt(schema.controllers.pairingExpiresAt, Date.now()),
        ),
      )
      .get();
    if (!c || c.disabled) return null;
    t.update(schema.controllers)
      .set({
        tokenHash: sha(token),
        pairingCodeHash: null,
        pairingExpiresAt: null,
        pairedAt: Date.now(),
        version: version.slice(0, 32),
      })
      .where(eq(schema.controllers.id, c.id))
      .run();
    recordAudit(
      { ingest: "agent" },
      {
        action: "controller.pair",
        targetType: "controller",
        targetId: c.id,
        targetLabel: c.name,
        controllerId: c.id,
        after: { version },
      },
      t,
    );
    return { agentId: c.id, token };
  });
}

/** The agent a bearer token belongs to, or null. Looked up by hash, so timing reveals nothing useful. */
export function authenticateAgent(request: Request): Controller | null {
  const token = request.headers.get("authorization")?.replace(/^Bearer\s+/i, "") ?? "";
  if (!token.startsWith("dtk_")) return null;
  const c = db
    .select()
    .from(schema.controllers)
    .where(eq(schema.controllers.tokenHash, sha(token)))
    .get();
  return c && !c.disabled ? c : null;
}

export function updateController(
  actor: Actor,
  id: string,
  change: { name?: string; region?: string | null; note?: string | null; disabled?: boolean },
  revertOfId?: number,
): Controller {
  return tx((t) => {
    const before = t.select().from(schema.controllers).where(eq(schema.controllers.id, id)).get();
    if (!before) throw new DomainError("not_found");
    if (id === LOCAL && change.disabled) throw new DomainError("invalid_input");
    const patch = {
      ...(change.name !== undefined ? { name: cleanName(change.name) } : {}),
      ...(change.region !== undefined
        ? { region: change.region?.trim().slice(0, 40) || null }
        : {}),
      ...(change.note !== undefined ? { note: change.note?.trim().slice(0, 300) || null } : {}),
      ...(change.disabled !== undefined ? { disabled: change.disabled } : {}),
    };
    const after = t
      .update(schema.controllers)
      .set(patch)
      .where(eq(schema.controllers.id, id))
      .returning()
      .get();
    const pick = (c: Controller) => ({
      name: c.name,
      region: c.region,
      note: c.note,
      disabled: c.disabled,
    });
    recordAudit(
      actor,
      {
        action: "controller.update",
        targetType: "controller",
        targetId: id,
        targetLabel: after.name,
        controllerId: id,
        before: pick(before),
        after: pick(after),
        revertible: true,
        revertOfId,
      },
      t,
    );
    return after;
  });
}

/** Forgets an agent. Its rows (chat, reports, audit) stay, still labelled with its id. */
export function removeController(actor: Actor, id: string): void {
  if (id === LOCAL) throw new DomainError("invalid_input");
  tx((t) => {
    const c = t.select().from(schema.controllers).where(eq(schema.controllers.id, id)).get();
    if (!c) throw new DomainError("not_found");
    t.delete(schema.controllers).where(eq(schema.controllers.id, id)).run();
    t.delete(schema.controllerSamples).where(eq(schema.controllerSamples.controllerId, id)).run();
    recordAudit(
      actor,
      {
        action: "controller.remove",
        targetType: "controller",
        targetId: id,
        targetLabel: c.name,
        controllerId: id,
        before: { name: c.name, region: c.region },
      },
      t,
    );
  });
}

/** Names for ids, for labelling rows from controllers that may since have been removed. */
export function controllerNames(): Record<string, string> {
  return Object.fromEntries(listControllers().map((c) => [c.id, c.name]));
}
