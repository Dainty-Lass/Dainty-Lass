import { expect, test } from "bun:test";
import { BADGE_HOURS, hoursOf, nextBadge, unlockedBadges } from "@/components/launcher/playtime";
import launcherEn from "../messages/launcher/en.json";

const H = 3600;

test("badges unlock exactly at their hour marks", () => {
  expect(unlockedBadges(0)).toEqual([]);
  expect(unlockedBadges(H - 1)).toEqual([]);
  expect(unlockedBadges(H)).toEqual([1]);
  expect(unlockedBadges(10 * H - 1)).toEqual([1]);
  expect(unlockedBadges(10 * H)).toEqual([1, 10]);
  expect(unlockedBadges(999 * H)).toEqual([1, 10, 25, 50, 100, 250, 500, 750]);
  expect(unlockedBadges(1000 * H)).toEqual([...BADGE_HOURS]);
  for (const mark of BADGE_HOURS) {
    expect(unlockedBadges(mark * H)).toContain(mark);
    expect(unlockedBadges(mark * H - 1)).not.toContain(mark);
  }
});

test("the next badge and the hours left to it", () => {
  expect(nextBadge(0)).toEqual({ mark: 1, hoursLeft: 1 });
  expect(nextBadge(H)).toEqual({ mark: 10, hoursLeft: 9 });
  expect(nextBadge(9 * H + 1)).toEqual({ mark: 10, hoursLeft: 1 });
  expect(nextBadge(749 * H)).toEqual({ mark: 750, hoursLeft: 1 });
  expect(nextBadge(1000 * H)).toBeNull();
});

test("bad totals count as none", () => {
  for (const bad of [-5, Number.NaN, Number.POSITIVE_INFINITY]) {
    expect(hoursOf(bad)).toBe(0);
    expect(unlockedBadges(bad)).toEqual([]);
  }
});

test("every badge has a name", () => {
  const names = launcherEn.playtime.badges as Record<string, string>;
  expect(Object.keys(names).sort()).toEqual(BADGE_HOURS.map((h) => `h${h}`).sort());
});
