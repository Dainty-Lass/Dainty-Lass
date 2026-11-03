"use client";

import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { useCallback } from "react";

/**
 * Filters live in the URL, so a filtered view can be shared and the server reads it on render. Any
 * change other than the page itself starts again from the first page.
 */
export function useQueryPatch() {
  const router = useRouter();
  const pathname = usePathname();
  const params = useSearchParams();
  return useCallback(
    (patch: Record<string, string | number | boolean | null | undefined>) => {
      const next = new URLSearchParams(params.toString());
      for (const [key, value] of Object.entries(patch)) {
        if (value === null || value === undefined || value === "" || value === false)
          next.delete(key);
        else next.set(key, value === true ? "1" : String(value));
      }
      if (!("page" in patch)) next.delete("page");
      if (!("before" in patch)) next.delete("before");
      const qs = next.toString();
      router.push(qs ? `${pathname}?${qs}` : pathname);
    },
    [params, pathname, router],
  );
}
