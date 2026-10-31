import { defineTheme } from "@astryxdesign/core/theme";
import { neutralTheme } from "@astryxdesign/theme-neutral";

/**
 * DaintyLass, from the launcher icon: periwinkle on deep navy, with a gold dot. One definition for the
 * dashboard, the launcher and the server manager; `bun run theme:build` compiles it to built/, which
 * the apps load as CSS so nothing is injected from a client effect (and Tauri's CSP keeps style-src 'self').
 */
export const daintyTheme = defineTheme({
  name: "dainty",
  extends: neutralTheme,
  // Light mode needs a darker periwinkle to keep 4.5:1 on white.
  color: { accent: ["#3563c9", "#a0c4ff"], neutralStyle: "cool" },
  // System fonts: the Tauri pages allow font-src 'self' only, and nothing here is worth a download.
  typography: {
    scale: { base: 14, ratio: 1.2 },
    body: { family: "system-ui", fallbacks: '-apple-system, "Segoe UI", Roboto, sans-serif' },
    heading: { family: "system-ui", fallbacks: '-apple-system, "Segoe UI", Roboto, sans-serif' },
  },
  radius: { base: 6, multiplier: 1 },
  tokens: {
    "--color-background-body": ["#eef1f6", "#10151c"],
    "--color-background-card": ["#ffffff", "#171f29"],
  },
});

export default daintyTheme;
