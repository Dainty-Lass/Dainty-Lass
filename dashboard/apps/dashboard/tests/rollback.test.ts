import { beforeAll, describe, expect, test } from "bun:test";
import { queryAudit } from "@/lib/audit/query";
import { revertEvent } from "@/lib/audit/revert";
import { can, canAssign } from "@/lib/auth/permissions";
import { ingestChat, searchChat } from "@/lib/chat/store";
import { createTerm, deleteTerm, listTerms, testText, updateTerm } from "@/lib/chat/terms";
import type { Staff } from "@/lib/db/schema";
import { DomainError } from "@/lib/errors";
import { claimReport, closeReport, createReport, getReport } from "@/lib/reports/store";
import { listRevisions, restoreRevision } from "@/lib/revisions";
import { getSettings } from "@/lib/settings";
import { updateSettings } from "@/lib/settings-change";
import { addStaff, resolveSignIn, updateStaff } from "@/lib/staff";

let owner: Staff;

beforeAll(() => {
  const outcome = resolveSignIn(
    { id: "100000000000000001", username: "owner", global_name: null, avatar: null },
    null,
  );
  if (!outcome.ok) throw new Error("owner sign-in refused");
  owner = outcome.staff;
});

const term = (pattern: string) => ({
  pattern,
  matchType: "word" as const,
  action: "mask" as const,
  category: "general",
  note: null,
  enabled: true,
  channels: null,
});
const lastEvent = () => queryAudit({ pageSize: 1 }).events[0]!;

describe("sign-in", () => {
  test("an environment owner is admitted; a stranger is not", () => {
    expect(owner.role).toBe("owner");
    expect(
      resolveSignIn(
        { id: "100000000000000099", username: "x", global_name: null, avatar: null },
        null,
      ),
    ).toEqual({ ok: false, reason: "not_staff" });
  });

  test("a guild role admits and is re-read at the next sign-in", () => {
    const user = {
      id: "100000000000000050",
      username: "guildmod",
      global_name: null,
      avatar: null,
    };
    const first = resolveSignIn(user, "moderator");
    expect(first.ok && first.staff.role).toBe("moderator");
    expect(resolveSignIn(user, null)).toEqual({ ok: false, reason: "not_staff" });
  });
});

describe("chat filter rollback", () => {
  test("create, edit and delete each revert to what was there", async () => {
    const created = createTerm(owner, term("darn"));
    expect(testText("darn it").verdict).toBe("masked");

    updateTerm(owner, created.id, { ...term("darn"), action: "block" });
    expect(testText("darn it").verdict).toBe("blocked");
    await revertEvent(owner, lastEvent().id);
    expect(testText("darn it").verdict).toBe("masked");

    deleteTerm(owner, created.id);
    expect(testText("darn it").verdict).toBe("allowed");
    const deletion = lastEvent();
    await revertEvent(owner, deletion.id);
    expect(listTerms().map((t) => t.id)).toContain(created.id);
    await expect(revertEvent(owner, deletion.id)).rejects.toThrow(DomainError);
  });

  test("restoring a revision brings back the whole list and is itself undoable", async () => {
    createTerm(owner, term("heck"));
    const before = listRevisions("chat_filter")[0]!;
    createTerm(owner, term("blast"));
    expect(listTerms()).toHaveLength(3);
    restoreRevision(before.id, owner);
    expect(
      listTerms()
        .map((t) => t.pattern)
        .sort(),
    ).toEqual(["darn", "heck"]);
    await revertEvent(owner, lastEvent().id);
    expect(listTerms()).toHaveLength(3);
  });

  test("ingested chat is rated by the current list", () => {
    ingestChat([
      {
        id: "t1",
        at: Date.now(),
        channel: "city",
        sender: { userId: "u9", name: "Tess" },
        body: "oh heck",
      },
    ]);
    expect(searchChat({ userId: "u9" }).messages[0]?.verdict).toBe("masked");
  });
});

describe("settings and staff rollback", () => {
  test("a settings change reverts to the previous values", async () => {
    updateSettings(owner, { serverName: "Renamed" });
    expect(getSettings().serverName).toBe("Renamed");
    await revertEvent(owner, lastEvent().id);
    expect(getSettings().serverName).toBe("DaintyLass");
  });

  test("the last owner cannot be demoted or disabled, and nobody demotes themselves", () => {
    const mod = addStaff(owner, {
      discordId: "100000000000000002",
      username: "mod",
      role: "moderator",
    });
    expect(() => updateStaff(owner, owner.id, { role: "admin" })).toThrow(DomainError);
    expect(() => updateStaff(mod, owner.id, { disabled: true })).toThrow(DomainError);
  });

  test("a role change reverts", async () => {
    const helper = addStaff(owner, {
      discordId: "100000000000000003",
      username: "helper",
      role: "viewer",
    });
    updateStaff(owner, helper.id, { role: "admin" });
    await revertEvent(owner, lastEvent().id);
    expect(
      queryAudit({ targetType: "staff", targetId: String(helper.id), pageSize: 1 }).events[0]
        ?.after,
    ).toMatchObject({ role: "viewer" });
  });
});

describe("reports", () => {
  test("workflow changes are audited and revertible", async () => {
    const { report } = createReport(owner, {
      source: "game",
      externalId: "x1",
      target: { userId: "u2", name: "Grevel" },
      category: "cheating",
      description: "speed hacking",
    });
    expect(report.priority).toBe("high");
    expect(
      createReport(owner, {
        source: "game",
        externalId: "x1",
        target: { name: "Grevel" },
        category: "other",
        description: "dupe",
      }).duplicate,
    ).toBe(true);
    claimReport(owner, report.id);
    closeReport(owner, report.id, "dismissed", "No evidence in logs");
    expect(getReport(report.id)?.status).toBe("dismissed");
    await revertEvent(owner, lastEvent().id);
    expect(getReport(report.id)?.status).toBe("claimed");
  });
});

describe("permissions", () => {
  test("roles are ranked and owners alone make owners", () => {
    expect(can("viewer", "players:moderate")).toBe(false);
    expect(can("moderator", "players:moderate")).toBe(true);
    expect(can("admin", "settings:write")).toBe(false);
    expect(canAssign("admin", "owner")).toBe(false);
    expect(canAssign("owner", "owner")).toBe(true);
    expect(canAssign("moderator", "viewer")).toBe(false);
  });
});
