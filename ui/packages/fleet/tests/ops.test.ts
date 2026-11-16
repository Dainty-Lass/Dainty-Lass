import { expect, test } from "bun:test";
import { backendAllowed, isAdminOp, normalizePairingCode, runBackend } from "../src";

test("only the listed backend routes pass", () => {
  expect(backendAllowed("GET", "/admin/v1/players?page=2&limit=25")).toBe(true);
  expect(backendAllowed("PATCH", "/admin/v1/players/abc-123/access")).toBe(true);
  expect(backendAllowed("DELETE", "/admin/v1/players/abc-123")).toBe(false);
  expect(backendAllowed("GET", "/admin/v1/grant-catalog")).toBe(true);
  expect(backendAllowed("POST", "/admin/v1/players/abc-123/grants")).toBe(true);
  expect(backendAllowed("GET", "/admin/v1/players/abc-123/grants")).toBe(false);
  expect(backendAllowed("POST", "/admin/v1/players/abc-123/tester-max")).toBe(false);
  expect(backendAllowed("GET", "/admin/v1/player-logs")).toBe(false);
  expect(backendAllowed("GET", "/admin/v1/players/../auth/login")).toBe(false);
  expect(backendAllowed("GET", "//evil.example/admin/v1/players")).toBe(false);
  expect(backendAllowed("GET", "/launcher/v1/me")).toBe(false);
});

test("admin operations are looked up by name, never taken as a document", () => {
  expect(isAdminOp("ban")).toBe(true);
  expect(isAdminOp("{ __schema { types { name } } }")).toBe(false);
  expect(isAdminOp("constructor")).toBe(false);
});

test("a refused route never reaches the network", async () => {
  const r = await runBackend(
    { adminApiUrl: "", backendUrl: "http://127.0.0.1:1", operatorSecret: "s".repeat(40) },
    "DELETE",
    "/admin/v1/players/x",
    { id: "1", name: "a" },
  );
  expect(r).toMatchObject({ ok: false, code: "forbidden_call" });
});

test("pairing codes read the way people type them", () => {
  expect(normalizePairingCode("abcd-efgh ")).toBe("ABCDEFGH");
});
