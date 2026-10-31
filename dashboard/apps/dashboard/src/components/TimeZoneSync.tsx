"use client";

import { useRouter } from "next/navigation";
import { useEffect } from "react";

const COOKIE = "dainty-tz";

/** Tells the server the reader's zone once, so timestamps render as they read them; refreshes if it was wrong. */
export function TimeZoneSync({ timeZone }: { timeZone: string }) {
  const router = useRouter();
  useEffect(() => {
    const actual = Intl.DateTimeFormat().resolvedOptions().timeZone;
    if (!actual || actual === timeZone) return;
    /* biome-ignore lint/suspicious/noDocumentCookie: Cookie Store is Chromium-only and async. */
    document.cookie = `${COOKIE}=${encodeURIComponent(actual)}; path=/; max-age=31536000; SameSite=Lax`;
    router.refresh();
  }, [timeZone, router]);
  return null;
}
