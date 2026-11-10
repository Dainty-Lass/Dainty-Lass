import { expect, test } from "bun:test";
import { movedLoopbackUrl, settledUrl } from "../src";

test("old loopback service ports move, everything else stays", () => {
  expect(movedLoopbackUrl("http://127.0.0.1:61003/graphql")).toEqual({
    url: "http://127.0.0.1:41003/graphql",
    moved: true,
  });
  expect(movedLoopbackUrl("http://localhost:61000").url).toBe("http://localhost:41000");
  expect(movedLoopbackUrl("http://[::1]:61000/admin").url).toBe("http://[::1]:41000/admin");
  for (const kept of [
    "http://127.0.0.1:41003/graphql",
    "http://100.64.0.1:61000",
    "https://game.example.org:61003/graphql",
    "http://127.0.0.1:61009",
    "not a url",
  ]) {
    expect(movedLoopbackUrl(kept)).toEqual({ url: kept, moved: false });
  }
});

test("a moved setting warns once with its name", () => {
  const lines: string[] = [];
  expect(settledUrl("BACKEND_URL", "http://127.0.0.1:61000", (l) => lines.push(l))).toBe(
    "http://127.0.0.1:41000",
  );
  expect(settledUrl("BACKEND_URL", "http://127.0.0.1:41000", (l) => lines.push(l))).toBe(
    "http://127.0.0.1:41000",
  );
  expect(lines).toHaveLength(1);
  expect(lines[0]).toContain("BACKEND_URL");
});
