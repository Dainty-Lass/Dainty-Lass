import { expect, test } from "bun:test";
import { runCommand } from "../src/run";
import { readEvents } from "../src/sse";

const cfg = {
  adminApiUrl: "http://127.0.0.1:1/graphql",
  adminApiKey: "k",
  backendUrl: "http://127.0.0.1:1",
  operatorSecret: "s".repeat(40),
};

test("a GraphQL document from the dashboard is refused; only named operations run", async () => {
  const r = await runCommand(cfg, {
    id: "1",
    kind: "admin",
    op: "{ __schema { types { name } } }",
    variables: {},
    operatorId: "1",
  });
  expect(r?.result).toMatchObject({ ok: false, code: "forbidden_call" });
});

test("a backend route off the allowlist is refused on the host", async () => {
  const r = await runCommand(cfg, {
    id: "2",
    kind: "backend",
    method: "DELETE",
    target: "/admin/v1/players/x",
    operator: { id: "1", name: "a" },
  });
  expect(r?.result).toMatchObject({ ok: false, code: "forbidden_call" });
});

test("an unknown kind is refused, and a malformed command ignored", async () => {
  expect(
    (await runCommand(cfg, { id: "3", kind: "shell", cmd: "rm -rf /" }))?.result,
  ).toMatchObject({ ok: false, code: "forbidden_call" });
  expect(await runCommand(cfg, "nonsense")).toBeNull();
});

test("an allowed call that cannot reach the game reports it unavailable", async () => {
  const r = await runCommand(cfg, {
    id: "4",
    kind: "admin",
    op: "serverStatus",
    variables: {},
    operatorId: "0",
  });
  expect(r?.result).toMatchObject({ ok: false, code: "unavailable" });
});

test("the event reader splits events across chunk boundaries and drops comments", async () => {
  const chunks = [": ping\n\nevent: command\nda", 'ta: {"id":"a"}\n\nevent: hello\ndata: x\n\n'];
  const stream = new ReadableStream<Uint8Array>({
    start(c) {
      for (const s of chunks) c.enqueue(new TextEncoder().encode(s));
      c.close();
    },
  });
  const events = [];
  for await (const e of readEvents(stream)) events.push(e);
  expect(events).toEqual([
    { event: "command", data: '{"id":"a"}' },
    { event: "hello", data: "x" },
  ]);
});
