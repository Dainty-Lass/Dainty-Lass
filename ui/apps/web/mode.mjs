// The one switch between the three builds of this app: DAINTY_MODE picks the routes (by page extension),
// the output (a static export for the Tauri shells, a Bun server for the dashboard) and the catalogs.
export const MODES = ["dashboard", "launcher", "manager"];

export function daintyMode() {
  const mode = process.env.DAINTY_MODE?.trim() || "dashboard";
  if (!MODES.includes(mode))
    throw new Error(`DAINTY_MODE must be one of ${MODES.join(", ")}, not "${mode}"`);
  return mode;
}

/** A file is a route in this mode when its name ends in .<mode>.tsx, or .desktop.tsx for both shells. */
export function pageExtensions(mode) {
  const tags = mode === "dashboard" ? ["dashboard"] : [mode, "desktop"];
  return tags.flatMap((tag) => [`${tag}.tsx`, `${tag}.ts`]);
}
