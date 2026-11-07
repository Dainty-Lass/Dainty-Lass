<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Bridge interface fixtures (RL8)

The three files the launcher's bridge generator writes, as they are generated today, **normalised** so they hold
only names, shapes and limits (decision L4, amended 2026-11-06 in `docs/port/plan-rd27-launcher.md`):

| Fixture | Generated file |
| --- | --- |
| `types.ts` | `ui/packages/bridge/src/generated/types.ts` |
| `commands.js` | `launcher/isolation/commands.js` |
| `app-commands.toml` | `permissions/app-commands.toml` |

RL8's generator test passes when its output, normalised the same way, equals these bytes. The generator writes
its own banner, doc comments and permission-set description; only the normalised form must match.

Normalisation, in order:

1. Line breaks are `\n`.
2. Leading lines that start with `//` or `#` (the banner) are dropped.
3. `types.ts` only: a doc comment (`/**` to the next `*/` and the line break after it) is removed together
   with the line break just before it when that line break follows other text on its line, since ts-rs adds
   that break only before a documented field or variant. A doc comment that starts a line (a documented type)
   is removed with the line break after it.
4. `app-commands.toml` only: the `[[set]]`'s `description` line (`description = "..."`) is dropped.

With this rule, the same types with 64 and with 233 doc comments normalise to identical bytes.
