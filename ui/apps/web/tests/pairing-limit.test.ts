import { beforeEach, describe, expect, test } from "bun:test";
import { clientIp } from "@/lib/auth/session";
import { pairingFailed, pairingLocked, resetPairingFailures } from "@/lib/fleet/agent-http";

beforeEach(() => resetPairingFailures());

describe("clientIp", () => {
  test("takes the hop the proxy appended, not the one the client sent", () => {
    const h = new Headers({ "x-forwarded-for": "6.6.6.6, 203.0.113.7" });
    expect(clientIp(h)).toBe("203.0.113.7");
    expect(clientIp(new Headers({ "x-real-ip": "198.51.100.2" }))).toBe("198.51.100.2");
    expect(clientIp(new Headers())).toBeNull();
  });
});

describe("pairing limit", () => {
  test("locks one address after ten wrong codes, for the window", () => {
    const now = 1_000_000;
    for (let i = 0; i < 10; i++) pairingFailed("a", now);
    expect(pairingLocked("a", now)).toBe(true);
    expect(pairingLocked("b", now)).toBe(false);
    expect(pairingLocked("a", now + 15 * 60_000 + 1)).toBe(false);
  });

  test("failures from other addresses never lock a fresh one", () => {
    const now = 1_000_000;
    for (let i = 0; i < 1000; i++) pairingFailed(`10.0.${i >> 8}.${i & 255}`, now);
    expect(pairingLocked("fresh", now)).toBe(false);
  });

  test("an expired window no longer locks once new addresses arrive", () => {
    for (let i = 0; i < 50; i++) pairingFailed(`old-${i}`, 0);
    const later = 15 * 60_000 + 1;
    pairingFailed("new", later);
    expect(pairingLocked("old-0", later)).toBe(false);
  });
});
