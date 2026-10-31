"use client";

import { useRouter } from "next/navigation";
import { useTranslations } from "next-intl";
import { useCallback, useTransition } from "react";
import { toast } from "sonner";
import type { ActionResult } from "@/lib/errors";

/**
 * Runs a server action, toasts its refusal in the catalog's words, and refreshes the page's data on
 * success. Returns whether it succeeded, so a dialog knows to close.
 */
export function useAction() {
  const t = useTranslations("errors");
  const router = useRouter();
  const [pending, startTransition] = useTransition();

  const run = useCallback(
    async <T>(action: () => Promise<ActionResult<T>>, success?: string): Promise<T | false> => {
      const result = await action();
      if (!result.ok) {
        toast.error(t(result.code, { detail: result.detail ?? "" }));
        return false;
      }
      if (success) toast.success(success);
      startTransition(() => router.refresh());
      return (result.value ?? true) as T;
    },
    [router, t],
  );

  return { run, pending };
}
