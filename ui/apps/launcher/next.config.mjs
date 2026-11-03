/**
 * Static files for the Tauri shell: no server behind the page, so no server components that read a
 * request. trailingSlash writes each route as <route>/index.html, which Tauri's asset resolver finds.
 */
/** @type {import('next').NextConfig} */
const nextConfig = {
  output: "export",
  trailingSlash: true,
  images: { unoptimized: true },
  poweredByHeader: false,
};

export default nextConfig;
