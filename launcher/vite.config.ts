// Builds the stand-in page in page/ for the Tauri launcher (tauri.conf.json: frontendDist ../dist-page).
// The page's Content-Security-Policy comes from tauri.conf.json, so index.html carries none (DR's L10:
// a meta tag with default-src 'none' would block the isolation frame and the IPC endpoint).

import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

const page = fileURLToPath(new URL("./page/", import.meta.url));

export default defineConfig({
  root: page,
  base: "./",
  build: {
    outDir: fileURLToPath(new URL("./dist-page/", import.meta.url)),
    emptyOutDir: true,
    assetsInlineLimit: 0,
    target: "es2022",
  },
});
