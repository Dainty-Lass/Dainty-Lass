import { defineTheme } from "@astryxdesign/core/theme";
import { neutralTheme } from "@astryxdesign/theme-neutral";

/**
 * Caddy Proxy Manager's palette: Astryx's neutral theme, untinted, with the accent chosen at runtime
 * (variants.css) rather than baked in, so every app and every server's branding share one build.
 * `bun run theme:build` compiles it to built/, which the apps load as CSS so nothing is injected from a
 * client effect (and Tauri's CSP keeps style-src 'self').
 */
export const daintyTheme = defineTheme({
  name: "dainty",
  extends: neutralTheme,
  // System fonts: the Tauri pages allow font-src 'self' only, and nothing here is worth a download.
  typography: {
    scale: { base: 14, ratio: 1.2 },
    body: { family: "system-ui", fallbacks: '-apple-system, "Segoe UI", Roboto, sans-serif' },
    heading: { family: "system-ui", fallbacks: '-apple-system, "Segoe UI", Roboto, sans-serif' },
  },
});

export default daintyTheme;
