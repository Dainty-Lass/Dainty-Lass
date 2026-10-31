/** English is the one catalog today; a language is one more file in messages/ and an entry here. */
export const LOCALE = "en" as const;
export type Locale = typeof LOCALE;

/** The browser writes its zone here (TimeZoneSync) so the server formats timestamps as the reader sees them. */
export const TIME_ZONE_COOKIE = "dainty-tz";

export function resolveTimeZone(value: string | undefined): string {
  if (!value) return "UTC";
  try {
    new Intl.DateTimeFormat("en", { timeZone: value });
    return value;
  } catch {
    return "UTC";
  }
}
