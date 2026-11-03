export type DaintyMode = "dashboard" | "launcher" | "manager";
export const MODES: DaintyMode[];
export function daintyMode(): DaintyMode;
export function pageExtensions(mode: DaintyMode): string[];
