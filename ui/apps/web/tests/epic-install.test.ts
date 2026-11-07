import { expect, test } from "bun:test";
import type { GameVersionEntry, InstallInfo, TaskProgress } from "@dainty/bridge";
import { installProgress, sameFolder, versionActions } from "@/components/launcher/EpicInstall";

const entry: GameVersionEntry = {
  version: "1.14.7",
  displayName: "Rustic Isles 1.14.7",
  origin: "launcher",
  install: "epic",
  epic: { buildVersion: "1.14.7.647472", manifest: "ready", downloadBytes: 1, installBytes: 2 },
};
const info = (state: InstallInfo["state"], progress: TaskProgress | null = null): InstallInfo => ({
  version: "1.14.7",
  state,
  folder: "D:/Games/1.14.7",
  progress,
  error: null,
});
const ctx = { signedIn: true, owned: true, otherRunning: false, inUse: false };
const p = (totalBytes: number, doneBytes: number): TaskProgress => ({
  kind: "install",
  totalBytes,
  doneBytes,
  filesTotal: 1,
  filesDone: 0,
  bytesPerSecond: 0,
  etaSeconds: null,
  currentFile: null,
});

test("an install needs a signed-in account that owns the game", () => {
  expect(versionActions(entry, null, ctx)).toEqual({ actions: ["install"], hint: null });
  expect(versionActions(entry, null, { ...ctx, signedIn: false }).hint).toBe("signInFirst");
  expect(versionActions(entry, null, { ...ctx, owned: false }).hint).toBe("notOwned");
  expect(versionActions(entry, null, { ...ctx, otherRunning: true }).hint).toBe("otherRunning");
});

test("each state offers its own buttons", () => {
  expect(versionActions(entry, info("downloading"), ctx).actions).toEqual(["pause", "cancel"]);
  expect(versionActions(entry, info("paused"), ctx).actions).toEqual(["resume", "cancel"]);
  expect(versionActions(entry, info("finishing"), ctx).actions).toEqual([]);
  expect(versionActions(entry, info("resumable"), ctx).actions).toEqual(["continue"]);
  expect(versionActions(entry, info("failed"), ctx).actions).toEqual(["retry"]);
  expect(versionActions(entry, info("installed"), ctx).actions).toEqual(["use"]);
  expect(versionActions(entry, info("installed"), { ...ctx, inUse: true }).actions).toEqual([]);
});

test("a manifest that is not ready offers no install, but a part-way one can continue", () => {
  const bad = { ...entry, epic: { ...entry.epic!, manifest: "unreachable" as const } };
  expect(versionActions(bad, null, ctx).actions).toEqual([]);
  expect(versionActions(bad, info("resumable"), ctx).actions).toEqual(["continue"]);
  expect(versionActions({ ...entry, install: "locate", epic: null }, null, ctx).actions).toEqual(
    [],
  );
});

test("the channel's progress counts only for the same download", () => {
  expect(installProgress(info("downloading", p(10, 1)), p(10, 5))?.doneBytes).toBe(5);
  expect(installProgress(info("downloading", p(10, 1)), p(99, 5))?.doneBytes).toBe(1);
  expect(installProgress(info("installed"), p(10, 5))).toBeNull();
});

test("folders compare across slashes and drive-letter case", () => {
  expect(sameFolder("C:\\Games\\1.14.7\\", "c:/games/1.14.7")).toBe(true);
  expect(sameFolder("/home/a", "/home/A")).toBe(false);
  expect(sameFolder(null, "C:\\")).toBe(false);
});
