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
import { addStaff, updateStaff } from "@/lib/staff";
import { identity, signedIn } from "./helpers";

let owner: Staff;

beforeAll(() => {
  owner = signedIn(identity({ name: "owner", discordId: "100000000000000001" }));
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
      role: "user",
    });
    updateStaff(owner, helper.id, { role: "admin" });
    await revertEvent(owner, lastEvent().id);
    expect(
      queryAudit({ targetType: "staff", targetId: String(helper.id), pageSize: 1 }).events[0]
        ?.after,
    ).toMatchObject({ role: "user" });
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
    expect(can("user", "players:moderate")).toBe(false);
    expect(can("moderator", "players:moderate")).toBe(true);
    expect(can("admin", "settings:write")).toBe(false);
    expect(canAssign("admin", "owner")).toBe(false);
    expect(canAssign("owner", "owner")).toBe(true);
    expect(canAssign("moderator", "user")).toBe(false);
  });
});
