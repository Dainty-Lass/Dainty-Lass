import { describe, expect, test } from "bun:test";
import { POST } from "@/app/api/ingest/crash-reports/route.dashboard";
import { MAX_CRASH_TEXT, parseCrashReport } from "@/lib/crash-reports/parse";
import {
  createCrashReport,
  getCrashReport,
  KEEP_CRASH_REPORTS,
  listCrashReports,
} from "@/lib/crash-reports/store";

let n = 0;
const hexId = () => (++n).toString(16).padStart(32, "0");
const sha = "a".repeat(64);

function valid(over: Record<string, unknown> = {}) {
  return {
    id: hexId(),
    receivedAt: 1_795_000_000_000,
    account: { userId: "u1", name: "Lass" },
    crashId: "2026-11-18_12-00-00",
    gameVersion: "1.14.7",
    launcherVersion: null,
    exitCode: "0xC0000005",
    serverPath: "/srv/crash-reports/u1/2026-11-18_12-00-00",
    files: [
      { name: "crash.log", size: 12, sha256: sha, text: "boom\n" },
      { name: "dump.dmp", size: 4096, sha256: sha, text: null },
    ],
    ...over,
  };
}

const post = (body: unknown, key: string | null = "k".repeat(40)) =>
  POST(
    new Request("http://dash.test/api/ingest/crash-reports", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        ...(key ? { authorization: `Bearer ${key}` } : {}),
      },
      body: JSON.stringify(body),
    }),
  );

describe("parseCrashReport", () => {
  test("accepts the content service's shape", () => {
    const body = valid();
    const parsed = parseCrashReport(body);
    expect(parsed).toMatchObject({
      id: body.id,
      account: { userId: "u1", name: "Lass" },
      launcherVersion: null,
      files: [
        { name: "crash.log", text: "boom\n" },
        { name: "dump.dmp", text: null },
      ],
    });
  });

  test("refuses a malformed id, hash, oversize text or too many files", () => {
    expect(parseCrashReport(valid({ id: "A".repeat(32) }))).toBeNull();
    expect(parseCrashReport(valid({ id: "abc" }))).toBeNull();
    const file = (over: Record<string, unknown>) => ({
      name: "crash.log",
      size: 1,
      sha256: sha,
      text: "x",
      ...over,
    });
    expect(parseCrashReport(valid({ files: [file({ sha256: "A".repeat(64) })] }))).toBeNull();
    expect(parseCrashReport(valid({ files: [file({ sha256: "a".repeat(63) })] }))).toBeNull();
    expect(
      parseCrashReport(valid({ files: [file({ text: "x".repeat(MAX_CRASH_TEXT + 1) })] })),
    ).toBeNull();
    expect(parseCrashReport(valid({ files: [file({ size: -1 })] }))).toBeNull();
    expect(
      parseCrashReport(valid({ files: Array.from({ length: 17 }, () => file({})) })),
    ).toBeNull();
    expect(
      parseCrashReport(valid({ files: Array.from({ length: 16 }, () => file({})) })),
    ).not.toBeNull();
  });

  test("refuses missing or oversize fields", () => {
    expect(parseCrashReport(null)).toBeNull();
    expect(parseCrashReport(valid({ account: undefined }))).toBeNull();
    expect(parseCrashReport(valid({ account: { userId: "u1", name: "x".repeat(65) } }))).toBeNull();
    expect(parseCrashReport(valid({ crashId: "" }))).toBeNull();
    expect(parseCrashReport(valid({ exitCode: "x".repeat(33) }))).toBeNull();
    expect(parseCrashReport(valid({ gameVersion: 7 }))).toBeNull();
    expect(parseCrashReport(valid({ serverPath: "x".repeat(513) }))).toBeNull();
    expect(parseCrashReport(valid({ receivedAt: "yesterday" }))).toBeNull();
    expect(parseCrashReport(valid({ files: "none" }))).toBeNull();
  });
});

describe("POST /api/ingest/crash-reports", () => {
  test("refuses without the ingest key", async () => {
    expect((await post(valid(), null)).status).toBe(401);
    expect((await post(valid(), "w".repeat(40))).status).toBe(401);
  });

  test("files a report once, and a retry is a duplicate", async () => {
    const body = valid();
    const first = await post(body);
    expect(first.status).toBe(201);
    expect(await first.json()).toEqual({ id: body.id, duplicate: false });
    const again = await post(body);
    expect(again.status).toBe(200);
    expect(await again.json()).toEqual({ id: body.id, duplicate: true });
    expect(getCrashReport(body.id)?.files).toHaveLength(2);
    expect(listCrashReports().find((r) => r.id === body.id)?.fileCount).toBe(2);
  });

  test("answers 400 to a malformed body", async () => {
    const res = await post(valid({ id: "nope" }));
    expect(res.status).toBe(400);
    expect(await res.json()).toEqual({ error: "bad_request" });
  });
});

test("keeps only the newest reports", () => {
  const ids: string[] = [];
  for (let i = 0; i < KEEP_CRASH_REPORTS + 5; i++) {
    const parsed = parseCrashReport(valid());
    if (!parsed) throw new Error("fixture refused");
    ids.push(createCrashReport(parsed).report.id);
  }
  expect(listCrashReports({ limit: 10_000 })).toHaveLength(KEEP_CRASH_REPORTS);
  expect(getCrashReport(ids[0]!)).toBeUndefined();
  expect(getCrashReport(ids.at(-1)!)).toBeDefined();
});
