"use server";

import { cookies } from "next/headers";
import { requireAction } from "@/lib/auth/guard";
import { attempt, DomainError } from "@/lib/errors";
import {
  createController,
  formatCode,
  getController,
  removeController,
  repairController,
  updateController,
} from "@/lib/fleet/controllers";
import { updateLogSettings } from "@/lib/fleet/log-settings";
import { ALL, SCOPE_COOKIE } from "@/lib/fleet/scope";
import type { LogSettingsChange } from "@/lib/game/admin-api";

export async function createControllerAction(name: string, region: string) {
  return attempt(async () => {
    const me = await requireAction("controllers:write");
    const { controller, code } = createController(me, { name, region });
    return { id: controller.id, code: formatCode(code), expiresAt: controller.pairingExpiresAt };
  });
}

export async function updateControllerAction(
  id: string,
  change: { name?: string; region?: string | null; note?: string | null; disabled?: boolean },
) {
  return attempt(async () => {
    updateController(await requireAction("controllers:write"), id, change);
  });
}

export async function repairControllerAction(id: string) {
  return attempt(async () =>
    formatCode(repairController(await requireAction("controllers:write"), id)),
  );
}

export async function removeControllerAction(id: string) {
  return attempt(async () => {
    removeController(await requireAction("controllers:write"), id);
  });
}

/** The controller's host.toml `[logs]`; the server takes it at its next start. */
export async function updateLogSettingsAction(id: string, change: LogSettingsChange) {
  return attempt(async () => {
    await updateLogSettings(await requireAction("controllers:write"), id, change);
  });
}

/** The rail's controller picker: a cookie, so every page renders in the chosen scope. */
export async function setScopeAction(scope: string) {
  return attempt(async () => {
    await requireAction("controllers:read");
    if (scope !== ALL && !getController(scope)) throw new DomainError("not_found");
    (await cookies()).set({
      name: SCOPE_COOKIE,
      value: scope,
      path: "/",
      sameSite: "lax",
      httpOnly: true,
      maxAge: 31_536_000,
    });
  });
}
