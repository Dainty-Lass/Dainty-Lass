import { expect, test } from "bun:test";
import launcherEn from "../messages/launcher/en.json";
import launcherPt from "../messages/launcher/pt-BR.json";
import managerEn from "../messages/manager/en.json";
import managerPt from "../messages/manager/pt-BR.json";

type Tree = { [key: string]: string | Tree };

function flatten(tree: Tree, prefix = ""): Record<string, string> {
  return Object.fromEntries(
    Object.entries(tree).flatMap(([k, v]) =>
      typeof v === "string" ? [[`${prefix}${k}`, v]] : Object.entries(flatten(v, `${prefix}${k}.`)),
    ),
  );
}

// An argument is a name followed by "}" or ",": plural branches' words are text, not arguments.
const args = (text: string) =>
  [...new Set([...text.matchAll(/\{(\w+)(?=[},])/g)].map((m) => m[1]))].sort();

// Both languages in the same commit, with the same keys and the same placeholders (dainty-lass AGENTS.md).
for (const [shell, en, pt] of [
  ["launcher", launcherEn, launcherPt],
  ["manager", managerEn, managerPt],
] as const) {
  test(`${shell}: pt-BR has every English key and placeholder`, () => {
    const a = flatten(en as Tree);
    const b = flatten(pt as Tree);
    expect(Object.keys(b).sort()).toEqual(Object.keys(a).sort());
    for (const key of Object.keys(a))
      expect([key, args(b[key] ?? "")]).toEqual([key, args(a[key]!)]);
  });
}
