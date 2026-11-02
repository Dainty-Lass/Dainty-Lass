// Client-safe: the report vocabulary without the database.
export const REPORT_STATUSES = ["open", "claimed", "resolved", "dismissed"] as const;
export const REPORT_CATEGORIES = [
  "harassment",
  "hate_speech",
  "cheating",
  "griefing",
  "spam",
  "inappropriate_name",
  "real_money_trading",
  "other",
] as const;
export const REPORT_PRIORITIES = ["low", "normal", "high"] as const;

export type ReportStatus = (typeof REPORT_STATUSES)[number];
export type ReportCategory = (typeof REPORT_CATEGORIES)[number];
export type ReportPriority = (typeof REPORT_PRIORITIES)[number];
