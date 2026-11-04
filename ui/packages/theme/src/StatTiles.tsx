import { Badge } from "@astryxdesign/core/Badge";
import { Card } from "@astryxdesign/core/Card";
import { Grid } from "@astryxdesign/core/Grid";
import { Link } from "@astryxdesign/core/Link";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import type { ReactNode } from "react";
import { CARD_TITLE_CLASS, HUES, type Hue, hueAt } from "./accent";

export interface StatTile {
  id: string;
  /** What the number is. Sits above it, as the card's title. */
  label: string;
  value: ReactNode;
  /** One line under the value, for the detail that makes the number actionable. */
  note?: ReactNode;
  /** A short qualifier beside the value: a trend, or what is wrong with it. */
  flag?: { label: string; variant: "neutral" | "info" | "success" | "warning" | "error" };
  /** The card's colour; by position when unset. */
  hue?: Hue;
  href?: string;
}

/**
 * The row of numbers a page opens with (Caddy Proxy Manager's StatTiles). Shared so "is anything wrong
 * here" reads the same on every page; tiles cover the whole visible set, never just the page loaded.
 */
export function StatTiles({ tiles }: { tiles: StatTile[] }) {
  return (
    <Grid columns={{ minWidth: 200, max: 5 }} gap={3}>
      {tiles.map((tile, index) => {
        const hue = HUES[tile.hue ?? hueAt(index)];
        return (
          <Card key={tile.id} padding={4} className={hue.edge}>
            <Stack gap={1}>
              <Text type="body" className={CARD_TITLE_CLASS}>
                {tile.href ? <Link href={tile.href}>{tile.label}</Link> : tile.label}
              </Text>
              <Stack direction="horizontal" gap={2} align="center">
                <Text type="large" weight="semibold" hasTabularNumbers className={hue.text}>
                  {tile.value}
                </Text>
                {tile.flag && <Badge variant={tile.flag.variant} label={tile.flag.label} />}
              </Stack>
              {tile.note && (
                <Text type="supporting" color="secondary">
                  {tile.note}
                </Text>
              )}
            </Stack>
          </Card>
        );
      })}
    </Grid>
  );
}
