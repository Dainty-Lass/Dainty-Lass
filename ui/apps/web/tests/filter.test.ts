import { describe, expect, test } from "bun:test";
import {
  compile,
  findMatches,
  maskText,
  segments,
  validatePattern,
  verdictOf,
} from "@/lib/chat/filter";

const term = (
  id: number,
  pattern: string,
  matchType: "word" | "substring" | "regex",
  action: "flag" | "mask" | "block",
  channels: string[] | null = null,
) => ({
  id,
  pattern,
  matchType,
  action,
  channels,
  enabled: true,
});

describe("chat filter", () => {
  test("a word term catches look-alikes and spacing but not longer words", () => {
    const terms = compile([term(1, "idiot", "word", "mask")]);
    expect(findMatches("you are such an id1ot lol", terms)).toHaveLength(1);
    expect(findMatches("i.d.i.o.t", terms)).toHaveLength(1);
    expect(findMatches("IDIOT!", terms)).toHaveLength(1);
    expect(findMatches("idiotic", terms)).toHaveLength(0);
  });

  test("a substring term matches inside words", () => {
    const terms = compile([term(1, "gold", "substring", "flag")]);
    expect(findMatches("goldsite dot com", terms)).toHaveLength(1);
  });

  test("the harshest action decides the verdict", () => {
    const terms = compile([term(1, "spam", "word", "flag"), term(2, "scam", "word", "block")]);
    expect(verdictOf(findMatches("spam and scam", terms))).toBe("blocked");
    expect(verdictOf(findMatches("hello", terms))).toBe("allowed");
  });

  test("masking replaces matched characters but keeps flags visible", () => {
    const terms = compile([term(1, "darn", "word", "mask"), term(2, "heck", "word", "flag")]);
    const text = "darn it, heck";
    expect(maskText(text, findMatches(text, terms))).toBe("**** it, heck");
  });

  test("channel-scoped terms skip other channels", () => {
    const terms = compile([term(1, "lfg", "word", "block", ["city"])]);
    expect(findMatches("lfg", terms, "city")).toHaveLength(1);
    expect(findMatches("lfg", terms, "party")).toHaveLength(0);
  });

  test("segments mark exactly the matched runs", () => {
    const terms = compile([term(1, "bad", "word", "mask")]);
    expect(segments("a bad day", findMatches("a bad day", terms))).toEqual([
      { text: "a ", hit: false },
      { text: "bad", hit: true },
      { text: " day", hit: false },
    ]);
  });

  test("invalid regexes are refused, and skipped if one is stored anyway", () => {
    expect(validatePattern("(unclosed", "regex")).toBe(false);
    expect(validatePattern("", "word")).toBe(false);
    expect(compile([term(1, "(unclosed", "regex", "block")])).toHaveLength(0);
  });
});
