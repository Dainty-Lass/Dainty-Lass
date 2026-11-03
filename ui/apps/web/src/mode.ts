/** Which build this is (mode.mjs): inlined at build time, so a branch on it drops the other modes' code. */
export type DaintyMode = "dashboard" | "launcher" | "manager";

export const MODE = (process.env.NEXT_PUBLIC_DAINTY_MODE ?? "dashboard") as DaintyMode;
