import crypto from "node:crypto";
import type { NextRequest } from "next/server";
import { NextResponse } from "next/server";
import { config as appConfig } from "@/lib/config";

/**
 * Security headers on every response and a nonce CSP on pages. Who may see a page is decided in the
 * pages themselves (requirePage), where the role is known; this only keeps the browser in line.
 */

const dev = process.env.NODE_ENV === "development";

function csp(nonce: string): string {
  return [
    "default-src 'self'",
    `script-src 'self' 'nonce-${nonce}'${dev ? " 'unsafe-eval'" : ""}`,
    // Astryx sets sizes as inline styles.
    "style-src 'self' 'unsafe-inline'",
    // Staff avatars come from Discord's CDN.
    "img-src 'self' data: https://cdn.discordapp.com",
    "font-src 'self'",
    `connect-src 'self'${dev ? " ws:" : ""}`,
    "frame-ancestors 'none'",
    "base-uri 'none'",
    "object-src 'none'",
    // Sign-in leaves for Discord as a redirect from our own route, which form-action does not cover.
    "form-action 'self'",
  ].join("; ");
}

function secure(response: NextResponse, policy: string): NextResponse {
  response.headers.set("Content-Security-Policy", policy);
  response.headers.set("X-Content-Type-Options", "nosniff");
  response.headers.set("X-Frame-Options", "DENY");
  response.headers.set("Referrer-Policy", "same-origin");
  response.headers.set("Permissions-Policy", "camera=(), microphone=(), geolocation=()");
  if (appConfig.baseUrl.startsWith("https:"))
    response.headers.set("Strict-Transport-Security", "max-age=31536000");
  return response;
}

export default function proxy(req: NextRequest) {
  const { pathname } = req.nextUrl;

  if (pathname.startsWith("/api/")) {
    // The sign-in and sign-out posts are plain forms, not server actions, so they get no origin check
    // from the framework: refuse one sent from another site.
    if (req.method === "POST" && pathname.startsWith("/api/auth/")) {
      const origin = req.headers.get("origin");
      if (origin && origin !== new URL(appConfig.baseUrl).origin && origin !== req.nextUrl.origin) {
        return secure(new NextResponse(null, { status: 403 }), "frame-ancestors 'none'");
      }
    }
    return secure(NextResponse.next(), "frame-ancestors 'none'");
  }

  // vinext reads the nonce from the request's copy of the policy.
  const nonce = crypto.randomBytes(16).toString("base64");
  const policy = csp(nonce);
  const headers = new Headers(req.headers);
  headers.set("Content-Security-Policy", policy);
  return secure(NextResponse.next({ request: { headers } }), policy);
}

// Built assets carry no page to protect, and skipping them saves a nonce per file.
export const config = { matcher: ["/((?!_next/static|favicon.ico).*)"] };
