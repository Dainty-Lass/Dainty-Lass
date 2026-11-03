/**
 * A small reader for Prometheus' text exposition format (version 0.0.4), enough for the admin service's
 * `/metrics`: comments and `# HELP` / `# TYPE` lines are skipped, a sample is `name{labels} value
 * [timestamp]`, and a line that does not parse is dropped rather than failing the scrape.
 */

export interface PromSample {
  name: string;
  labels: Record<string, string>;
  value: number;
}

/** A scrape this large is not one the admin service sends; stop rather than hold it all. */
const MAX_SAMPLES = 50_000;

const NAME = /^[a-zA-Z_:][a-zA-Z0-9_:]*/;
const LABEL_NAME = /^[a-zA-Z_][a-zA-Z0-9_]*/;

function parseValue(token: string): number | null {
  switch (token) {
    case "+Inf":
    case "Inf":
      return Number.POSITIVE_INFINITY;
    case "-Inf":
      return Number.NEGATIVE_INFINITY;
    case "NaN":
      return Number.NaN;
  }
  if (!/^[-+]?(\d+\.?\d*|\.\d+)([eE][-+]?\d+)?$/.test(token)) return null;
  return Number(token);
}

/** The labels from just after `{`; returns them and the index after `}`, or null when malformed. */
function parseLabels(line: string, start: number): [Record<string, string>, number] | null {
  const labels: Record<string, string> = {};
  let i = start;
  for (;;) {
    while (line[i] === " " || line[i] === "\t") i++;
    if (line[i] === "}") return [labels, i + 1];
    const name = LABEL_NAME.exec(line.slice(i))?.[0];
    if (!name) return null;
    i += name.length;
    while (line[i] === " " || line[i] === "\t") i++;
    if (line[i] !== "=") return null;
    i++;
    while (line[i] === " " || line[i] === "\t") i++;
    if (line[i] !== '"') return null;
    i++;
    let value = "";
    for (;;) {
      const c = line[i];
      if (c === undefined) return null;
      if (c === '"') break;
      if (c === "\\") {
        const next = line[i + 1];
        value += next === "n" ? "\n" : next === undefined ? "" : next;
        i += 2;
        continue;
      }
      value += c;
      i++;
    }
    i++;
    labels[name] = value;
    while (line[i] === " " || line[i] === "\t") i++;
    if (line[i] === ",") i++;
    else if (line[i] !== "}") return null;
  }
}

export function parseSampleLine(raw: string): PromSample | null {
  const line = raw.trim();
  if (!line || line.startsWith("#")) return null;
  const name = NAME.exec(line)?.[0];
  if (!name) return null;
  let i = name.length;
  let labels: Record<string, string> = {};
  if (line[i] === "{") {
    const parsed = parseLabels(line, i + 1);
    if (!parsed) return null;
    [labels, i] = parsed;
  }
  const rest = line.slice(i).trim().split(/\s+/);
  if (rest.length < 1 || rest.length > 2 || !rest[0]) return null;
  const value = parseValue(rest[0]);
  if (value === null) return null;
  return { name, labels, value };
}

export function parsePrometheus(text: string): PromSample[] {
  const samples: PromSample[] = [];
  for (const line of text.split("\n")) {
    const sample = parseSampleLine(line);
    if (!sample) continue;
    samples.push(sample);
    if (samples.length >= MAX_SAMPLES) break;
  }
  return samples;
}
