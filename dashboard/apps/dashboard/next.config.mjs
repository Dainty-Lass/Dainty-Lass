/** @type {import('next').NextConfig} */
const nextConfig = {
  // A Bun built-in, left to the runtime; there is no Node.js fallback to alias it to.
  serverExternalPackages: ["bun:sqlite"],
  output: "standalone",
  poweredByHeader: false,
  // Metadata in <head> for every request, not streamed into a hidden <div>.
  htmlLimitedBots: /.*/,
  // Security headers are per-request in src/proxy.ts, for the CSP nonce.
};

export default nextConfig;
