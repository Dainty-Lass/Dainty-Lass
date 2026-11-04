import { daintyMode, pageExtensions } from "./mode.mjs";

const mode = daintyMode();

/** @type {import('next').NextConfig} */
const dashboard = {
  // A Bun built-in, left to the runtime; there is no Node.js fallback to alias it to.
  serverExternalPackages: ["bun:sqlite"],
  output: "standalone",
  // Metadata in <head> for every request, not streamed into a hidden <div>.
  htmlLimitedBots: /.*/,
  // Security headers are per-request in src/proxy.dashboard.ts, for the CSP nonce.
};

/**
 * Static files for the Tauri shells: no server behind the page, so no server components that read a
 * request. trailingSlash writes each route as <route>/index.html, which Tauri's asset resolver finds.
 */
/** @type {import('next').NextConfig} */
const desktop = {
  output: "export",
  trailingSlash: true,
  images: { unoptimized: true },
};

/** @type {import('next').NextConfig} */
const nextConfig = {
  ...(mode === "dashboard" ? dashboard : desktop),
  pageExtensions: pageExtensions(mode),
  poweredByHeader: false,
};

export default nextConfig;
