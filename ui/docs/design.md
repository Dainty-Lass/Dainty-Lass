# Design principles

The dashboard, the launcher and the server manager share one look, taken from Caddy Proxy Manager
(its `astryx-variants.css`, `globals.css` and `components/ui`). It lives in `packages/theme`, so a
change there reaches all three apps, and none of them styles itself.

## Colour

- **Neutral, with one accent.** Astryx's neutral theme, untinted. The only colour that is a choice is the
  accent: pink by default, or purple, blue, cyan, teal, green, orange or red (Settings → General in the
  dashboard; a server's branding in the launcher). Yellow and grey are never accents: neither holds the
  contrast an accent's text and controls need. The accent is a runtime attribute (`data-dainty-accent`),
  not a theme build, so every app and every server share one stylesheet.
- **Black and near-white pages.** The body is `#f1f1f1` in light mode and pure `#000` in dark mode, at
  every width, so desktop and phone read as one theme. Cards lift off it in the card colour.
- **The rail is one step off the page:** `#e0e0e0` / `#111111` (`--dainty-rail`, which a palette tints), 17 levels either way, with its footer a
  further step darker so it reads as apart from the list scrolling above it.
- **The current page is tonal.** The selected nav item is the accent's background with the accent's ink,
  not the theme's hover tint, which read as "the pointer is here" rather than "you are here".
- **Ink that reads at label sizes.** Secondary text is `#474747` / `#bdbdbd`, darker than the theme's,
  which read as disabled; green and success are `#00490b` / `#b6e8b6`, since green on green was dim.
- **Phone dark mode lifts cards** to `#262626` on black and drops primary ink to `#ebebeb`, because white on
  black smears on OLED while scrolling.
- **The launcher icon's navy, periwinkle and gold are for the brand mark only**, never for chrome.

## Appearance

Each viewer can change their own look, beside the colour mode (`AppearanceControls`, under Account in the
dashboard and Settings in both shells; applied as picked, kept per device):

- **Palette:** Neutral (the look above), Slate, Midnight, Ember or Forest. A palette moves only the
  surfaces (page, card, popover, surface and rail), a tint either side of the neutral greys, so every ink
  keeps the contrast the theme was checked at. `data-dainty-palette` on `<html>`.
- **Accent:** the server's (the default), or any of the eight. `data-dainty-accent` is always the accent in
  effect.
- **Text size:** Default, or Large, the theme's touch scale a step up at every size. `data-dainty-text`.

The dashboard keeps the choice in the `dainty-appearance` cookie, so its server renders the first paint in
it; the shells keep it in local storage.

## The rail

- **At reading size:** rows at the large text size and 40 px tall, icons at 20 px, the current page in
  medium weight. 288 px wide by default and resizable (220 to 420, remembered).
- **Every icon in its hue**, as Caddy Proxy Manager's rail: passed as an element with a `HUES` class, so
  it keeps its colour on the tonal current row. A hue matches what the page is about where a stat tile
  already says it (players cyan, reports red, controllers teal, settings grey).

## Numbers

- **A page that has numbers opens with stat tiles** (`StatTiles`): the label above as the card's title,
  between primary and secondary ink; the value large, semibold and in the card's hue; one line under it
  that makes the number actionable; and a badge beside the value for what is wrong with it.
- **A tile's hue means something when it can:** green for players, purple for capacity, orange for work
  waiting (open reports), red for trouble (controllers down, flagged chat), grey when a problem count is
  zero. Tiles that mean nothing in particular take hues by position (purple, green, yellow, cyan).
- **Tiles cover the whole set in view**, never just the page of rows that is loaded.

## Tables

- **Dense data is rows, never cards.** A card is a standalone widget; a list of things is a `Table`.
- **Values at body size**, truncating rather than shrinking: tables asked for the small sizes all over,
  which read as fine print. **Headings are a step smaller** than the values they head, and wrap rather than
  lose their end to an ellipsis. Cells are padded wider than the theme's, so neighbouring values never
  read as one.
- **A two-line cell** (`dainty-cell-lines`) has its second line at the heading size.

## Status

- **Never colour alone.** A status is a `StatusDot` with its words beside it (`ControllerState`), a
  `Badge` for a count or a short state, a `Token` for a category. Problems sort first.

## Controls

- **Touch targets reach 44 px** on a coarse pointer (36 / 44 / 48 for the three sizes).
- **Button variants:** `primary` (the solid accent) for the one action a view is for; `tonal` (the accent
  at a lower weight) for a control that outranks `secondary` where the solid accent is already taken;
  `pink` for the one thing a page needs done next, whatever the accent; `destructive` for what cannot be
  taken back without Undo.

## Building pages

The Astryx rules from Caddy Proxy Manager's AGENTS.md apply unchanged: components do the layout (no
`<div>`), tokens for every value (no raw hex or pixels in components; the exceptions are the stylesheets
in `packages/theme/src`, where these rules are written down once), and look a component up with
`bunx astryx component <Name>` before guessing its props. A new variant needs a rule in `variants.css`
and its name in `packages/theme/src/astryx.d.ts`, or it type-checks and paints nothing.
