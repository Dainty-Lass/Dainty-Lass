import { describe, expect, test } from "bun:test";
import { eq } from "drizzle-orm";
import { standingFromGuild } from "@/lib/auth/discord";
import { can } from "@/lib/auth/permissions";
import { signInMethod, unlinkAccount, waysIn } from "@/lib/auth/sign-in";
import { db, schema } from "@/lib/db";
import { DomainError } from "@/lib/errors";
import { completeOnboarding, needsOnboarding } from "@/lib/onboarding";
import { getSettings } from "@/lib/settings";
import { updateSettings } from "@/lib/settings-change";
import { updateStaff } from "@/lib/staff";
import { identity, signedIn, signIn } from "./helpers";

const owner = signedIn(identity({ name: "owner", discordId: "100000000000000001" }));
const map = { admin: ["1"], moderator: ["2"], user: ["3"] };

describe("tiers", () => {
  test("users see players but not reports, chat or the audit log; moderators lack only the audit log", () => {
    for (const c of ["overview:read", "players:read", "controllers:read"] as const)
      expect(can("user", c)).toBe(true);
    for (const c of ["reports:read", "chat:read", "audit:read"] as const)
      expect(can("user", c)).toBe(false);
    expect(can("moderator", "reports:read")).toBe(true);
    expect(can("moderator", "chat:read")).toBe(true);
    expect(can("moderator", "audit:read")).toBe(false);
    expect(can("admin", "audit:read")).toBe(true);
  });

  test("anyone may sign in, as a plain user until something grants more", () => {
    const m = signedIn(identity({ name: "stranger", email: "stranger@example.com" }));
    expect(m).toMatchObject({ role: "user", roleSource: "default", discordId: null });
  });

  test("a guild role sets the tier and is re-read; an unreadable guild keeps the last tier", () => {
    const id = identity({ name: "guildmod", discordId: "100000000000000050" });
    expect(signedIn(id, standingFromGuild(["2"], map, "")).role).toBe("moderator");
    expect(signedIn(id, undefined).role).toBe("moderator");
    expect(signedIn(id, standingFromGuild(["3"], map, "")).role).toBe("user");
    expect(signedIn(id, null)).toMatchObject({ role: "user", roleSource: "default" });
  });

  test("a role set here outlives the guild until the member follows Discord again", () => {
    const id = identity({ name: "pinned", discordId: "100000000000000051" });
    const m = signedIn(id, standingFromGuild(["3"], map, ""));
    updateStaff(owner, m.id, { role: "admin" });
    expect(signedIn(id, null).role).toBe("admin");
    updateStaff(owner, m.id, { roleSource: "guild" });
    expect(signedIn(id, standingFromGuild(["2"], map, "")).role).toBe("moderator");
  });

  test("a disabled member is refused", () => {
    const id = identity({ name: "gone" });
    const m = signedIn(id);
    updateStaff(owner, m.id, { disabled: true });
    expect(signIn(id)).toEqual({ ok: false, reason: "disabled" });
  });

  test("the approved guild role approves", () => {
    const m = signedIn(
      identity({ name: "member", discordId: "100000000000000052" }),
      standingFromGuild(["9"], map, "9"),
    );
    expect(m.approvedVia).toBe("guild");
  });
});

describe("onboarding", () => {
  test("only unapproved users wait, and accepting the terms approves them", async () => {
    updateSettings(owner, { requireApproval: true, tosText: "Be kind." });
    const s = getSettings();
    const newcomer = signedIn(identity({ name: "newcomer" }));
    expect(needsOnboarding(newcomer, s)).toBe(true);
    expect(needsOnboarding(owner, s)).toBe(false);

    await expect(
      completeOnboarding(newcomer, {
        displayName: "New Comer",
        gameName: "",
        acceptedVersion: s.tosVersion - 1,
      }),
    ).rejects.toBeInstanceOf(DomainError);
    await expect(
      completeOnboarding(newcomer, {
        displayName: "x",
        gameName: "",
        acceptedVersion: s.tosVersion,
      }),
    ).rejects.toBeInstanceOf(DomainError);
    expect(
      await completeOnboarding(newcomer, {
        displayName: "New Comer",
        gameName: "new_comer",
        acceptedVersion: s.tosVersion,
      }),
    ).toEqual({ roleAdded: null });
    const after = db.select().from(schema.staff).where(eq(schema.staff.id, newcomer.id)).get()!;
    expect(after).toMatchObject({
      displayName: "New Comer",
      gameName: "new_comer",
      approvedVia: "tos",
    });
    expect(needsOnboarding(after, s)).toBe(false);

    // New terms ask again of those who accepted, not of those approved another way.
    updateSettings(owner, { tosText: "Be kind. Be fair." });
    const s2 = getSettings();
    expect(s2.tosVersion).toBe(s.tosVersion + 1);
    expect(needsOnboarding(after, s2)).toBe(true);
    const byHand = signedIn(identity({ name: "vouched" }));
    updateStaff(owner, byHand.id, { approvedAt: Date.now(), approvedVia: "manual" });
    const vouched = db.select().from(schema.staff).where(eq(schema.staff.id, byHand.id)).get()!;
    expect(needsOnboarding(vouched, s2)).toBe(false);
    updateSettings(owner, { requireApproval: false });
  });
});

describe("sign-in methods", () => {
  test("names how a session started", () => {
    expect(signInMethod("/callback/discord")).toBe("discord");
    expect(signInMethod("/passkey/verify-authentication")).toBe("passkey");
  });

  test("the last way in cannot be unlinked, and unlinking Discord drops the Discord id", () => {
    const id = identity({ name: "solo", discordId: "100000000000000060" });
    const m = signedIn(id);
    expect(m.discordId).toBe("100000000000000060");
    const discord = db
      .select()
      .from(schema.authAccounts)
      .where(eq(schema.authAccounts.userId, id.userId))
      .get()!;
    expect(waysIn(db, id.userId)).toBe(1);
    expect(() => unlinkAccount(m, discord.id)).toThrow(DomainError);

    const now = new Date();
    db.insert(schema.authAccounts)
      .values({
        userId: id.userId,
        providerId: "twitch",
        accountId: "t1",
        createdAt: now,
        updatedAt: now,
      })
      .run();
    unlinkAccount(m, discord.id);
    expect(
      db.select().from(schema.staff).where(eq(schema.staff.id, m.id)).get()?.discordId,
    ).toBeNull();
  });
});
