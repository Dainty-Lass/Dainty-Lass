import { Card } from "@astryxdesign/core/Card";
import { Link } from "@astryxdesign/core/Link";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import type { ReactNode } from "react";

/** One number on the overview strip: a label, the value, and what it is out of or compared with. */
export function StatTile({
  label,
  value,
  hint,
  href,
  status,
}: {
  label: string;
  value: ReactNode;
  hint?: ReactNode;
  href?: string;
  status?: ReactNode;
}) {
  return (
    <Card padding={4}>
      <Stack gap={2}>
        <Stack direction="horizontal" justify="between" align="center">
          <Text type="label" color="secondary">
            {href ? <Link href={href}>{label}</Link> : label}
          </Text>
          {status}
        </Stack>
        <span className="dainty-stat">{value}</span>
        {hint && <Text type="supporting">{hint}</Text>}
      </Stack>
    </Card>
  );
}
