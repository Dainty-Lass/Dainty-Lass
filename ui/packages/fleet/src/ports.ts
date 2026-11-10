/**
 * The loopback ports dainty-lass's services listened on before they moved below Windows' dynamic range
 * (49152-65535, lent to Hyper-V, WSL and Docker), and where they went. A private gateway never listens on
 * loopback, so 127.0.0.1:61000 is always the old backend.
 */
export const MOVED_PORTS: Readonly<Record<string, string>> = {
  "61000": "41000",
  "61001": "41001",
  "61002": "41002",
  "61003": "41003",
  "61005": "41005",
  "61006": "41006",
};

const LOOPBACK = ["127.0.0.1", "localhost", "[::1]"];

/** `url` with an old loopback service port moved to its new one; anything else is returned as it is. */
export function movedLoopbackUrl(url: string): { url: string; moved: boolean } {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return { url, moved: false };
  }
  const to = MOVED_PORTS[parsed.port];
  if (!to || !LOOPBACK.includes(parsed.hostname)) return { url, moved: false };
  const authority = `${parsed.hostname}:${parsed.port}`;
  return { url: url.replace(authority, `${parsed.hostname}:${to}`), moved: true };
}

/** [`movedLoopbackUrl`] for a setting, warning through `warn` when it moved the port. */
export function settledUrl(
  name: string,
  url: string,
  warn: (line: string) => void = console.warn,
): string {
  const result = movedLoopbackUrl(url);
  if (result.moved)
    warn(
      `${name}: ${url} is an old port of dainty-lass's; using ${result.url}. Update the setting.`,
    );
  return result.url;
}
