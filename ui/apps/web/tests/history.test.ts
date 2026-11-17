import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { gameHistory } from "@/lib/audit/game-history";
import { queryAudit } from "@/lib/audit/query";
import { ingestChat } from "@/lib/chat/store";
import { createTerm, deleteTerm } from "@/lib/chat/terms";
import type { Staff } from "@/lib/db/schema";
import { identity, signedIn } from "./helpers";

let owner: Staff;
let termId = 0;
const at = Date.now();
const history = (userId: string) =>
  queryAudit({ targetType: "player", targetId: userId, pageSize: 50 }).events;

beforeAll(() => {
  owner = signedIn(identity({ name: "historian", discordId: "100000000000000077" }));
  termId = createTerm(owner, {
    pattern: "potato",
    matchType: "word",
    action: "mask",
    category: "general",
    note: null,
    enabled: true,
    channels: null,
  }).id;
});

// The database is shared by every test file: leave the filter as it was
afterAll(() => deleteTerm(owner, termId));

describe("a player's history", () => {
  test("names each filter hit, its verdict and term, never the line", () => {
    const batch = [
      {
        id: "hist-1",
        at,
        channel: "city" as const,
        roomId: "city",
        sender: { userId: "9c65cf9f-hist", name: "Spud" },
        body: "potato everywhere",
      },
      {
        id: "hist-2",
        at: at + 1,
        channel: "direct" as const,
        sender: { userId: "9c65cf9f-hist", name: "Spud" },
        recipient: { userId: "other", name: "Other" },
        body: "a secret potato",
      },
      {
        id: "hist-3",
        at: at + 2,
        channel: "city" as const,
        roomId: "city",
        sender: { userId: "9c65cf9f-hist", name: "Spud" },
        body: "hello there",
      },
    ];
    ingestChat(batch);
    // A retried batch stores nothing new, so it records nothing new
    ingestChat(batch);
    const events = history("9c65cf9f-hist");
    expect(events.map((e) => e.action)).toEqual(["chat.filter_masked", "chat.filter_masked"]);
    for (const e of events) {
      expect(e.revertible).toBe(false);
      expect(e.actorName).toBe("ingest:chat-filter");
      expect((e.after as { terms: string[] }).terms).toEqual(["potato"]);
      expect(JSON.stringify(e)).not.toContain("secret");
      expect(JSON.stringify(e)).not.toContain("everywhere");
    }
    expect(events.map((e) => (e.after as { channel: string }).channel).sort()).toEqual([
      "city",
      "direct",
    ]);
  });

  test("shows the game server's own record, but not what this dashboard already recorded", () => {
    const rows = gameHistory(
      [
        {
          action: "player.grant",
          actorUserId: "manager:1",
          targetUserId: "u1",
          createdAt: "2026-11-16T10:00:00Z",
        },
        {
          action: "player.roles",
          actorUserId: "boss-user",
          targetUserId: "u1",
          createdAt: "2026-11-16T11:00:00Z",
        },
        { action: "player.roles", actorUserId: "boss-user", targetUserId: "u2" },
      ],
      "u1",
      "Spud",
      "local",
    );
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({
      action: "player.roles",
      actorName: "boss-user",
      targetId: "u1",
      revertible: false,
    });
    expect(rows[0]!.id).toBeLessThan(0);
  });
});
