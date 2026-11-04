/**
 * A hue for a card: a coloured left edge, and its number in the same hue (Caddy Proxy Manager's
 * components/ui/accent.ts). Whole class names, never built from the hue, since Tailwind finds utilities
 * by scanning the source; each is backed by Astryx's per-hue tokens, so both modes are covered.
 */
export type Hue =
  | "blue"
  | "cyan"
  | "gray"
  | "green"
  | "orange"
  | "pink"
  | "purple"
  | "red"
  | "teal"
  | "yellow";

export const HUES: Record<Hue, { edge: string; text: string }> = {
  blue: { edge: "border-l-2 border-l-blue-ring", text: "text-blue-number" },
  cyan: { edge: "border-l-2 border-l-cyan-ring", text: "text-cyan-number" },
  gray: { edge: "border-l-2 border-l-gray-ring", text: "text-gray-number" },
  green: { edge: "border-l-2 border-l-green-ring", text: "text-green-number" },
  orange: { edge: "border-l-2 border-l-orange-ring", text: "text-orange-number" },
  pink: { edge: "border-l-2 border-l-pink-ring", text: "text-pink-number" },
  purple: { edge: "border-l-2 border-l-purple-ring", text: "text-purple-number" },
  red: { edge: "border-l-2 border-l-red-ring", text: "text-red-number" },
  teal: { edge: "border-l-2 border-l-teal-ring", text: "text-teal-number" },
  yellow: { edge: "border-l-2 border-l-yellow-ring", text: "text-yellow-number" },
};

/** By position, for a row of tiles that carry no meaning of their own to colour by. */
export const POSITION_HUES: readonly Hue[] = ["purple", "green", "yellow", "cyan"];

export function hueAt(index: number): Hue {
  return POSITION_HUES[index % POSITION_HUES.length]!;
}

/**
 * Every shell's rail: wider than Astryx's 260px for the larger rows (variants.css), and resizable, the
 * width kept per app in local storage.
 */
export const RAIL_RESIZE = {
  defaultWidth: 288,
  minWidth: 220,
  maxWidth: 420,
  autoSaveId: "dainty-rail",
};

/** A stat card's title, between primary and secondary ink (`.dainty-card-title` in brand.css). */
export const CARD_TITLE_CLASS = "dainty-card-title";
