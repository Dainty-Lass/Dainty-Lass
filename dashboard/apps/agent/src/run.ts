import {
  type AgentCommand,
  type GameConfig,
  type GameResult,
  isAdminOp,
  runAdminOp,
  runBackend,
} from "@dainty/fleet";

/**
 * Runs one command from the dashboard, but only one the protocol allows: a named admin operation or a
 * backend route on the allowlist (@dainty/fleet ops.ts). Anything else is refused here, on the host.
 */
export async function runCommand(
  cfg: GameConfig,
  raw: unknown,
): Promise<{ id: string; result: GameResult } | null> {
  if (!raw || typeof raw !== "object") return null;
  const c = raw as Partial<AgentCommand> & { id?: unknown };
  if (typeof c.id !== "string" || c.id.length > 64) return null;
  if (c.kind === "admin") {
    if (!isAdminOp(c.op))
      return { id: c.id, result: { ok: false, code: "forbidden_call", message: String(c.op) } };
    const variables = c.variables && typeof c.variables === "object" ? c.variables : {};
    return {
      id: c.id,
      result: await runAdminOp(cfg, c.op, variables, String(c.operatorId ?? "0")),
    };
  }
  if (c.kind === "backend") {
    const operator =
      c.operator && typeof c.operator === "object"
        ? { id: String(c.operator.id ?? "0"), name: String(c.operator.name ?? "dashboard") }
        : { id: "0", name: "dashboard" };
    return {
      id: c.id,
      result: await runBackend(
        cfg,
        String(c.method ?? ""),
        String(c.target ?? ""),
        operator,
        c.body,
      ),
    };
  }
  return { id: c.id, result: { ok: false, code: "forbidden_call", message: String(c.kind) } };
}
