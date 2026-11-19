// The hour marks a badge unlocks at, in order; each has a name under launcher.playtime.badges.
export const BADGE_HOURS = [1, 10, 25, 50, 100, 250, 500, 750, 1000] as const;
export type BadgeHours = (typeof BADGE_HOURS)[number];

const HOUR = 3600;

/** Whole hours in `seconds`; anything not a finite non-negative number is none. */
export function hoursOf(seconds: number): number {
  return Number.isFinite(seconds) && seconds > 0 ? Math.floor(seconds / HOUR) : 0;
}

/** The badges `seconds` of playtime has unlocked. */
export function unlockedBadges(seconds: number): BadgeHours[] {
  const hours = hoursOf(seconds);
  return BADGE_HOURS.filter((mark) => hours >= mark);
}

/** The next badge and the whole hours still to play for it (rounded up); none once all are unlocked. */
export function nextBadge(seconds: number): { mark: BadgeHours; hoursLeft: number } | null {
  const hours = hoursOf(seconds);
  const mark = BADGE_HOURS.find((m) => hours < m);
  if (mark === undefined) return null;
  const played = Number.isFinite(seconds) && seconds > 0 ? seconds : 0;
  return { mark, hoursLeft: Math.ceil((mark * HOUR - played) / HOUR) };
}
