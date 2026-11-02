import { beforeAll, describe, expect, test } from "bun:test";
import { queryAudit } from "@/lib/audit/query";
import { revertEvent } from "@/lib/audit/revert";
import { ingestChat, searchChat } from "@/lib/chat/store";
import type { Staff } from "@/lib/db/schema";
import { DomainError } from "@/lib/errors";
import {
  authenticateAgent,
  createController,
  getController,
  pairAgent,
  repairController,
  updateController,
} from "@/lib/fleet/controllers";
import { attach, isConnected, sendCommand, settle } from "@/lib/fleet/registry";
import { recordStatus, totals, viewOf } from "@/lib/fleet/status";
import { game } from "@/lib/fleet/transport";
import { createReport, listReports } from "@/lib/reports/store";
import { identity, signedIn } from "./helpers";

let owner: Staff;

beforeAll(() => {
  owner = signedIn(identity({ name: "owner", discordId: "100000000000000001" }));
});

const bearer = (token: string) =>
  new Request("http://x/", { headers: { Authorization: `Bearer ${token}` } });

describe("pairing", () => {
  test("a code pairs once, and only its token authenticates", () => {
    const { controller, code } = createController(owner, { name: "EU 2", region: "eu-west" });
    expect(viewOf(getController(controller.id)!).state).toBe("pending");
    const paired = pairAgent(code.toLowerCase(), "0.1.0");
    expect(paired?.agentId).toBe(controller.id);
    expect(pairAgent(code, "0.1.0")).toBeNull();
    expect(authenticateAgent(bearer(paired!.token))?.id).toBe(controller.id);
    expect(authenticateAgent(bearer("dtk_wrong"))).toBeNull();
  });

  test("re-pairing revokes the old token", () => {
    const { controller, code } = createController(owner, { name: "US 1" });
    const first = pairAgent(code, "0.1.0")!;
    const second = pairAgent(repairController(owner, controller.id), "0.1.0")!;
    expect(authenticateAgent(bearer(first.token))).toBeNull();
    expect(authenticateAgent(bearer(second.token))?.id).toBe(controller.id);
  });

  test("a disabled controller's token stops working", () => {
    const { controller, code } = createController(owner, { name: "Temp" });
    const { token } = pairAgent(code, "0.1.0")!;
    updateController(owner, controller.id, { disabled: true });
    expect(authenticateAgent(bearer(token))).toBeNull();
  });
});

describe("commands", () => {
  test("a command reaches the connected agent and its answer comes back", async () => {
    const { controller, code } = createController(owner, { name: "Relay" });
    pairAgent(code, "0.1.0");
    const sent: { event: string; data: { id: string } }[] = [];
    const detach = attach(controller.id, {
      send: (event, data) => sent.push({ event, data: data as { id: string } }),
      close: () => {},
    });
    expect(isConnected(controller.id)).toBe(true);
    const pending = game(controller.id).admin<{ name: string }>("serverStatus", {}, "0");
    expect(sent[0]?.event).toBe("command");
    // Another agent cannot answer for this one.
    expect(settle("someone-else", sent[0]!.data.id, { ok: true, data: { name: "forged" } })).toBe(
      false,
    );
    settle(controller.id, sent[0]!.data.id, { ok: true, data: { name: "Relay server" } });
    expect(await pending).toEqual({ name: "Relay server" });
    detach();
  });

  test("a dropped stream fails what was waiting, and an offline agent fails at once", async () => {
    const { controller, code } = createController(owner, { name: "Flaky" });
    pairAgent(code, "0.1.0");
    const detach = attach(controller.id, { send: () => {}, close: () => {} });
    const pending = sendCommand(controller.id, {
      kind: "admin",
      op: "serverStatus",
      variables: {},
      operatorId: "0",
    });
    detach();
    expect(await pending).toMatchObject({ ok: false, code: "unavailable" });
    await expect(game(controller.id).admin("serverStatus", {}, "0")).rejects.toThrow(DomainError);
  });
});

describe("fleet status and scope", () => {
  test("reports roll up across controllers that are up", () => {
    const { controller, code } = createController(owner, { name: "Stats" });
    pairAgent(code, "0.1.0");
    const detach = attach(controller.id, { send: () => {}, close: () => {} });
    recordStatus(controller.id, {
      protocol: 1,
      version: "0.1.0",
      capabilities: ["admin", "backend"],
      at: Date.now(),
      adminApi: "ok",
      backend: "ok",
      server: null,
      hunts: { maxHuntServers: 8, running: 3, free: 5, hunts: [] },
      online: 12,
    });
    const view = viewOf(getController(controller.id)!);
    expect(view.state).toBe("online");
    expect(totals([view])).toMatchObject({ players: 12, huntsRunning: 3, huntsMax: 8 });
    detach();
    expect(viewOf(getController(controller.id)!).state).toBe("offline");
  });

  test("chat and reports stay with the controller they came from", () => {
    const a = createController(owner, { name: "A" }).controller.id;
    const b = createController(owner, { name: "B" }).controller.id;
    // The same stanza id on two controllers is two lines, not a duplicate.
    ingestChat(
      [
        {
          id: "s1",
          at: Date.now(),
          channel: "city",
          sender: { userId: "u1", name: "Moonwick" },
          body: "hi",
        },
      ],
      a,
    );
    ingestChat(
      [
        {
          id: "s1",
          at: Date.now(),
          channel: "city",
          sender: { userId: "u1", name: "Moonwick" },
          body: "hi",
        },
      ],
      b,
    );
    expect(searchChat({ controllerIds: [a] }).messages).toHaveLength(1);
    expect(searchChat({ userId: "u1" }).messages.length).toBeGreaterThanOrEqual(2);
    createReport(owner, {
      controllerId: b,
      source: "game",
      externalId: "r1",
      target: { name: "Grevel" },
      category: "spam",
      description: "spam",
    });
    expect(listReports({ controllerIds: [a] })).toHaveLength(0);
    expect(listReports({ controllerIds: [b] })).toHaveLength(1);
  });

  test("renaming a controller is audited against it and can be undone", async () => {
    const { controller } = createController(owner, { name: "Old name" });
    updateController(owner, controller.id, { name: "New name" });
    const ev = queryAudit({
      controllerIds: [controller.id],
      action: "controller.update",
      pageSize: 1,
    }).events[0]!;
    await revertEvent(owner, ev.id);
    expect(getController(controller.id)?.name).toBe("Old name");
  });
});
