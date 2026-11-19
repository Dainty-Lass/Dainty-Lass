export interface CrashFile {
  name: string;
  size: number;
  sha256: string;
  /** The file's text when the content service sent it; binaries and oversize files stay on the server. */
  text: string | null;
}

export interface NewCrashReport {
  id: string;
  receivedAt: number;
  account: { userId: string; name: string };
  crashId: string;
  gameVersion: string | null;
  launcherVersion: string | null;
  exitCode: string | null;
  serverPath: string;
  files: CrashFile[];
}

export const MAX_CRASH_FILES = 16;
export const MAX_CRASH_TEXT = 262_144;

const ID = /^[0-9a-f]{32}$/;
const SHA256 = /^[0-9a-f]{64}$/;

const str = (v: unknown, max: number) =>
  typeof v === "string" && v.length > 0 && v.length <= max ? v : undefined;
// Absent and null both mean unknown; a wrong type or an oversize value refuses the report.
const optional = (v: unknown, max: number): string | null | undefined =>
  v === undefined || v === null ? null : str(v, max);

function parseFile(raw: unknown): CrashFile | null {
  if (!raw || typeof raw !== "object") return null;
  const f = raw as Record<string, unknown>;
  const name = str(f.name, 80);
  const size = f.size;
  const sha256 = typeof f.sha256 === "string" && SHA256.test(f.sha256) ? f.sha256 : undefined;
  const text =
    f.text === null || f.text === undefined
      ? null
      : typeof f.text === "string" && f.text.length <= MAX_CRASH_TEXT
        ? f.text
        : undefined;
  if (!name || typeof size !== "number" || !Number.isSafeInteger(size) || size < 0) return null;
  if (!sha256 || text === undefined) return null;
  return { name, size, sha256, text };
}

/** Parses one crash report from the content service; anything malformed refuses the whole report. */
export function parseCrashReport(raw: unknown): NewCrashReport | null {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return null;
  const r = raw as Record<string, unknown>;
  const account = r.account as Record<string, unknown> | undefined | null;
  const id = typeof r.id === "string" && ID.test(r.id) ? r.id : undefined;
  const receivedAt =
    typeof r.receivedAt === "number" && Number.isSafeInteger(r.receivedAt) && r.receivedAt > 0
      ? r.receivedAt
      : undefined;
  const userId = account && typeof account === "object" ? str(account.userId, 64) : undefined;
  const name = account && typeof account === "object" ? str(account.name, 64) : undefined;
  const crashId = str(r.crashId, 128);
  const gameVersion = optional(r.gameVersion, 64);
  const launcherVersion = optional(r.launcherVersion, 64);
  const exitCode = optional(r.exitCode, 32);
  const serverPath = str(r.serverPath, 512);
  if (!id || !receivedAt || !userId || !name || !crashId || !serverPath) return null;
  if (gameVersion === undefined || launcherVersion === undefined || exitCode === undefined)
    return null;
  if (!Array.isArray(r.files) || r.files.length > MAX_CRASH_FILES) return null;
  const files: CrashFile[] = [];
  for (const f of r.files) {
    const file = parseFile(f);
    if (!file) return null;
    files.push(file);
  }
  return {
    id,
    receivedAt,
    account: { userId, name },
    crashId,
    gameVersion,
    launcherVersion,
    exitCode,
    serverPath,
    files,
  };
}
