import type { LogSettings } from "@dainty/fleet";
import { recordAudit } from "@/lib/audit/record";
import type { Staff } from "@/lib/db/schema";
import { DomainError } from "@/lib/errors";
import { getController } from "@/lib/fleet/controllers";
import { adminApi, type LogSettingsChange } from "@/lib/game/admin-api";
import { operatorId } from "@/lib/players";

/**
 * A controller's `[logs]` in host.toml (dainty-lass docs/logging.md): the console's format and color and the
 * Graylog (GELF) collector. The server writes the file; it takes the change at its next start.
 */

const FORMATS = ["text", "json"] as const;
const COLORS = ["auto", "always", "never"] as const;

/** The controller's settings, or null when it cannot answer (offline, or an admin API without host.toml). */
export async function readLogSettings(
  me: Staff,
  controllerId: string,
): Promise<LogSettings | null> {
  try {
    return await adminApi(controllerId).logSettings(operatorId(me));
  } catch {
    return null;
  }
}

/** Checked here for the message; the server checks again, the GELF URL's transport rules included. */
function checked(change: LogSettingsChange): LogSettingsChange {
  const out: LogSettingsChange = {};
  if (change.format !== undefined) {
    if (!FORMATS.includes(change.format)) throw new DomainError("invalid_input");
    out.format = change.format;
  }
  if (change.color !== undefined) {
    if (!COLORS.includes(change.color)) throw new DomainError("invalid_input");
    out.color = change.color;
  }
  if (change.gelfUrl !== undefined) {
    const url = change.gelfUrl.trim();
    if (url.length > 2048 || /\s/.test(url)) throw new DomainError("invalid_input");
    out.gelfUrl = url;
  }
  return out;
}

export async function updateLogSettings(
  me: Staff,
  controllerId: string,
  change: LogSettingsChange,
  revertOfId?: number,
): Promise<LogSettings> {
  const controller = getController(controllerId);
  if (!controller) throw new DomainError("not_found");
  const wanted = checked(change);
  const api = adminApi(controllerId);
  const before = await api.logSettings(operatorId(me));
  const after = await api.setLogSettings(wanted, operatorId(me));
  recordAudit(me, {
    action: "controller.logs",
    targetType: "controller",
    targetId: controllerId,
    targetLabel: controller.name,
    controllerId,
    before: { format: before.format, color: before.color, gelfUrl: before.gelfUrl },
    after: { format: after.format, color: after.color, gelfUrl: after.gelfUrl },
    revertible: true,
    revertOfId,
  });
  return after;
}
