import { createRequire } from "node:module";
import tailwindcss from "@tailwindcss/vite";
import vinext from "vinext";
import { defineConfig } from "vite";
import { daintyMode } from "./mode.mjs";

const { version } = createRequire(import.meta.url)("./package.json");
const mode = daintyMode();

export default defineConfig({
  plugins: [
    tailwindcss(),
    // Every mode builds to dist/, which a build empties first: one mode at a time
    vinext(),
  ],
  // Inlined, so the rest of package.json stays out of the client bundle.
  define: {
    "process.env.NEXT_PUBLIC_APP_VERSION": JSON.stringify(process.env.APP_VERSION || version),
    "process.env.NEXT_PUBLIC_DAINTY_MODE": JSON.stringify(mode),
  },
  build: {
    target: "es2022",
    rolldownOptions: {
      // An import that is always undefined is a dead branch the bundler found; fail, not warn.
      onwarn(warning, warn) {
        if (warning.code === "IMPORT_IS_UNDEFINED") throw new Error(warning.message);
        warn(warning);
      },
    },
  },
});
