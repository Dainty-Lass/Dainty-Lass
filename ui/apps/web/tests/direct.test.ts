import { beforeAll, describe, expect, test } from "bun:test";
import { queryAudit } from "@/lib/audit/query";
import { readDirectMessages } from "@/lib/chat/direct";
import { chatAround, ingestChat, searchChat } from "@/lib/chat/store";
import type { Staff } from "@/lib/db/schema";
import { DomainError } from "@/lib/errors";
import { identity, signedIn } from "./helpers";

let owner: Staff;
const at = Date.now();

beforeAll(() => {
  owner = signedIn(identity({ name: "owner", discordId: "100000000000000001" }));
  ingestChat([
    {
      id: "dm-c",
      at,
      channel: "city",
      roomId: "dm-room",
      sender: { userId: "dm1", name: "Vale" },
      body: "hello all",
    },
    {
      id: "dm-d",
      at: at + 1,
      channel: "direct",
      sender: { userId: "dm1", name: "Vale" },
      recipient: { userId: "dm2", name: "Brook" },
      body: "just between us",
    },
  ]);
});

describe("direct messages", () => {
  test("stay out of every chat query", () => {
    expect(searchChat({ userId: "dm1" }).messages.map((m) => m.channel)).toEqual(["city"]);
    expect(searchChat({ userId: "dm2" }).messages).toHaveLength(0);
    expect(searchChat({ channel: "direct" }).messages).toHaveLength(0);
    expect(
      chatAround(at, 5, { controllerId: "local", userId: "dm1" }).map((m) => m.channel),
    ).toEqual(["city"]);
  });

  test("are read only with a reason, and each read is audited", () => {
    expect(() =>
      readDirectMessages(owner, {
        controllerId: "local",
        userId: "dm2",
        name: "Brook",
        reason: " ",
      }),
    ).toThrow(DomainError);

    const read = readDirectMessages(owner, {
      controllerId: "local",
      userId: "dm2",
      name: "Brook",
      reason: "Report #4",
      reportId: 4,
    });
    expect(read.lines.map((m) => m.body)).toEqual(["just between us"]);
    const event = queryAudit({ action: "chat.direct_read", pageSize: 1 }).events[0]!;
    expect(event.id).toBe(read.eventId);
    expect(event).toMatchObject({ targetType: "player", targetId: "dm2", reason: "Report #4" });
    expect(event.after).toEqual({ rows: 1, reportId: 4 });
    expect(event.revertible).toBe(false);
  });
});
